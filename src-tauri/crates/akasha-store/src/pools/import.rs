//! **把 [`crate::sshconfig`] 解析出来的条目落进 ssh 配置池**（plan 0506）。
//!
//! 解析器是纯函数（一段文本 → 一批条目），这里负责它剩下的一半：**一次事务、要么全成**。
//!
//! ## 为什么先插再挂链（而不是一次插好）
//!
//! `hosts::insert_host` **刻意不做**跳板成环检查 —— 它的论证是"新插入的行此刻还没有被任何
//! 人引用，所以不可能成环"（数学归纳：老数据无环 + 新行无入边 = 仍无环）。
//!
//! ⚠️ 导入是**第一条打破这个前提**的路径：它一次插入多行，而这些行**互相引用**
//! （`A` 的跳板是 `B`，`B` 的跳板是 `A` 完全写得出来）。所以这里分两步：先把每一行按
//! `jump_id = NULL` 插进去，再用 [`hosts::update_host`] 逐条挂链 —— 于是成环检查**仍然只有
//! 一份实现**（那条路径上本来就有的 `assert_no_cycle`），不需要在这里抄第二份。
//!
//! ## 谁会被动、谁不会
//!
//! * **同名已存在**：默认**不动它**（报告里逐条列出），`overwrite` 时才整行替换。
//! * **为跳板补建的条目**（[`Target::provisional`]）：**永不覆盖**。它是"够用就好"的兜底，
//!   不是用户的声明 —— 拿它去盖掉用户手写的同名行，等于让一条 `ProxyJump` 附带的推断
//!   毁掉一份配置。

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::hosts::{self, Auth, Host, NewHost};
use crate::StoreError;
use crate::sshconfig::Target;

/// 落库的结果（报告的另一半）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub created: Vec<Row>,
    pub updated: Vec<Row>,
    pub skipped: Vec<(String, Reason)>,
}

/// 池里的一行（过报告用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: i64,
    pub name: String,
}

/// 没动这一行的原因 —— 两句话对应两个不同的下一步动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// 池里已经有同名的行了（想换掉它就带 `overwrite` 再来一次）。
    Exists,
    /// 这是**为跳板补建**的条目，而池里已有同名行 —— 用的是池里那一行。
    Fallback,
}

/// 一批条目落进池。**一次事务**：任何一条失败（重名、成环、外键）整批回滚。
///
/// `overwrite` 只对"配置里真写了 `Host` 块"的条目有效（见模块文档）。
pub fn import_hosts(
    conn: &Connection,
    targets: &[Target],
    overwrite: bool,
) -> Result<Outcome, StoreError> {
    let tx = conn.unchecked_transaction()?;
    let mut ids: BTreeMap<String, i64> = hosts::hosts(&tx)?
        .into_iter()
        .map(|row| (row.name, row.id))
        .collect();
    let mut outcome = Outcome::default();

    // 第一步：把池里的行数凑齐（新建的一律先不挂跳板）。
    for target in targets {
        match ids.get(&target.name).copied() {
            Some(_) if target.provisional => {
                outcome
                    .skipped
                    .push((target.name.clone(), Reason::Fallback));
            }
            Some(_) if !overwrite => {
                outcome.skipped.push((target.name.clone(), Reason::Exists));
            }
            Some(id) => outcome.updated.push(Row {
                id,
                name: target.name.clone(),
            }),
            None => {
                let id = hosts::insert_host(&tx, &new_row(target))?;
                ids.insert(target.name.clone(), id);
                outcome.created.push(Row {
                    id,
                    name: target.name.clone(),
                });
            }
        }
    }

    // 第二步：挂链。走 `update_host` 是为了那条路径上的成环检查（模块文档）。
    for target in targets {
        if outcome.skipped.iter().any(|(name, _)| name == &target.name) {
            continue;
        }
        let Some(id) = ids.get(&target.name).copied() else {
            continue;
        };
        // 跳板名一定在 `ids` 里：解析器为每一个没写 `Host` 块的跳板名补建过条目。
        let jump_id = target.jump.as_ref().and_then(|name| ids.get(name)).copied();
        hosts::update_host(&tx, &row(target, id, jump_id))?;
    }

    tx.commit()?;
    Ok(outcome)
}

/// 认证方式一律 `publickey` + 不带钥匙。
///
/// 六条里与认证有关的只有 `IdentityFile`，而它的私钥**不导入**（见 `sshconfig` 的模块文档）——
/// 于是这条条目落成"公钥认证、钥匙在 ssh-agent 里"，正是 `hosts` 模块文档里那种**合法状态**。
/// 配置里只有口令认证的情形不用另开一档：`akasha-ssh` 的认证顺序在公钥那几档走不通之后
/// 本来就会请用户输入口令（ADR-0003 D7）。
fn auth() -> Auth {
    Auth::PublicKey
}

fn new_row(target: &Target) -> NewHost {
    NewHost {
        name: target.name.clone(),
        host: target.host.clone(),
        port: target.port,
        user: target.user.clone(),
        auth: auth(),
        key_id: None,
        jump_id: None,
    }
}

fn row(target: &Target, id: i64, jump_id: Option<i64>) -> Host {
    Host {
        id,
        name: target.name.clone(),
        host: target.host.clone(),
        port: target.port,
        user: target.user.clone(),
        auth: auth(),
        key_id: None,
        jump_id,
    }
}
