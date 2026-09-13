//! **ssh 配置池**：host / port / user / 认证方式 / 跳板（`scope.md` §3）。
//!
//! 字段集是 `~/.ssh/config` 的**受限子集**（"从系统 ssh config 导入"是阶段 5 的 plan 0504）。
//! 两条刻意的取舍：
//!
//! * **库里不许重名**（`name UNIQUE`）。`~/.ssh/config` 允许重复的 `Host` 块、先匹配者生效 ——
//!   那是文本文件的历史包袱（写下去时不知道后面还会有人写同名块），不是我们要继承的语义：
//!   库里的重名是"两行看起来一样、行为取决于顺序"，而顺序没有地方能看见。
//! * **不存口令**。认证方式只是一个取值；口令从哪来是运行时的事（Bitwarden / 交互输入，
//!   见 `scope.md` §6）。把口令存进配置池等于把"配置"和"机密"混在一起，
//!   而机密有自己的一套去处（ADR-0002 D13）。
//!
//! ## 跳板链为什么不能成环（以及这里只挡了一半）
//!
//! 自环（`A` 的跳板是 `A`）由表的 `CHECK` 挡住。`A → B → C → A` 这种环**库表达不出来**
//! （要递归查询），所以它在写入路径上由一个显式走链的检查挡住（[`update_host`]）。
//! 只有 `update` 需要走这条链：**新插入的行此刻还没有被任何人引用**，所以它不可能成环
//! （数学归纳：老数据无环 + 新行无入边 = 仍然无环）。
//!
//! 走链还有一个深度上限（[`MAX_JUMP_DEPTH`]）兜底：真有人手工把库改出一个环来，
//! 我们要报错而不是**死循环**。

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{MAX_JUMP_DEPTH, constrained, read_enum, touched};
use crate::StoreError;

/// 池名。写死成常量而不是让调用方拼字符串：错误消息里的池名必须与实际表名一致。
const POOL: &str = "hosts";

/// 认证方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    Password,
    PublicKey,
    Agent,
}

impl Auth {
    /// 全部取值。兼作 `parse` 的**唯一**清单 —— 两张清单会漂移，一张不会。
    pub const ALL: [Self; 3] = [Self::Password, Self::PublicKey, Self::Agent];

    /// 库里存的文本（也是 DDL 的 `CHECK` 里那份取值）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::PublicKey => "publickey",
            Self::Agent => "agent",
        }
    }

    /// 读回来。不认识就 `None`（由读路径翻成错误，**不给默认值**）。
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|auth| auth.as_str() == raw)
    }
}

/// ssh 配置池里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    /// 用哪把密钥（`keys.id`）。只有 `auth = 'publickey'` 时才有意义 ——
    /// 而 `publickey` 也**可以**没有 `key_id`（走 ssh-agent 的钥匙不在我们的池里）。
    pub key_id: Option<i64>,
    /// 跳板：另一台 `hosts.id`。
    pub jump_id: Option<i64>,
}

/// 还没入库的一行（**没有 id**）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewHost {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    pub key_id: Option<i64>,
    pub jump_id: Option<i64>,
}

/// 插入一行，返回新的 `id`。
///
/// 不需要走跳板链检查（理由见模块文档）。库这一侧仍然会拦住：跳板不存在（外键）、
/// 跳板是自己（`CHECK`）、`key_id` 配了 `password`/`agent`（`CHECK`）、重名（`UNIQUE`）。
pub fn insert_host(conn: &Connection, new: &NewHost) -> Result<i64, StoreError> {
    let rows = constrained(
        POOL,
        conn.execute(
            "INSERT INTO hosts (name, host, port, user, auth, key_id, jump_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new.name,
                new.host,
                new.port,
                new.user,
                new.auth.as_str(),
                new.key_id,
                new.jump_id
            ],
        ),
    )?;
    debug_assert_eq!(rows, 1, "INSERT 只该影响一行");
    Ok(conn.last_insert_rowid())
}

/// 读一行。
pub fn host(conn: &Connection, id: i64) -> Result<Host, StoreError> {
    conn.query_row("SELECT * FROM hosts WHERE id = ?1", [id], from_row)
        .optional()?
        .ok_or(StoreError::NoSuchRow { pool: POOL, id })
}

