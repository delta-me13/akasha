//! **导出与还原**（plan 0404 / ADR-0002 D6、D7、D12）。
//!
//! 这是本 crate 里**最高危**的一段：库里含私钥，而导出会把它写到一个用户挑的位置。
//! 所以两条要求不是文档里的措辞，而是这条 API 的形状：
//!
//! 1. **明文导出走不通**，除非调用方拿出一个 [`PlaintextAck`] —— 那个值只能由
//!    `PlaintextAck::typed()` 用一句后果说明**逐字**换出来（D6 的"二次确认"）；
//!    而且导出件的文件名必须自曝含 `plain`（D6 的"文件名必须自曝"）。
//! 2. **导出口令必须独立**：与它导出时那把口令相同就是拒绝（D6 的"不复用库口令"）。
//!
//! ## 导出件就是库（D6 的容器选择）
//!
//! 导出用的是 SQLCipher 自己的 `sqlcipher_export`：往一个**新文件**上 `ATTACH … KEY`，
//! 把全部内容拷过去。于是导出件与库是**同一种文件**、同一个读者、同一批已测过的代码 ——
//! 它不需要还原就能直接被 [`crate::open`] 打开（拿导出口令），也可以先还原到别处再开。
//! 自造容器（magic + 头部 + 密文）意味着**恢复路径是一段全新的代码**，而恢复路径的 bug
//! 最贵：用户只在需要它的时候才会发现它坏了。
//!
//! ## 明文那条路是**同一段代码**
//!
//! `KEY ''`（空 key）让 SQLCipher **不挂 codec**，于是同一个 `sqlcipher_export` 写出一个
//! 明文 sqlite 文件（实测，见 plan 0404 的实施记录）。刻意**不做**第二条实现路径：
//! 两条路就是两份会漂移的代码，而漂移的方向总是"明文那条少一个校验"。
//!
//! ## 三段式：`…partial` → 原子改名
//!
//! 直接往目标路径写的话，中途失败会留下一个**看起来像导出件、其实打不开**的文件 ——
//! 而用户以为备份好了。所以先写 `<目标>.partial`（同目录，改名才可能是原子的），
//! 成功后 `rename` 覆盖过去；失败则把半成品删掉（D6 的"先写临时名，成功后原子改名"）。
//!
//! 写完还要**自己开一次**：加密件用真正的读者 [`crate::open`]，明文件用裸 sqlite +
//! 版本与四张表检查。这一步的代价是一次 KDF（~105 ms），换来的是"导出成功 = 这个东西
//! 真的能打开"，而不是"我们以为写完了"。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, DatabaseName, params};

use crate::{Passphrase, StoreError, open, schema};

/// 明文导出的确认短语（D6 的"二次确认"落地）。
///
/// 它**自己就把后果说了**：导出的文件里私钥是明文。逐字比较，不 trim、不忽略大小写 ——
/// "差不多对"就不是确认。
pub const PLAINTEXT_CONFIRMATION: &str = "私钥会变成明文";

/// 明文导出件文件名里必须出现的那一段（D6 的"文件名必须自曝"）。
const PLAINTEXT_MARK: &str = "plain";

/// 门槛没过时给出的两条固定原因。英文、常量、**不含用户输入**（`docs/logging.md` 的规矩：
/// 原因是一句话，变量是字段 —— 而这里连变量都不该有，文件名可能含用户的东西）。
const PHRASE_REASON: &str = "the confirmation phrase does not match";
const NAME_REASON: &str = "the file name must contain \"plain\"";

/// 明文导出的门槛凭据。
///
/// D6 要求"必须显式二次确认 + 明示内容含私钥"。**一个默认勾选的勾选框不是门槛** ——
/// 它可以被闭眼点过去，也可以在实现里被当成"默认值"。所以门槛做成类型事实：
/// [`to_plaintext`] 只收这个类型，而它
///
/// - **没有 `Default`**、不从 `bool` 造，
/// - 唯一的构造口 [`PlaintextAck::typed`] 要求逐字敲对一句后果说明。
///
/// 于是"没有确认"在**编译期**就走不到那条路，而不是靠调用方记得传一个 `true`。
///
/// ⚠️ 边界照实写：它挡的是"顺手导出明文"，挡不住明知故犯 —— 后者本来就有权这么做
/// （数据是他自己的）。
pub struct PlaintextAck(());

