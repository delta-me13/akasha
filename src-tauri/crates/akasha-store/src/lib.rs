//! akasha-store —— **加密库的打开路径**：把口令经 C API 送进 SQLCipher，然后把库打开。
//!
//! 这个 crate 只做一件事，但这件事必须做对：**顺序**。`Connection::open` 之后、
//! **第一条会读库的语句**之前就得把密钥送进去 —— SQLCipher 是"首次用到密钥时才派生"，
//! 任何真正读页的操作都算"用到"。违反它的表现是**库打不开**，而错误信息不会指向
//! "你少调了一次 key"（ADR-0002 D4）。
//!
//! ⚠️ **一个例外，而且是刻意的**：[`open`] / [`create`] 会在送密钥**之前**设
//! `cipher_memory_security`（见 [`enable_memory_security`]）。它不读库，所以不违反 D4
//! 的意图；而排在前面才有意义 —— 密钥材料的第一份副本是 `sqlite3_key` 自己分配的。
//!
//! 四条不可动的约束，各有出处：
//!
//! 1. **口令经 `sqlite3_key()`，不进 SQL 文本**（D4）—— 文本形式会把口令写进一条 SQL 语句，
//!    于是它有机会出现在错误消息与语句追踪里；且口令里有 `'` / `\` 或非 UTF-8 字节时，
//!    字符串形式要么报错要么被改写。
//! 2. **空口令不可表示**（D5）—— 不是"打开函数里有个 if"，而是 [`Passphrase`] 造不出空值。
//!    空 key 送进 `sqlite3_key()` 的后果是**得到一个明文库**（D5 / §7 实测）。
//! 3. **不设任何 `cipher_*` / `kdf_iter` 参数**（D2）—— 全取 SQLCipher 4 的默认值。
//!    非默认值必须每次打开都重新声明，那就得有个地方存它，等于把已否掉的"旁挂文件"
//!    换个名字请回来。唯一的例外是 `cipher_memory_security`：它不写进文件，见 §6。
//! 4. **`user_version` 是唯一的格式权威**（D7）—— [`create`] 写它，[`open`] 校验它。
//!
//! **打开与新建是两条路**，不是一个函数的两个分支：实测（plan 0402 §实施记录）
//! 一个不存在的文件（或 0 字节的文件）**用任何口令都能"打开"** —— 库里没有任何东西可解，
//! KDF 根本没跑。于是"打开不存在的库"不会失败，反而会把这把口令当成创建口令。
//! 拆开之后：`open` 只开已有的库（没有就 `NoVault`），`create` 从不覆盖已有内容。
//!
//! 表结构与 CRUD 不在这里（plan 0403）：[`create`] 只写版本号，**一列都不建**。
//!
//! **零 Tauri 依赖**（`AGENTS.md` §3.1），由
//! `.ast-grep/rules/no-tauri-in-core-crates.yml` 强制。

mod passphrase;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, ffi};

pub use passphrase::Passphrase;

/// 库文件名（ADR-0002 D1）：四套池与 Bitwarden 缓存**同一个**文件。
///
/// 放在这里而不是调用方：它是**磁盘上的格式**的一部分，改它等于迁移用户数据。
pub const STORE_FILE_NAME: &str = "akasha.db";

/// 格式版本（ADR-0002 D7）：`PRAGMA user_version` 的当前取值。
///
/// v1 就是 `1`，没有别的语义。`> 1` = 更新版本的程序写的，明确拒绝；`< 1` 见 [`open`]。
pub const FORMAT_VERSION: i64 = 1;

/// 打开之后、密钥送入之后的第一条**读库**语句。
///
/// 它有两个作用：① 逼 SQLCipher 真正派生一次密钥（口令错在这里暴露，而不是在
/// 几十行之后的某次写入）；② 确认这确实是个可读的库。
///
/// ⚠️ 它**只有在库里有内容时才验证得了口令**（实测）：空文件上它照样成功。
/// 所以真正的保证来自"`open` 拒绝 0 字节文件" + "`create` 把口令钉进文件"这两条，
/// 不是来自这条查询本身。
const PROBE_SQL: &str = "SELECT count(*) FROM sqlite_master";

