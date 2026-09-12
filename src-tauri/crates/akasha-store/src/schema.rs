//! 库的**磁盘格式**：四套池的表结构（ADR-0002 D7 / D1）。
//!
//! 这个模块是 `user_version = 1` 那个 `1` 的**定义处**：v1 的含义就是下面这段 DDL，
//! 不多不少。改一个列名、加一个列，都等于换了一个格式 —— 那时 `FORMAT_VERSION` 要 +
//! 并给出迁移，而不是让新代码去读一个"看起来能开、其实形状不同"的库（D7 的意图）。
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

use rusqlite::Connection;

use crate::StoreError;

/// v1 的四张表，**顺序就是建表顺序**（`hosts` 引用 `keys`，`forwards` 引用 `hosts`）。
///
/// 这份清单同时是 [`check`] 的判据：`user_version = 1` 而缺其中任何一张，
/// 那个库就不是本程序写的（或写到一半被打断），要明确拒绝而不是"开起来看着像空的"。
pub const TABLES: [&str; 4] = ["keys", "hosts", "serials", "forwards"];

/// v1 的建表语句。只在 [`crate::create`] 里跑**一次**。
const DDL: &str = "
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

/// 建表。**只在空库上跑**（[`crate::create`] 的路径），不在打开路径上跑任何 DDL。
pub(crate) fn create(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(DDL)?;
    Ok(())
}

/// 查四张表在不在。少任何一张 → [`StoreError::MissingTable`]。
///
/// 只查**表**，不查列：列的形状由 `user_version` 负责（[`TABLES`] 上方那段）。
/// 想在打开时把列也比一遍，就得在这里再写一份 DDL 的镜像 —— 那份镜像迟早与 DDL 不一致，
/// 而"不一致的检查"比没有检查更坏（它会在正确的事情上报错）。
pub(crate) fn check(conn: &Connection) -> Result<(), StoreError> {
    let mut stmt =
        conn.prepare("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1")?;
    for table in TABLES {
        let found: i64 = stmt.query_row([table], |row| row.get(0))?;
        if found == 0 {
            return Err(StoreError::MissingTable { table });
        }
    }
    Ok(())
}