impl PlaintextAck {
    /// 用户敲下 `phrase` 之后给的凭据。不对就是拒绝，没有"近似匹配"。
    pub fn typed(phrase: &str) -> Result<Self, StoreError> {
        if phrase == PLAINTEXT_CONFIRMATION {
            Ok(Self(()))
        } else {
            Err(StoreError::PlaintextRefused {
                reason: PHRASE_REASON,
            })
        }
    }
}

/// **加密导出**（D6）：把 `source` 的全部内容写到 `dest`，用 `dest_passphrase` 加密。
///
/// - `source` 是已经解好锁的连接（谁持有它、口令从哪来是 plan 0407 的事）；
/// - `dest` 是**调用方给的显式路径**（本层不做默认落点：默认落进共享目录等于把含私钥的
///   文件放到最不该放的地方）；已存在就被**原子覆盖** —— 用户是在文件选择器里点名要它的；
/// - `source_passphrase` 只用来做一件事：**拦住"导出用库口令"**（D6 的"独立口令"）。
///   它不参与写文件，所以这条检查不改变导出件的任何字节。
///
/// 返回 `Ok` 的含义是"已经写完了、而且**它自己开得起来**"。
pub fn to_encrypted(
    source: &Connection,
    source_passphrase: &mut Passphrase,
    dest: &Path,
    dest_passphrase: &mut Passphrase,
) -> Result<(), StoreError> {
    check_independent(source_passphrase, dest_passphrase)?;
    write_encrypted(source, dest, dest_passphrase)
}

/// **明文导出**（D6）：同一段代码，key 参数给空。
///
/// 两道门槛都在写文件**之前**：先要 [`PlaintextAck`]（类型层 + 短语），再要文件名自曝
/// （`…plain…`）。所以"没确认"的表现是**目标路径上什么都没有**，不是一个可疑的文件。
pub fn to_plaintext(
    source: &Connection,
    dest: &Path,
    _confirmed: PlaintextAck,
) -> Result<(), StoreError> {
    if !self_describing(dest) {
        return Err(StoreError::PlaintextRefused {
            reason: NAME_REASON,
        });
    }
    // D6：与加密那条**同一段代码**，只是 key 为空（= 不挂 codec）。
    write_vault(source, dest, &[])?;
    // 明文件用不了 `open`（它要求库里是密文头），改用裸 sqlite 读一次 ——
    // "读得出来"正是"没有加密"这条判据本身，顺带校验版本与那个版本该有的表。
    open_plaintext(dest)?;
    Ok(())
}

/// **还原加密导出件**：把一个导出文件变成一个能用的库（`restore` = 导入并替换库槽）。
///
/// 只写进**空**的位置：`dest` 已经有内容就是 [`StoreError::VaultExists`]，与 [`crate::create`]
/// 同一条规则 —— 绝不覆盖一个库，那可能是用户全部的数据。
///
/// 不合并（plan 0404 的决定）：合并要处理四套池的重名与 `key_id` / `jump_id` / `host_id` 的
/// 重新编号，做一半的合并就是静默丢数据。而"导出件就是库"让还原不需要任何新机制。
///
/// `source_passphrase` 在这里是**导出件那把口令**：它既用来打开来源，也用来执行
/// "还原后的库不能与它同口令"这条检查。
///
/// ⚠️ 来源用 [`crate::open_unmigrated`] 打开，**不是** [`crate::open`]：后者会为了迁移而
/// 写那个文件，而导出件是用户的产物（可能在只读介质上）。旧格式的来源由目标那一路升上来
/// —— 见 [`copy_tables`] 与 [`write_encrypted`]。
pub fn restore(
    source: &Path,
    source_passphrase: &mut Passphrase,
    dest: &Path,
    dest_passphrase: &mut Passphrase,
) -> Result<(), StoreError> {
    refuse_existing(dest)?;
    let (conn, _version) = crate::open_unmigrated(source, source_passphrase)?;
    to_encrypted(&conn, source_passphrase, dest, dest_passphrase)
}

/// **还原明文导出件**：把明文件重新加密成一个库（`docs/portable.md` 的"数据不锁死在
/// 我们自己挑的格式里"那条退出通道，回程）。
///
/// 刻意**没有**确认门槛：门槛拦的是"留下明文"，而这一步是**消除**明文。
pub fn restore_plaintext(
    source: &Path,
    dest: &Path,
    dest_passphrase: &mut Passphrase,
) -> Result<(), StoreError> {
    refuse_existing(dest)?;
    let (conn, _version) = open_plaintext(source)?;
    write_encrypted(&conn, dest, dest_passphrase)
}