/// 打开 / 创建加密库时可能出的错。
///
/// `NotADatabase` 的名字是**照实**起的：SQLCipher 对"口令错"和"文件损坏"给的是同一个
/// `SQLITE_NOTADB`，我们也不假装能区分这两者。想给人看的文案由调用方决定。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// 空口令（ADR-0002 D5）。由 [`Passphrase::new`] 返回 —— **不是**"口令太弱"，
    /// 弱口令由用户自己承担，空口令则是"加密悄悄没了"。
    #[error("empty passphrase refused: an empty SQLCipher key disables encryption")]
    EmptyPassphrase,
    /// 口令比 `i32::MAX` 还长（`sqlite3_key` 的长度参数是 `c_int`）。现实中到不了，
    /// 但它让"长度转换失败"不必退化成 `unwrap`（`AGENTS.md` §0 禁止）。
    #[error("passphrase longer than i32::MAX bytes")]
    PassphraseTooLong,
    /// `sqlite3_key` 自己返回了非 `SQLITE_OK`。正常情况下它总是返回 OK ——
    /// 口令对不对要到第一条语句才知道，所以这条分支是"意外状态"，不是正常失败路径。
    #[error("sqlite3_key returned {0}")]
    KeyRejected(i32),
    /// 要打开的库不存在（文件缺失或 0 字节）：**还没有建过**。
    ///
    /// 与"口令错"分开是因为用户的下一步动作不同：这里是"去创建"，那里是"重新输入"。
    #[error("no vault at {0}")]
    NoVault(PathBuf),
    /// 要创建的位置**已经有内容**。绝不覆盖一个库 —— 那可能是用户全部的数据。
    #[error("vault already exists at {0}")]
    VaultExists(PathBuf),
    /// `user_version` 不是 [`FORMAT_VERSION`]（ADR-0002 D7）。
    #[error("unsupported vault format version {found} (this build writes {FORMAT_VERSION})")]
    UnsupportedVersion { found: i64 },
    /// 打不开：口令错**或**文件不是个库。
    #[error("not a database: wrong passphrase or corrupt file")]
    NotADatabase,
    /// 其余 sqlite 错误原样上抛，不在这里解释。
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// 文件系统错误（路径探测）。刻意**不**把"文件不存在"归到这里，那是 [`StoreError::NoVault`]。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// 数据目录里的库文件路径（ADR-0002 D1：与 `config.json` **同一个**目录）。
///
/// 目录本身由调用方决定（便携目录还是 OS 数据目录，见 `docs/portable.md` §4）——
/// 本 crate 不猜、也**不创建目录**（`portable.md` §3.1）。
pub fn vault_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STORE_FILE_NAME)
}

/// 打开一个**已经存在**的加密库，返回解好锁的连接。
///
/// 不存在（或 0 字节）→ [`StoreError::NoVault`]：**不创建**。创建走 [`create`]。
/// 两条路分开的理由见 crate 文档第 4 条 —— 合在一起时，"打开"在新建路径上等于没验证口令。
///
/// 返回的连接已经解好锁：密钥送进去了、已经成功读过一次 `sqlite_master`、
/// `user_version` 也校验过了。调用方拿到它就等于拿到了一个能用的库，
/// 不需要（也不应该）自己再设密钥。
pub fn open(path: &Path, passphrase: &Passphrase) -> Result<Connection, StoreError> {
    if file_len(path)? == 0 {
        return Err(StoreError::NoVault(path.to_path_buf()));
    }

    let conn = Connection::open(path)?;
    unlock(&conn, passphrase)?;
    check_format_version(&conn)?;
    restrict_to_owner(path);
    Ok(conn)
}

/// 在一个**空位置**上建一个新库，并把 `passphrase` 钉进去。
///
/// 已经有内容 → [`StoreError::VaultExists`]（永不覆盖）。
/// 落地的动作只有 `PRAGMA user_version = FORMAT_VERSION` 一句 —— 它同时是
/// ①D7 的版本字段、②把文件从 0 字节变成 4096 字节的那一次写。**没有第二句**：
/// 表结构是 plan 0403 的事，本函数建出来的库除了版本号什么都没有。
///
/// 为什么"写一句版本号"就等于"钉住口令"：SQLCipher 的盐与密钥校验值都只在**第一次写页**
/// 时落盘（连带生成 16 字节随机盐）。在那之前文件是空的，任何口令都能打开它 ——
/// 这正是 §实施记录里那条实测。
pub fn create(path: &Path, passphrase: &Passphrase) -> Result<Connection, StoreError> {
    if file_len(path)? > 0 {
        return Err(StoreError::VaultExists(path.to_path_buf()));
    }

    let conn = Connection::open(path)?;
    unlock(&conn, passphrase)?;
    conn.pragma_update(None, "user_version", FORMAT_VERSION)?;
    restrict_to_owner(path);
    Ok(conn)
}

/// 送密钥前后的固定三步。两条路（打开 / 新建）用的是**同一段** ——
/// 复制成两份，迟早有一份会少一步（少的那一步的表现是"库打不开"）。
fn unlock(conn: &Connection, passphrase: &Passphrase) -> Result<(), StoreError> {
    enable_memory_security(conn)?;
    apply_key(conn, passphrase)?;
    probe_unlocked(conn)
}

/// 让 SQLCipher 擦除自己分配的内存（ADR-0002 §6 的"建议开"）。
///
/// **必须在送密钥之前**：`sqlite3_key` 会复制一份口令进 codec context，
/// 那一份才该落在安全分配器上。它不读库，所以与 D4 的"密钥先于一切"不冲突
/// （D4 的意图是"先于任何**读页**的操作"）。
///
/// 三个实测到的性质，都会让"想当然的断言"失败（ADR-0002 §7）：
///
/// - 它是**进程级**全局（`sqlcipher_mem_security_on` 是静态变量），**不是连接级**；
/// - **只能开不能关** —— 上游 `sqlcipher_set_mem_security` 的实现是 `if(on) { … }`，
///   所以设 `OFF` 不报错、也没效果；
/// - 读回来的值是 `on && executed` 的合取（`executed` 表示安全分配器被用过至少一次），
///   所以只能断言"我们打开过之后读回来是 1"，不能断言"默认是 0" —— 同进程里前一个连接
///   开过之后，全局就是 1 了。
///
/// 失败**不报错**的余地不存在：这句 pragma 在 SQLCipher 里不返回失败。真取不到时
/// 说明链接的不是 SQLCipher（那时 `cipher_version` 也取不到），按 sqlite 错误上抛。
fn enable_memory_security(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch("PRAGMA cipher_memory_security = ON")?;
    Ok(())
}