/// 全部主机，按名字排序（顺序确定，才谈得上"可断言的输出"）。
pub fn hosts(conn: &Connection) -> Result<Vec<Host>, StoreError> {
    let mut stmt = conn.prepare("SELECT * FROM hosts ORDER BY name")?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 全量替换一行。
///
/// 这是唯一需要挡跳板环的地方（[`assert_no_cycle`]）：把 `A` 的跳板改成 `C`，
/// 而 `C` 的链上又回到 `A` —— 那以后每次连接都会顺着链绕圈。
pub fn update_host(conn: &Connection, host: &Host) -> Result<(), StoreError> {
    assert_no_cycle(conn, host.id, host.jump_id)?;
    let rows = constrained(
        POOL,
        conn.execute(
            "UPDATE hosts SET name = ?1, host = ?2, port = ?3, user = ?4, auth = ?5,
                    key_id = ?6, jump_id = ?7
             WHERE id = ?8",
            params![
                host.name,
                host.host,
                host.port,
                host.user,
                host.auth.as_str(),
                host.key_id,
                host.jump_id,
                host.id
            ],
        ),
    )?;
    touched(POOL, host.id, rows)
}

/// 删一行。**还有转发规则引用它时删不掉**（外键 `ON DELETE RESTRICT`）。
/// 仍然被别的行当跳板时同理 —— 那是"这台机器还在别人的链上"。
pub fn delete_host(conn: &Connection, id: i64) -> Result<(), StoreError> {
    let rows = constrained(POOL, conn.execute("DELETE FROM hosts WHERE id = ?1", [id]))?;
    touched(POOL, id, rows)
}

/// 把 `id` 当跳板的主机（"我改这台机器会不会影响到别人"）。
pub fn hosts_jumping_to(conn: &Connection, id: i64) -> Result<Vec<Host>, StoreError> {
    let mut stmt = conn.prepare("SELECT * FROM hosts WHERE jump_id = ?1 ORDER BY name")?;
    let rows = stmt.query_map([id], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 整条跳板链，**从要连的那台往上走**：`[这台, 它的跳板, 跳板的跳板, …]`。
///
/// 顺序是这么定的：链的语义本来就是"我要连**这台**，它得先经**那台**"，所以第一项是目标。
/// 连接那条路（`akasha-ssh` 的 `connect_via`）要的是反过来的顺序，它自己 `rev` 一下 ——
/// 让**读**这一侧保持"从目标往回走"的直觉，比让每个调用方都想一遍"哪个是最外层"要好。
///
/// ## 为什么读路径也要挡住环与深度
///
/// [`update_host`] 挡住的是**我们的**写入。它挡不住有人拿 `sqlite3` 改库、挡不住旧版本的
/// bug、也挡不住从别处还原回来的一份文件 —— 而链上真有环时，连接那条路会顺着环走下去：
/// 那不是"报错"，是**挂住**（而"挂住"在用户看来就是点了没反应）。
/// 所以深度上限与成环判定在**读**这一侧也各有一份，代价是一次 `Vec` 扫描。
pub fn jump_chain(conn: &Connection, id: i64) -> Result<Vec<Host>, StoreError> {
    let mut chain: Vec<Host> = Vec::new();
    let mut cursor = Some(id);
    while let Some(current) = cursor {
        // 已经走过这一行（成环），或者链长得离谱 —— 两者对用户是同一件事：
        // 这条配置连不通，而且都不是能连的配置。
        if chain.len() >= MAX_JUMP_DEPTH || chain.iter().any(|host| host.id == current) {
            return Err(StoreError::JumpChain);
        }
        let row = host(conn, current)?;
        cursor = row.jump_id;
        chain.push(row);
    }
    Ok(chain)
}

/// 从 `jump_id` 沿链往上走，撞见 `host_id` 就是环。
///
/// 每跳一次查一次库是有意的：这条链**短**（跳板链的现实长度是 1～3），
/// 而把它读进内存再判断会多一份"链的快照"，那是一份新的真相源。
fn assert_no_cycle(
    conn: &Connection,
    host_id: i64,
    jump_id: Option<i64>,
) -> Result<(), StoreError> {
    let mut cursor = jump_id;
    let mut hops = 0;
    while let Some(current) = cursor {
        if current == host_id {
            return Err(StoreError::JumpChain);
        }
        hops += 1;
        if hops > MAX_JUMP_DEPTH {
            return Err(StoreError::JumpChain);
        }
        cursor = jump_of(conn, current)?;
    }
    Ok(())
}

/// 某一行的跳板（外键保证它存在；不存在只可能是库被外力改过）。
fn jump_of(conn: &Connection, id: i64) -> Result<Option<i64>, StoreError> {
    conn.query_row("SELECT jump_id FROM hosts WHERE id = ?1", [id], |row| {
        row.get(0)
    })
    .optional()?
    .ok_or(StoreError::NoSuchRow { pool: POOL, id })
}

/// 按**列名**读一行（`pub(super)`：密钥池要反查"谁在用我这把钥匙"）。
pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Host> {
    Ok(Host {
        id: row.get("id")?,
        name: row.get("name")?,
        host: row.get("host")?,
        port: row.get("port")?,
        user: row.get("user")?,
        auth: read_enum(row.get("auth")?, "auth", Auth::parse)?,
        key_id: row.get("key_id")?,
        jump_id: row.get("jump_id")?,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn every_value_round_trips_through_its_text_form() {
        // 这张表和 DDL 的 `CHECK` 里那份取值是**同一份**：`as_str` 写出来的必须能被
        // `parse` 读回来，否则"写得进、读不出"就是一条静默的数据损坏。
        for auth in Auth::ALL {
            assert_eq!(Auth::parse(auth.as_str()), Some(auth));
        }
    }

    #[test]
    fn an_unknown_value_is_not_a_default() {
        assert_eq!(Auth::parse("cert"), None);
        assert_eq!(Auth::parse("PublicKey"), None, "取值区分大小写");
    }
}