// ── 内部：三段式写入 ────────────────────────────────────────────────────────

/// 把 `source` 的全部内容写成 `dest`（`key` 为空 = 不加密），三段式收尾。
fn write_vault(source: &Connection, dest: &Path, key: &[u8]) -> Result<(), StoreError> {
    let partial = partial_path(dest);
    // 上一轮留下的半成品：它可能是个打不开的文件，留着只会让 `create_new` 失败。
    let _ = fs::remove_file(&partial);
    create_private(&partial)?;

    if let Err(err) = copy_into(source, &partial, key) {
        discard(&partial);
        return Err(err);
    }
    if let Err(err) = fs::rename(&partial, dest) {
        discard(&partial);
        return Err(StoreError::Io(err));
    }
    Ok(())
}

/// 拷内容：`ATTACH` → `sqlcipher_export` → 显式版本号，最后**无论如何** `DETACH`。
///
/// 三个语句里的别名（`export`）必须一致 —— 写死成字面量，三处一起看。
fn copy_into(source: &Connection, dest: &Path, key: &[u8]) -> Result<(), StoreError> {
    // ⚠️ 口令经**绑定参数**送进 `ATTACH`，不拼进 SQL 文本 —— 与 D4 拒绝 `PRAGMA key = '…'`
    // 是同一个理由：文本形式会把口令写进一条语句，于是它有机会出现在错误消息与跟踪里；
    // 口令里有 `'` / `\` 或非 UTF-8 字节时，字符串形式还会被改写或直接报错。
    // 空 key 参数（明文那条路）是合法输入：SQLCipher 不挂 codec，导出的就是明文库（实测）。
    let dest = path_text(dest)?;
    source.execute(ATTACH_SQL, params![dest, key])?;

    let copied = copy_tables(source);
    let detached = source.execute_batch(DETACH_SQL).map_err(StoreError::from);
    // 两个错误都算错，但只报第一个：`Result::and` 不吞第二个（它会被 drop），
    // 而"顺序"在这里不重要 —— 两种情况都意味着这个导出件不能用。
    copied.and(detached)
}

/// `sqlcipher_export` + 版本号。
fn copy_tables(source: &Connection) -> Result<(), StoreError> {
    source.query_row(COPY_SQL, [], |_| Ok(()))?;
    // ⚠️ **必须显式写**：`sqlcipher_export` **不传递** `user_version`（上游文档 + 实测，
    // 见 ADR-0002 D7 与 §7）。不写的话每个导出件都是版本 0 —— 而版本 0 的库我们自己会拒
    // （D7：v1 之前没有版本，"0" 说明它不是本程序写的库）。于是"导出成功、还原打不开"。
    //
    // 抄的是**来源的**版本，不是写死的 `FORMAT_VERSION`：还原一个 v1 时代的导出件时，
    // "导出件就是库"（D6）要求先原样抄成 v1，再由 `write_encrypted` 末尾那次 `open`
    // 把它升上来 —— 写死当前版本会造出一个"版本号说 v2、表却只有 v1 那四张"的文件，
    // 而那种文件正是 D7 要防的"能开但形状不对"。
    let version: i64 = source.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    source.pragma_update(
        Some(DatabaseName::Attached(EXPORT_ALIAS)),
        "user_version",
        version,
    )?;
    Ok(())
}

/// 加密件写完再开一次 —— 用**真正的读者**（[`crate::open`]：送密钥、校验版本、
/// 迁移旧格式、查该版本的表、收紧权限）。它成功就意味着"这个导出件在另一台机器上
/// 也能被同一个代码路径打开"。
///
/// 旧来源（v1）的还原在这里被**升到当前格式**：抄过来的 v1 内容在这条路上变成 v2 的库 ——
/// 升级发生在**新写出来的目标**上，用户的导出件一个字节都没动。
fn write_encrypted(
    source: &Connection,
    dest: &Path,
    dest_passphrase: &mut Passphrase,
) -> Result<(), StoreError> {
    {
        // 提权窗口只在这一次写里打开：`write_vault` 走完，key 就从进程里消失了
        // （受保护页回到静止态，源缓冲那条路也由上游擦零）。
        let key = dest_passphrase.expose()?;
        write_vault(source, dest, &key)?;
    }
    open(dest, dest_passphrase)?;
    Ok(())
}