/// 把口令经 `sqlite3_key()` 送进连接。**必须是打开之后的第一件读库之外的事。**
///
/// 为什么不用 `PRAGMA key = '…'`：见 crate 文档第 1 条。磁盘上的字节完全一致
/// （`PRAGMA key` 内部就是调它），换掉的是**秘密经过的路径**。
///
/// 这里**没有**空口令分支：空值造不出来（[`Passphrase::new`]），所以这个函数收到的一定
/// 是非空字节。这就是"应用层拒绝"从 if 升级成类型之后的样子 —— 少一条永远不该走的分支。
#[allow(unsafe_code)] // 全仓库唯一的 unsafe 单点，见下面 SAFETY 与 ADR-0002 D4
fn apply_key(conn: &Connection, passphrase: &Passphrase) -> Result<(), StoreError> {
    let bytes = passphrase.expose();
    let len = i32::try_from(bytes.len()).map_err(|_| StoreError::PassphraseTooLong)?;

    // SAFETY: ① `conn.handle()` 返回本连接持有的 `*mut sqlite3`，在 `conn` 存活期间一直有效，
    // 而 `conn` 在这里活着（借用而非复制）；② `bytes` 的 ptr/len 在本次调用期间有效，
    // 且 `sqlite3_key` 只读它——它把密钥复制进自己的缓冲区，调用返回后不持有这个指针；
    // ③ 调用发生在 `Connection::open` 之后、任何**读库**的语句之前（本函数之上只有一句
    // 不读库的 pragma，之下才是 `PROBE_SQL`）。
    // 单测覆盖见 `tests/`：错误口令打不开、正确口令打得开、空口令在类型层就被拦。
    let rc = unsafe { ffi::sqlite3_key(conn.handle(), bytes.as_ptr().cast(), len) };

    if rc != ffi::SQLITE_OK {
        return Err(StoreError::KeyRejected(rc));
    }
    Ok(())
}

/// 送完密钥后读一次库，把"口令不对"这件事**逼到眼前**。
fn probe_unlocked(conn: &Connection) -> Result<(), StoreError> {
    match conn.query_row(PROBE_SQL, [], |row| row.get::<_, i64>(0)) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(err, _))
            if err.code == rusqlite::ErrorCode::NotADatabase =>
        {
            Err(StoreError::NotADatabase)
        }
        Err(err) => Err(StoreError::Sqlite(err)),
    }
}

/// 校验格式版本（ADR-0002 D7）。
///
/// `!= FORMAT_VERSION` 一律拒绝，包括 `< 1`：v1 之前**没有版本**，"0" 出现在一个非空文件里
/// 说明它不是本程序写的库（明文库走不到这里 —— 它在读第一页时就已经以 `NotADatabase` 失败了，
/// 实测）。D7 原写的 "`< 1` 走迁移"是给**将来的 v2 读 v1** 留的话，不是给 0 留的；
/// 把一个来路不明的文件当"待迁移"接下去，正是"能开但内容不对"的入口。已记入 §10。
fn check_format_version(conn: &Connection) -> Result<(), StoreError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found == FORMAT_VERSION {
        Ok(())
    } else {
        Err(StoreError::UnsupportedVersion { found })
    }
}

/// 文件长度；**不存在**返回 0（不是错误）。
///
/// "不存在"与"0 字节"归成一类是有意的：两者都表示**还没有密钥落在这条路径上**
/// （实测：0 字节的文件用什么口令都能打开），所以对 [`open`] 都是 `NoVault`、
/// 对 [`create`] 都是"可以建"。
///
/// ⚠️ 这是一次 TOCTOU 检查（先看长度、再让 sqlite 打开）。桌面应用是单进程单线程的
/// 使用方式，这里接受它；真要防的话得把"排他创建"下沉到 `open(2)` 的 flags 上 ——
/// 而那不是现在的问题（0403 接进 app 时若出现并发解锁，回来改这里）。
fn file_len(path: &Path) -> Result<u64, StoreError> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(meta.len()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(StoreError::Io(err)),
    }
}

/// 把库文件收紧到 0600（ADR-0002 D12）。
///
/// **尽力而为，失败不报错**：只读挂载、非 unix 语义的文件系统上会失败，
/// 而"权限位没设上"不该挡住用户解锁自己的数据 —— 谁来观察这件事是**启动检查**的事
/// （`portable.md` §4 第 3 条，还没实现）。
#[cfg(unix)]
fn restrict_to_owner(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::Permissions::from_mode(0o600);
    let _ = std::fs::set_permissions(path, mode);
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) {}
