//! **端口转发规则池**：方向（L/R/D）/ 绑定地址与端口 / 目标 / 所属主机 / 自启（`scope.md` §3）。
//!
//! 三条约束写进了 DDL，因为它们用一句话就能说清、而且**必须是库自己拦得住的不变量**：
//!
//! 1. **`dynamic`（SOCKS5）没有目标**：`-D` 的目标是"客户端自己说要去哪"，
//!    配一个 `target_host` 是自相矛盾的记录 —— `CHECK` 让它进不来；
//! 2. **`local` / `remote` 必须有目标**（同一个 `CHECK` 的另一半）；
//! 3. **规则必须属于一台主机**（`host_id NOT NULL` + 外键 `ON DELETE RESTRICT`）：
//!    "没有所属主机的转发规则"在语义上是空的，而且 `-R` 的绑定端在**远端**上，
//!    没有远端就没有它存在的地方。
//!
//! 方向与作用域的关系（哪一端是绑定的、哪一端是目标）由 `scope.md` §4 与 ADR-0003 定，
//! 这一层只存"用户配置了什么"。

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{constrained, read_enum, touched};
use crate::StoreError;

/// 池名。写死成常量而不是让调用方拼字符串：错误消息里的池名必须与实际表名一致。
const POOL: &str = "forwards";

/// 转发方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `-L`：本地绑定，转发到目标。
    Local,
    /// `-R`：远端绑定，转发回本地侧。
    Remote,
    /// `-D`：本地起一个 SOCKS5，目标由客户端给。
    Dynamic,
}

impl Direction {
    pub const ALL: [Self; 3] = [Self::Local, Self::Remote, Self::Dynamic];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Dynamic => "dynamic",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|direction| direction.as_str() == raw)
    }
}

/// 转发规则池里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forward {
    pub id: i64,
    pub name: String,
    pub direction: Direction,
    pub bind_host: String,
    pub bind_port: u16,
    /// `dynamic` 时是 `None`（不变量在 DDL 的 `CHECK` 里）。
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
    /// 所属主机（`hosts.id`）。
    pub host_id: i64,
    /// 会话建立时是否自动起这条转发。
    pub autostart: bool,
}

/// 还没入库的一行（**没有 id**）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewForward {
    pub name: String,
    pub direction: Direction,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
    pub host_id: i64,
    pub autostart: bool,
}

/// 插入一行，返回新的 `id`。
pub fn insert_forward(conn: &Connection, new: &NewForward) -> Result<i64, StoreError> {
    let rows = constrained(
        POOL,
        conn.execute(
            "INSERT INTO forwards (name, direction, bind_host, bind_port, target_host, target_port,
                                   host_id, autostart)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                new.name,
                new.direction.as_str(),
                new.bind_host,
                new.bind_port,
                new.target_host,
                new.target_port,
                new.host_id,
                new.autostart
            ],
        ),
    )?;
    debug_assert_eq!(rows, 1, "INSERT 只该影响一行");
    Ok(conn.last_insert_rowid())
}

/// 读一行。
pub fn forward(conn: &Connection, id: i64) -> Result<Forward, StoreError> {
    conn.query_row("SELECT * FROM forwards WHERE id = ?1", [id], from_row)
        .optional()?
        .ok_or(StoreError::NoSuchRow { pool: POOL, id })
}

/// 全部规则，按名字排序（顺序确定，才谈得上"可断言的输出"）。
pub fn forwards(conn: &Connection) -> Result<Vec<Forward>, StoreError> {
    let mut stmt = conn.prepare("SELECT * FROM forwards ORDER BY name")?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 某台主机上的全部规则（会话建立时按它决定起哪几条）。
pub fn forwards_of_host(conn: &Connection, host_id: i64) -> Result<Vec<Forward>, StoreError> {
    let mut stmt = conn.prepare("SELECT * FROM forwards WHERE host_id = ?1 ORDER BY name")?;
    let rows = stmt.query_map([host_id], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 全量替换一行。
pub fn update_forward(conn: &Connection, forward: &Forward) -> Result<(), StoreError> {
    let rows = constrained(
        POOL,
        conn.execute(
            "UPDATE forwards SET name = ?1, direction = ?2, bind_host = ?3, bind_port = ?4,
                    target_host = ?5, target_port = ?6, host_id = ?7, autostart = ?8
             WHERE id = ?9",
            params![
                forward.name,
                forward.direction.as_str(),
                forward.bind_host,
                forward.bind_port,
                forward.target_host,
                forward.target_port,
                forward.host_id,
                forward.autostart,
                forward.id
            ],
        ),
    )?;
    touched(POOL, forward.id, rows)
}

/// 删一行。所属主机还在不影响它（这是"主机上的规则"，方向反着）。
pub fn delete_forward(conn: &Connection, id: i64) -> Result<(), StoreError> {
    let rows = constrained(
        POOL,
        conn.execute("DELETE FROM forwards WHERE id = ?1", [id]),
    )?;
    touched(POOL, id, rows)
}

/// 按**列名**读一行。
fn from_row(row: &Row<'_>) -> rusqlite::Result<Forward> {
    Ok(Forward {
        id: row.get("id")?,
        name: row.get("name")?,
        direction: read_enum(row.get("direction")?, "direction", Direction::parse)?,
        bind_host: row.get("bind_host")?,
        bind_port: row.get("bind_port")?,
        target_host: row.get("target_host")?,
        target_port: row.get("target_port")?,
        host_id: row.get("host_id")?,
        autostart: row.get("autostart")?,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn every_value_round_trips_through_its_text_form() {
        for direction in Direction::ALL {
            assert_eq!(Direction::parse(direction.as_str()), Some(direction));
        }
    }

    #[test]
    fn an_unknown_value_is_not_a_default() {
        // `-R`/`-L`/`-D` 是命令行写法，库里存的是词；别让两种写法都"能读"——
        // 两边都认等于库里有两套取值。
        assert_eq!(Direction::parse("L"), None);
        assert_eq!(Direction::parse("socks5"), None);
    }
}