/// 用**裸 sqlite** 打开一个明文件，并校验版本与**该版本**的表。
///
/// 两种用法共用这一个读者：明文导出写完的自检、明文件还原时的来源检查。
/// 拿一个**加密**的库喂给它，第一页就读不出来 —— 那时错误是"不是个库"，
/// 与 [`crate::open`] 收到错口令时的说法一致（SQLCipher 对这两者本来就给同一个
/// `SQLITE_NOTADB`）。
///
/// **不迁移**：明文件可能就是用户抽屉里那份 v1 时代的导出，我们不去改写它（同
/// [`crate::open_unmigrated`]）。返回版本号让调用方原样抄走。
fn open_plaintext(path: &Path) -> Result<(Connection, i64), StoreError> {
    if crate::file_len(path)? == 0 {
        return Err(StoreError::NoVault(path.to_path_buf()));
    }
    let conn = Connection::open(path)?;
    let found: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(crate::as_database_error)?;
    // 认识的旧版本照样认（"导出件会长期躺在用户的抽屉里"，D7）—— 认不出的在这里拒绝。
    schema::check(&conn, found)?;
    Ok((conn, found))
}

/// 导出口令不得与它导出时那把相同（D6："加密导出用独立口令，不复用库口令"）。
///
/// 为什么值得做成运行时检查：导出件常被写进 U 盘、发到别处、写在便签上 —— 而它一旦
/// 用了库口令，那把口令的暴露面就从"用户脑子里"扩散到"导出件经过的每一个地方"，
/// 且暴露的东西**能打开用户的库**。这条约束留在文档里必然失守，而这里只需要一次比较。
///
/// ⚠️ 两个提权窗口同时打开（各自用 `&mut` 借一次）；比较不是恒定时间的 ——
/// 两边都是同一个用户刚输入的东西，没有可被利用的预言机。
fn check_independent(from: &mut Passphrase, to: &mut Passphrase) -> Result<(), StoreError> {
    let source = from.expose()?;
    let export = to.expose()?;
    if *source == *export {
        return Err(StoreError::SharedPassphrase);
    }
    Ok(())
}

/// 还原只写进空位置（与 [`crate::create`] 同一条规则）。0 字节的文件算"空"：
/// 那是上一次失败留下的壳，不是数据（实测：0 字节的库文件用什么口令都能"打开"）。
fn refuse_existing(dest: &Path) -> Result<(), StoreError> {
    if crate::file_len(dest)? > 0 {
        return Err(StoreError::VaultExists(dest.to_path_buf()));
    }
    Ok(())
}

/// 明文件的文件名必须自曝（D6）。大小写不敏感：`PLAIN` 也是自曝。
fn self_describing(dest: &Path) -> bool {
    dest.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().contains(PLAINTEXT_MARK))
}

/// `…<名字>.partial`：与目标**同目录**（跨文件系统的改名不是原子的），
/// 名字里带 `partial` 则自曝它是半成品。
fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".partial");
    PathBuf::from(name)
}

/// 预建一个 0600 的空文件：导出件**从头就是 0600**（D12），而不是"先按 umask 建出来、
/// 事后收紧"—— 中间那段时间里它已经是含私钥的密文，权限位不该由 umask 决定。
fn create_private(path: &Path) -> Result<(), StoreError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?;
    Ok(())
}

/// 删掉半成品。**尽力而为**：失败也不改变结论（这次导出已经失败了），
/// 而"删不掉"最可能的原因是路径本来就不归我们管。
fn discard(path: &Path) {
    let _ = fs::remove_file(path);
}

/// 路径进 SQL 参数要求是文本（SQLite 的 C API 收 UTF-8 文件名）。
/// 非 UTF-8 路径在 unix 上可能，但这里**明确报错**而不是猜测转码。
fn path_text(path: &Path) -> Result<&str, StoreError> {
    path.to_str().ok_or_else(|| {
        StoreError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "export path is not valid UTF-8",
        ))
    })
}

/// 导出用的 attaches 别名。三个语句必须一致（`ATTACH` / `sqlcipher_export` / `DETACH`）。
const EXPORT_ALIAS: &str = "export";
const ATTACH_SQL: &str = "ATTACH DATABASE ?1 AS export KEY ?2";
const COPY_SQL: &str = "SELECT sqlcipher_export('export')";
const DETACH_SQL: &str = "DETACH DATABASE export";
