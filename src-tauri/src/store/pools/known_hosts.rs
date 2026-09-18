//! **known_hosts 缓存**：我们记下来的主机密钥（ADR-0003 **D11**）。
//!
//! 它是**缓存**，不是第 5 套池 —— 三条区别都是有意的：
//!
//! * 没有 `name`，不从界面新建：它由"用户确认了一把没见过的密钥"这件事写进来；
//! * 它装的**全是公开信息**（主机密钥本来就是公开的），所以不涉及受保护页（D13）；
//! * 它不进 `dump` 的四套池清单 —— 用户要备份的是他配置的东西，不是我们记的指纹。
//!
//! ## 为什么"记住"这件事不许静默改写
//!
//! [`remember`] 遇到**同一个 `(host, port, key_type)` 上已经记着一把不同的密钥**时返回
//! [`StoreError::Conflict`]，而不是覆盖。D11 的原话是"既不静默接受、也不静默改写"——
//! 把这条约束放在**写路径**上，而不是指望每个调用方都记得先比对：密钥变化是**攻击**的
//! 典型形态（中间人换了密钥），而"顺手更新一下缓存"正是它会伪装成的样子。
//!
//! 要接受一把新密钥，调用方必须先 [`forget_host`] —— 那是一次**显式的**动作，
//! 也正是让用户看见"你正在丢掉一把旧密钥"的时刻。

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::constrained;
use crate::store::StoreError;

/// 池名。写死成常量而不是让调用方拼字符串：错误消息里的池名必须与实际表名一致。
const POOL: &str = "known_hosts";

/// 记下来的一把主机密钥。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHost {
    pub id: i64,
    pub host: String,
    pub port: u16,
    /// 密钥算法（`ssh-ed25519` 一类）。
    pub key_type: String,
    /// SSH 线格式的密钥本体 —— **判定的材料**（逐字节比）。
    pub key_blob: Vec<u8>,
    /// 给人核对的那串 `SHA256:…`，不参与判定。
    pub fingerprint: String,
}

/// 还没入库的一行（**没有 id**）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewKnownHost {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub key_blob: Vec<u8>,
    pub fingerprint: String,
}

/// 全部记录（按主机名排序 —— 界面列出来时可复现）。
pub fn known_hosts(conn: &Connection) -> Result<Vec<KnownHost>, StoreError> {
    let mut stmt = conn.prepare("SELECT * FROM known_hosts ORDER BY host, port, key_type")?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 读一行。
pub fn known_host(conn: &Connection, id: i64) -> Result<KnownHost, StoreError> {
    conn.query_row("SELECT * FROM known_hosts WHERE id = ?1", [id], from_row)
        .optional()?
        .ok_or(StoreError::NoSuchRow { pool: POOL, id })
}

/// 这台主机上这个类型的密钥记的是什么（没有 → `None` = **未知**）。
///
/// 只按 `(host, port, key_type)` 查：**类型不同不算同一条记录**。这条与上游
/// `check_known_hosts_path` 的语义对齐（它在类型不同的记录上返回"不匹配"而不是"变化了"），
/// 免得服务端换掉算法时被误判成密钥变化。
pub fn lookup(
    conn: &Connection,
    host: &str,
    port: u16,
    key_type: &str,
) -> Result<Option<KnownHost>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT * FROM known_hosts WHERE host = ?1 AND port = ?2 AND key_type = ?3",
            params![host, port, key_type],
            from_row,
        )
        .optional()?)
}

/// 记下一把**用户确认过**的密钥，返回它的 `id`。
///
/// 已经记着**同一把** → 幂等（返回原 id）：用户对同一台主机点两次"确认"不该是错误，
/// 也不该多出一行。
///
/// 已经记着**别的** → [`StoreError::Conflict`]，**绝不覆盖**（理由见模块文档）。
pub fn remember(conn: &Connection, new: &NewKnownHost) -> Result<i64, StoreError> {
    // 这次查询只为**说清是什么冲突**（"记的是 A、现在来的是 B"）；真正的保证在
    // `UNIQUE (host, port, key_type)` 上 —— 绕过这个函数直接写 SQL 也覆盖不了。
    if let Some(existing) = lookup(conn, &new.host, new.port, &new.key_type)? {
        if existing.key_blob == new.key_blob {
            return Ok(existing.id);
        }
        return Err(StoreError::Conflict {
            pool: POOL,
            detail: format!(
                "a different key is already recorded for this host: {} → {}",
                existing.fingerprint, new.fingerprint
            ),
        });
    }

    let rows = constrained(
        POOL,
        conn.execute(
            "INSERT INTO known_hosts (host, port, key_type, key_blob, fingerprint)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                new.host,
                new.port,
                new.key_type,
                new.key_blob,
                new.fingerprint
            ],
        ),
    )?;
    debug_assert_eq!(rows, 1, "INSERT 只该影响一行");
    Ok(conn.last_insert_rowid())
}

/// 忘掉一台主机的**全部**密钥（所有类型），返回删掉几行。
///
/// 这是"用户决定不再信任这台主机 / 要接受它的新密钥"的那个动作。删不存在的记录**不报错**
/// （结果是 0 行）：目标状态已经达成，而"我以为它记着"不是用户能处理的错误。
pub fn forget_host(conn: &Connection, host: &str, port: u16) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "DELETE FROM known_hosts WHERE host = ?1 AND port = ?2",
        params![host, port],
    )?)
}

/// 忘掉一行（按 id）。不在 → [`StoreError::NoSuchRow`]：调用方点的是**某一项**，
/// 它已经不在了就必须说出来（界面上的那一项是过期的）。
pub fn forget(conn: &Connection, id: i64) -> Result<(), StoreError> {
    let rows = conn.execute("DELETE FROM known_hosts WHERE id = ?1", [id])?;
    if rows == 0 {
        return Err(StoreError::NoSuchRow { pool: POOL, id });
    }
    Ok(())
}

/// 清空全部记录，返回删掉几行。
pub fn clear(conn: &Connection) -> Result<usize, StoreError> {
    Ok(conn.execute("DELETE FROM known_hosts", [])?)
}

/// 列顺序与 `schema.rs` 里 v2 那段 DDL 一致。
fn from_row(row: &Row<'_>) -> rusqlite::Result<KnownHost> {
    Ok(KnownHost {
        id: row.get(0)?,
        host: row.get(1)?,
        port: row.get(2)?,
        key_type: row.get(3)?,
        key_blob: row.get(4)?,
        fingerprint: row.get(5)?,
    })
}
