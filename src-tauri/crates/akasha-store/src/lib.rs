//! akasha-store —— **加密库的打开路径**：把口令经 C API 送进 SQLCipher，然后把库打开。
//!
//! 这个 crate 只做一件事，但这件事必须做对：**顺序**。`Connection::open` 之后
//! **第一条**语句之前就得把密钥送进去（[`open`] 就是这么写的）——
//! SQLCipher 是"首次用到密钥时才派生"，任何 `PRAGMA` 与查询都算"用到"。
//! 违反它的表现是**库打不开**，而错误信息不会指向"你少调了一次 key"（ADR-0002 D4）。
//!
//! 三条不可动的约束，各有出处：
//!
//! 1. **口令经 `sqlite3_key()`，不进 SQL 文本**（ADR-0002 D4）—— 文本形式会把口令写进
//!    一条 SQL 语句，于是它有机会出现在错误消息与语句追踪里；且口令里有 `'` / `\`
//!    或非 UTF-8 字节时，字符串形式要么报错要么被改写。
//! 2. **空口令在应用层拒绝**（ADR-0002 D5）—— SQLCipher 里"空 key"的语义是
//!    **关闭加密**，一次校验疏漏的后果是"看起来有口令、其实是明文库"。
//!    [`open`] 在**碰文件之前**就返回。
//! 3. **不设置任何 `cipher_*` / `kdf_iter` 参数**（ADR-0002 D2）—— 全取 SQLCipher 4 的默认值。
//!    非默认值必须每次打开都重新声明，那就得有个地方存它，等于把已否掉的"旁挂文件"
//!    换个名字请回来。
//!
//! 表结构与 CRUD 不在这里（plan 0403），口令 → 库密钥的应用层封装也不在这里（plan 0402）。
//!
//! **零 Tauri 依赖**（`AGENTS.md` §3.1），由
//! `.ast-grep/rules/no-tauri-in-core-crates.yml` 强制。

use std::path::Path;

use rusqlite::{Connection, ffi};

/// 库文件名（ADR-0002 D1）：四套池与 Bitwarden 缓存**同一个**文件。
///
/// 放在这里而不是调用方：它是**磁盘上的格式**的一部分，改它等于迁移用户数据。
pub const STORE_FILE_NAME: &str = "akasha.db";

/// 打开之后、密钥送入之后的第一条语句。
///
/// 它有两个作用：① 逼 SQLCipher 真正派生一次密钥（口令错在这里暴露，而不是在
/// 几十行之后的某次写入）；② 确认这确实是个可读的库。
const PROBE_SQL: &str = "SELECT count(*) FROM sqlite_master";

/// 打开 / 创建加密库时可能出的错。
///
/// `NotADatabase` 的名字是**照实**起的：SQLCipher 对"口令错"和"文件损坏"给的是同一个
/// `SQLITE_NOTADB`，我们也不假装能区分这两者。想给人看的文案由调用方决定。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// 空口令（ADR-0002 D5）。**不是**"口令太弱"—— 弱口令由用户自己承担，
    /// 空口令则是"加密悄悄没了"。
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
    /// 打不开：口令错**或**文件不是个库。
    #[error("not a database: wrong passphrase or corrupt file")]
    NotADatabase,
    /// 其余 sqlite 错误原样上抛，不在这里解释。
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// 用 `passphrase` 打开（不存在则创建）`path` 上的加密库。
///
/// 返回的连接已经**解好锁**：密钥送进去了，并且已经成功读过一次 `sqlite_master`。
/// 调用方拿到它就等于拿到了一个能用的库，不需要（也不应该）自己再设密钥。
///
/// `passphrase` 是**字节**而不是 `String`（ADR-0002 D5）：口令可以是任意字节，
/// 而 `String` 会强制它必须是合法 UTF-8，还会让口令在堆上多留一份。
pub fn open(path: &Path, passphrase: &[u8]) -> Result<Connection, StoreError> {
    // ⚠️ 顺序：先校验，再碰文件。空口令若走到 sqlite3_key，得到的是**未加密**的库。
    if passphrase.is_empty() {
        return Err(StoreError::EmptyPassphrase);
    }

    let conn = Connection::open(path)?;
    apply_key(&conn, passphrase)?;
    probe_unlocked(&conn)?;
    restrict_to_owner(path);
    Ok(conn)
}

/// 把口令经 `sqlite3_key()` 送进连接。**必须是打开之后的第一件事。**
///
/// 为什么不用 `PRAGMA key = '…'`：见 crate 文档第 1 条。磁盘上的字节完全一致
/// （`PRAGMA key` 内部就是调它），换掉的是**秘密经过的路径**。
#[allow(unsafe_code)] // 全仓库唯一的 unsafe 单点，见下面 SAFETY 与 ADR-0002 D4
fn apply_key(conn: &Connection, passphrase: &[u8]) -> Result<(), StoreError> {
    let len = i32::try_from(passphrase.len()).map_err(|_| StoreError::PassphraseTooLong)?;

    // SAFETY: ① `conn.handle()` 返回本连接持有的 `*mut sqlite3`，在 `conn` 存活期间一直有效，
    // 而 `conn` 在这里活着（借用而非复制）；② `passphrase` 的 ptr/len 在本次调用期间有效，
    // 且 `sqlite3_key` 只读它——它把密钥复制进自己的缓冲区，调用返回后不持有这个指针；
    // ③ 调用发生在 `Connection::open` 之后、任何 `PRAGMA` / 查询之前（本函数是 `open` 的第二句）。
    // 单测覆盖见 `tests/sqlcipher_contract.rs`：错误口令打不开、正确口令打得开、空口令被拦。
    let rc = unsafe { ffi::sqlite3_key(conn.handle(), passphrase.as_ptr().cast(), len) };

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
