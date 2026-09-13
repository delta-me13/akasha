//! **池的只读读取**（plan 0504）—— 界面凭什么把一台主机交给 SSH 那条路。
//!
//! 阶段 4 落的是四套池的 **CRUD**（`akasha-store` 的 `pools::*`，plan 0403），
//! 而它一条命令都没接进 IPC：那时没有任何界面要读它们。plan 0504 需要第一条 ——
//! "从界面选一个主机"要求界面**看得见**池里有什么。
//!
//! ## 为什么只做"读"，以及为什么这样不算越界
//!
//! 增删改是**用户动作**，各有各的判据（重名怎么办、跳板成环怎么提示、删掉被引用的行怎么
//! 解释）—— 那些是阶段 4 之后仍未规划的界面工作，不属于本 plan。而"列出有哪些主机"
//! 没有可选项：它就是把库里已有的东西显示出来，**不改任何状态**。
//!
//! ## 过 IPC 的形状是有意的
//!
//! [`HostEntry`] 里**没有** `private_pem`（密钥池的秘密），也没有库里那一列 `auth` 的
//! 原始文本 —— 认证方式过 IPC 是一个**枚举**，因为它是前端要分支的东西，而"字符串里的取值"
//! 这种东西改起来没有任何人会红。`jump_id` 在 plan 0505 补上了：它也是库里的一列，
//! 而"这一台经谁连"是选主机的人有权知道的事。

use akasha_store::pools::hosts::Auth;
use serde::Serialize;
use tauri::State;

use crate::vault::{Vault, VaultError};

/// 池行的**过 IPC 表示**。
///
/// 为什么不直接用 `i64`：生成器**拒绝**把 64 位整数导出成 TS（BigInt 精度问题，坑 #32），
/// 而打开 shell 侧那个"危险地当 `number` 用"的开关，会让将来**每一个** `i64` 字段都悄悄
/// 失去保护。这里照 `SessionHandle` 的做法用 `u32` 代理，并**checked** 转换 ——
/// 截断会把"要连的那台主机"变成**另一台**，而不是报错。
pub type HostId = u32;

/// 行 id → 过 IPC 的表示。装不下就报错，**绝不截断**。
fn host_id(id: i64) -> Result<HostId, VaultError> {
    HostId::try_from(id).map_err(|_| VaultError::Unusable {
        message: format!("主机池的行 id 超出可表示范围（{id}）"),
    })
}

/// 界面看得见的一台主机。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HostEntry {
    /// 池里的行 id（`open_ssh_session` 要的就是它）。
    pub id: HostId,
    /// 用户给这台主机起的名字（池里唯一）。标签页标题用它。
    pub name: String,
    /// 主机名或 IP。
    pub host: String,
    pub port: u16,
    pub user: String,
    /// 认证方式（池里的取值，见 [`AuthMethod`]）。
    pub auth: AuthMethod,
    /// 配了哪把钥匙（`keys.id`）。只有 `publickey` 时可能非空 —— 而它也可能是空的
    /// （走 ssh-agent 的钥匙不在我们的池里）。
    pub key_id: Option<HostId>,
    /// 这台主机**经哪台连**（`hosts.jump_id`，plan 0505）。
    ///
    /// 出现在这里是因为它已经是库里的一列，而"从界面选主机"的人有权知道这一次点下去
    /// 会**经过谁** —— 不然一条跳板链在界面上没有任何痕迹，连不上时也无从判断是目标的问题
    /// 还是跳板的问题。链本身（跳板还有跳板）由连接那条路自己走，界面只看一跳。
    pub jump_id: Option<HostId>,
}

/// 认证方式过 IPC 的形状。
///
/// 与 `akasha_store::pools::hosts::Auth` 分开：存储 crate **不依赖 specta**
/// （那是 app 钉住版本的东西），所以这里做一次显式映射 —— 而映射写成穷尽 `match`，
/// 池里加一种认证方式时**这里编译不过**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AuthMethod {
    Password,
    PublicKey,
    Agent,
}

impl From<Auth> for AuthMethod {
    fn from(auth: Auth) -> Self {
        match auth {
            Auth::Password => Self::Password,
            Auth::PublicKey => Self::PublicKey,
            Auth::Agent => Self::Agent,
        }
    }
}

/// 库里主机池的全部行（按名字排序 —— 顺序确定，界面才不会每次刷新换一个样）。
///
/// 库锁着 → [`VaultError::Locked`]：池在库里，没有别的来路。
#[tauri::command]
#[specta::specta]
pub fn vault_hosts(vault: State<'_, Vault>) -> Result<Vec<HostEntry>, VaultError> {
    let rows = vault
        .with_conn(akasha_store::pools::hosts::hosts)
        .map_err(VaultError::from_conn)?;
    rows.into_iter()
        .map(|row| {
            Ok(HostEntry {
                id: host_id(row.id)?,
                name: row.name,
                host: row.host,
                port: row.port,
                user: row.user,
                auth: AuthMethod::from(row.auth),
                key_id: row.key_id.map(host_id).transpose()?,
                jump_id: row.jump_id.map(host_id).transpose()?,
            })
        })
        .collect()
}
