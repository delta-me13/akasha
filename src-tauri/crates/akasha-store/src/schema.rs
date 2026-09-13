//! 库的**磁盘格式**：v1 的四套池（ADR-0002 D7 / D1）+ v2 的 known_hosts 缓存（ADR-0003 D11）。
//!
//! 这个模块是 `user_version` 那两个取值的**定义处**：写的是几，形状就是下面这几段 DDL，
//! 不多不少。改一个列名、加一个列、加一张表，都等于换了一个格式 —— 那时 `FORMAT_VERSION`
//! 要 + 并在这里给出**迁移**，而不是让新代码去读一个"看起来能开、其实形状不同"的库（D7 的意图）。
//!
//! 三处刻意的写法：
//!
//! 1. **`STRICT` 表**：列类型是真的 —— 往 `INTEGER` 列里塞字符串会当场报错，而不是
//!    悄悄按 SQLite 的"类型亲和性"存下去。库是唯一真相源，那么"存进去的是什么"也不该
//!    由亲和性规则决定。前提是 SQLite ≥ 3.37（`sqlite_version()` 由
//!    `tests/schema_contract.rs` 断言，免得这行字变成一句没人验过的假设）。
//! 2. **`CHECK` 写在不变量上**，能表达的就不留给应用层：端口的范围、`dynamic` 没有目标
//!    主机、`key_id` 只对 `publickey` 有意义。库自己拦下来的东西，绕过任何一条代码路径
//!    都拦得住（将来的导入、dump 恢复、手工修库都算）。
//! 3. **外键一律 `ON DELETE RESTRICT`**：删一把还在被引用的密钥 / 主机 / 规则，是**报错**，
//!    不是把引用悄悄清空。⚠️ 但 sqlite 的 `PRAGMA foreign_keys` **默认是关的** ——
//!    声明了外键不等于它会生效，这是一条"静默失效"的经典形态，所以
//!    [`crate::unlock`] 每个连接都开一次，并由 `tests/schema_contract.rs` 读回断言。
//!
//! **没有 `*_path` / `*_dir` / `*_file` 列，也不许加**（`portable.md` §2 第 2 条）：
//! 库里存绝对路径，搬走文件夹之后它们会**静默失效**。密钥存在库里（BLOB 列）而不是
//! 存"密钥文件在哪"，正是这条要求的实现方式。唯一像路径的是 serial 的 `port`，
//! 它是**操作系统给的设备名**（`/dev/ttyUSB0` / `COM3`），不是我们的文件位置 ——
//! 见 `docs/scope.md` §3。
//!
//! ## 版本与迁移
//!
//! - **v1 = 四张池表**（plan 0403 起）；**v2 = v1 + `known_hosts`**（plan 0503）。
//! - [`DDL_V1`] 是 v1 的**冻结定义**：验证迁移要能造出一个**真 v1 库**，而"真 v1"只能有
//!   一个定义处，所以它公开 —— 公开的是**历史格式的文本**，不是一条绕开池的写入路径。
//! - 迁移的规则（本仓库第一次，以后照抄）：**一次事务**里加表并写 `user_version`，
//!   失败整体回滚；由 [`crate::open`] 自动做（理由见 [`crate::upgrade`]）。

use rusqlite::Connection;

use crate::StoreError;

/// v1 的四张表，**顺序就是建表顺序**（`hosts` 引用 `keys`，`forwards` 引用 `hosts`）。
///
/// 这份清单同时是 [`check`] 的判据：`user_version = 1` 而缺其中任何一张，
/// 那个库就不是本程序写的（或写到一半被打断），要明确拒绝而不是"开起来看着像空的"。
pub const TABLES_V1: [&str; 4] = ["keys", "hosts", "serials", "forwards"];

/// **当前**格式（v2）的表：v1 那四张 + known_hosts。
///
/// known_hosts 是**缓存**不是池：它没有名字、不从界面新建，装的也全是公开信息
/// （主机密钥本来就是公开的）—— 所以它不进 `dump` 那份"四套池"清单。
pub const TABLES: [&str; 5] = ["keys", "hosts", "serials", "forwards", "known_hosts"];

/// v1 的建表语句，**冻结**：这是"v1 是什么"的定义（D7）。
///
/// ⚠️ 不许改这里一个字符。改了就等于改写历史 —— 而 [`migrate_step`] 与
/// `tests/format_migration.rs` 都靠它造一个**真正的 v1 库**；历史被改掉之后，
/// 那些测试验的就不再是"用户的旧库"了。
pub const DDL_V1: &str = "
CREATE TABLE keys (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL UNIQUE,
    private_pem BLOB    NOT NULL,
    public_key  TEXT    NOT NULL DEFAULT '',
    comment     TEXT
) STRICT;

CREATE TABLE hosts (
    id      INTEGER PRIMARY KEY,
    name    TEXT    NOT NULL UNIQUE,
    host    TEXT    NOT NULL,
    port    INTEGER NOT NULL DEFAULT 22 CHECK (port BETWEEN 1 AND 65535),
    user    TEXT    NOT NULL,
    auth    TEXT    NOT NULL CHECK (auth IN ('password', 'publickey', 'agent')),
    key_id  INTEGER REFERENCES keys(id) ON DELETE RESTRICT,
    jump_id INTEGER REFERENCES hosts(id) ON DELETE RESTRICT,
    CHECK (key_id IS NULL OR auth = 'publickey'),
    CHECK (jump_id IS NULL OR jump_id <> id)
) STRICT;

CREATE TABLE serials (
    id        INTEGER PRIMARY KEY,
    name      TEXT    NOT NULL UNIQUE,
    port      TEXT    NOT NULL,
    baud      INTEGER NOT NULL CHECK (baud > 0),
    data_bits INTEGER NOT NULL CHECK (data_bits IN (5, 6, 7, 8)),
    stop_bits INTEGER NOT NULL CHECK (stop_bits IN (1, 2)),
    parity    TEXT    NOT NULL CHECK (parity IN ('none', 'even', 'odd')),
    flow      TEXT    NOT NULL CHECK (flow IN ('none', 'software', 'hardware'))
) STRICT;

CREATE TABLE forwards (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL UNIQUE,
    direction   TEXT    NOT NULL CHECK (direction IN ('local', 'remote', 'dynamic')),
    bind_host   TEXT    NOT NULL,
    bind_port   INTEGER NOT NULL CHECK (bind_port BETWEEN 1 AND 65535),
    target_host TEXT,
    target_port INTEGER CHECK (target_port BETWEEN 1 AND 65535),
    host_id     INTEGER NOT NULL REFERENCES hosts(id) ON DELETE RESTRICT,
    autostart   INTEGER NOT NULL DEFAULT 0 CHECK (autostart IN (0, 1)),
    CHECK ((direction = 'dynamic') = (target_host IS NULL AND target_port IS NULL))
) STRICT;
";

/// v2 相对 v1 **加**的东西（`user_version` 1 → 2）。
///
/// `known_hosts` 的列与理由：
///
/// - `key_blob` 是 SSH 线格式的密钥本体 —— **判定的材料**。拿指纹文本比对也行得通，
///   但那是把"同一把密钥"押在一段有损的字符串表示上；逐字节比 blob 才是同一件事。
/// - `fingerprint` 是给人核对的那串 `SHA256:…`，**不参与判定**。
/// - `UNIQUE (host, port, key_type)`：同一台主机的同一种密钥类型只认一把。
///   **不同类型各记一行**是照上游 `check_known_hosts_path` 的语义来的（类型不同不算不匹配），
///   免得服务端换掉算法时被误判成"密钥变了"。
const DDL_V2: &str = "
CREATE TABLE known_hosts (
    id          INTEGER PRIMARY KEY,
    host        TEXT    NOT NULL,
    port        INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    key_type    TEXT    NOT NULL,
    key_blob    BLOB    NOT NULL,
    fingerprint TEXT    NOT NULL,
    UNIQUE (host, port, key_type)
) STRICT;
";

/// 建当前格式（v2）的全部表。**只在空库上跑**（[`crate::create`] 的路径）。
pub(crate) fn create(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(DDL_V1)?;
    conn.execute_batch(DDL_V2)?;
    Ok(())
}

/// 查**某个版本**的库该有的表在不在。少任何一张 → [`StoreError::MissingTable`]。
///
/// 只查**表**，不查列：列的形状由 `user_version` 负责（[`TABLES_V1`] 上方那段）。
/// 想在打开时把列也比一遍，就得在这里再写一份 DDL 的镜像 —— 那份镜像迟早与 DDL 不一致，
/// 而"不一致的检查"比没有检查更坏（它会在正确的事情上报错）。
///
/// ⚠️ 带 `version` 参数是本仓库第一处**版本相关**的检查：迁移要**先**按旧版本确认形状
/// （一个 v1 库缺了 `keys` 表就不是"待迁移"，而是坏了），再动手加表。
pub(crate) fn check(conn: &Connection, version: i64) -> Result<(), StoreError> {
    let tables = tables_of(version).ok_or(StoreError::UnsupportedVersion { found: version })?;
    let mut stmt =
        conn.prepare("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1")?;
    for table in tables {
        let found: i64 = stmt.query_row([table], |row| row.get(0))?;
        if found == 0 {
            return Err(StoreError::MissingTable { table });
        }
    }
    Ok(())
}

/// 某个版本的库该有哪些表。**不认识就是 `None`** —— 由调用方翻成拒绝，不给默认值。
fn tables_of(version: i64) -> Option<&'static [&'static str]> {
    match version {
        1 => Some(TABLES_V1.as_slice()),
        v if v == crate::FORMAT_VERSION => Some(TABLES.as_slice()),
        _ => None,
    }
}

/// 单步迁移：`from` → `from + 1`，返回**新**版本号。
///
/// 这一版只加表、不动已有列 —— 所以不需要重建表、不需要搬数据。真到了要改列的那天，
/// 这里是"建新表 + `INSERT INTO … SELECT` + 改名"那三步该在的地方（它们的顺序不能反）。
///
/// **调用方负责事务与写 `user_version`**：让每一步自带事务，会让"v1 → v3"那种连走两步的
/// 迁移变成两个事务，中间崩掉就留下一个没人认得的版本号。
pub(crate) fn migrate_step(conn: &Connection, from: i64) -> Result<i64, StoreError> {
    match from {
        1 => {
            conn.execute_batch(DDL_V2)?;
            Ok(2)
        }
        other => Err(StoreError::UnsupportedVersion { found: other }),
    }
}
