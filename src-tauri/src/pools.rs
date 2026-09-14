//! **池与界面之间的接口** —— 阶段 4 落的是四套池的 CRUD（`akasha-store` 的 `pools::*`，
//! plan 0403），而它一条命令都没接进 IPC：那时没有任何界面要读它们。
//!
//! | 命令 | 哪来的 | 为什么是它 |
//! |---|---|---|
//! | [`vault_hosts`]（plan 0504） | "从界面选一个主机"要求界面**看得见**池里有什么 | 纯读，不改任何状态 |
//! | [`import_ssh_config`]（plan 0506） | 用户的机器上早就有 `~/.ssh/config` | 唯一的**写**路径，而它写什么是**文件说了算**（不是界面上一格一格填） |
//!
//! ## 为什么到现在也只有这两条
//!
//! 增删改是**用户动作**，各有各的判据（重名怎么办、跳板成环怎么提示、删掉被引用的行怎么
//! 解释）—— 那些是仍未规划的界面工作。而导入不一样：它没有"填什么"的自由度，只有"照不照
//! 这份文件做"这一个问题，而那个问题的答案已经写在 `akasha-store::sshconfig` 里了。
//!
//! ## 过 IPC 的形状是有意的
//!
//! [`HostEntry`] 里**没有** `private_pem`（密钥池的秘密），也没有库里那一列 `auth` 的
//! 原始文本 —— 认证方式过 IPC 是一个**枚举**，因为它是前端要分支的东西，而"字符串里的取值"
//! 这种东西改起来没有任何人会红。`jump_id` 在 plan 0505 补上了：它也是库里的一列，
//! 而"这一台经谁连"是选主机的人有权知道的事。

use akasha_store::pools::forwards::Direction;
use akasha_store::pools::hosts::Auth;
use akasha_store::pools::import::Reason;
use serde::Serialize;
use tauri::State;

use crate::vault::{ConnError, Vault, VaultError};

/// 池行的**过 IPC 表示**。
///
/// 为什么不直接用 `i64`：生成器**拒绝**把 64 位整数导出成 TS（BigInt 精度问题，问题 #32），
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

// ── 从 `~/.ssh/config` 导入（plan 0506） ────────────────────────────────────

/// 导入报告（过 IPC 的形状）。
///
/// 它同时回答三个问题，缺一个用户就没法相信这次导入：
///
/// | 字段 | 回答 |
/// |---|---|
/// | [`Self::created`] / [`Self::updated`] / [`Self::skipped`] | **池里变了吗** |
/// | [`Self::ignored`] | **配置里哪些话我们没照做**（受限子集的边界就落在这里） |
/// | [`Self::notes`] | **我们替你补了什么、跳过了什么**（通配块、补建的跳板条目……） |
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// **真读的那个文件**（`path` 没给时是这样推出来的）。
    pub path: String,
    pub created: Vec<ImportedHost>,
    pub updated: Vec<ImportedHost>,
    pub skipped: Vec<SkippedHost>,
    /// 没生效的指令：行号 + 关键字 + 说法。
    pub ignored: Vec<ConfigFinding>,
    pub notes: Vec<String>,
}

/// 池里新增 / 被替换的一行。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportedHost {
    pub id: HostId,
    pub name: String,
}

/// 看见了、但**没动**的一行。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SkippedHost {
    pub name: String,
    pub reason: SkipReason,
}

/// 没动这一行的原因。**两个取值对应两个不同的下一步动作**（对用户说的是两句话）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// 池里已经有同名的行了（想换掉它就再导一次并选择覆盖）。
    Existing,
    /// 这一条是**为跳板补建**的，而池里已有同名行 —— 用的是池里那一行。
    Fallback,
}

impl From<Reason> for SkipReason {
    fn from(reason: Reason) -> Self {
        match reason {
            Reason::Exists => Self::Existing,
            Reason::Fallback => Self::Fallback,
        }
    }
}

/// 报告里的一条：行号 + 关键字 + 说法。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFinding {
    pub line: u32,
    pub keyword: String,
    pub message: String,
}

/// 导入失败。变体按**用户的下一步动作**分（与 `VaultError` / `SshIpcError` 同一原则）。
#[derive(Debug, thiserror::Error, Serialize, specta::Type)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum ImportError {
    /// 库没解锁。导入要**写**进 ssh 配置池，所以解锁是硬前提（不像列主机那样只是读不到）。
    #[error("库是锁着的：导入要把条目写进 ssh 配置池，先解锁")]
    Locked,

    /// 文件读不到（不存在 / 权限 / 不是 UTF-8）。**不是"导入了 0 台"**：这两件事要分得开，
    /// 否则用户会去池子里找一台本来就还在原地的机器。
    #[error("读不到 `{path}`：{message}")]
    Unreadable { path: String, message: String },

    /// 这份配置**整份**不能照着做（`Match` / `Include` / 不认识的指令……）。
    /// 一行都没写 —— 这是"宁可明确报错，也不静默误解析"那条判据的落点。
    ///
    /// `message` 里带条数（`thiserror` 的格式串只认字段，所以那句话在命令里拼好）。
    #[error("{message}")]
    Refused {
        path: String,
        message: String,
        problems: Vec<ConfigFinding>,
    },

    /// 其余（写库失败、取不到本机用户名……）。
    #[error("{message}")]
    Failed { message: String },
}

/// 把一份 `~/.ssh/config`（或 `path` 指定的文件）导入 ssh 配置池。
///
/// `path` 为 `None` → `~/.ssh/config`（家目录取不到时**明确报错**，不猜一个路径去读）。
/// `overwrite` = `false`（默认）：同名已存在就**不动它**，只在报告里列出来。
///
/// 支持哪些指令、不支持时是报错还是警告 —— 全在 `akasha_store::sshconfig` 里，这个命令
/// 只负责"读文件、问用户名、写库、把结果拼成报告"。
#[tauri::command]
#[specta::specta]
pub fn import_ssh_config(
    vault: State<'_, Vault>,
    path: Option<String>,
    overwrite: bool,
) -> Result<ImportReport, ImportError> {
    let file = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => akasha_ssh::user_ssh_config_file().ok_or_else(|| ImportError::Unreadable {
            path: "~/.ssh/config".to_owned(),
            message: "取不到家目录（`HOME` / `USERPROFILE` 都没有），不给路径就不知道该读哪个文件"
                .to_owned(),
        })?,
    };
    // 显示用的路径：报告里要写清**真读了哪一个**（给相对路径时这一点尤其重要）。
    let shown = file.to_string_lossy().into_owned();

    let text = std::fs::read_to_string(&file).map_err(|err| ImportError::Unreadable {
        path: shown.clone(),
        message: err.to_string(),
    })?;

    // 没写 `User` 的条目用本机用户名（OpenSSH 的默认值就是当前用户）。
    let user = local_user().ok_or_else(|| ImportError::Failed {
        message: "取不到本机用户名（`USER` / `USERNAME` 都没有），没写 `User` 的条目不知道该填谁"
            .to_owned(),
    })?;

    let parsed = akasha_store::sshconfig::parse(&text, &user).map_err(|problems| {
        let problems: Vec<ConfigFinding> = problems.into_iter().map(finding).collect();
        ImportError::Refused {
            path: shown.clone(),
            message: format!(
                "`{shown}` 里有 {} 处不能照着做，所以一行都没导入（每处一行，见 problems）",
                problems.len()
            ),
            problems,
        }
    })?;

    let outcome = vault
        .with_conn(|conn| {
            akasha_store::pools::import::import_hosts(conn, &parsed.targets, overwrite)
        })
        .map_err(|err| match err {
            ConnError::Locked => ImportError::Locked,
            ConnError::Store(err) => ImportError::Failed {
                message: err.to_string(),
            },
            ConnError::Internal(message) => ImportError::Failed { message },
        })?;

    tracing::info!(
        path = shown.as_str(),
        created = outcome.created.len(),
        updated = outcome.updated.len(),
        skipped = outcome.skipped.len(),
        ignored = parsed.ignored.len(),
        "ssh config imported"
    );

    let host = |row: akasha_store::pools::import::Row| {
        host_id(row.id)
            .map(|id| ImportedHost { id, name: row.name })
            .map_err(|err| ImportError::Failed {
                message: err.to_string(),
            })
    };
    Ok(ImportReport {
        created: outcome
            .created
            .into_iter()
            .map(host)
            .collect::<Result<_, _>>()?,
        updated: outcome
            .updated
            .into_iter()
            .map(host)
            .collect::<Result<_, _>>()?,
        skipped: outcome
            .skipped
            .into_iter()
            .map(|(name, reason)| SkippedHost {
                name,
                reason: reason.into(),
            })
            .collect(),
        ignored: parsed.ignored.into_iter().map(finding).collect(),
        notes: parsed.notes,
        path: shown,
    })
}

/// `akasha-store` 的说法 → 过 IPC 的形状。
fn finding(finding: akasha_store::sshconfig::Finding) -> ConfigFinding {
    ConfigFinding {
        line: u32::try_from(finding.line).unwrap_or(u32::MAX),
        keyword: finding.keyword,
        message: finding.message,
    }
}

/// 本机用户名（OpenSSH 在没写 `User` 时用的那个）。
fn local_user() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|user| !user.is_empty())
}

// ── 转发规则池的只读读取（plan 0601） ─────────────────────────────────────────

/// 转发规则行的过 IPC 表示。理由同 [`HostId`]：`i64` 不过 IPC，截断绝不允许。
pub type ForwardId = u32;

/// 行 id → 过 IPC 的表示。装不下就报错，**绝不截断**。
fn forward_id(id: i64) -> Result<ForwardId, VaultError> {
    ForwardId::try_from(id).map_err(|_| VaultError::Unusable {
        message: format!("转发规则池的行 id 超出可表示范围（{id}）"),
    })
}

/// 转发方向过 IPC 的形状（与 [`AuthMethod`] 同一个理由：存储 crate 不依赖 specta）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ForwardDirection {
    /// `-L`：本地绑定，转发到目标。
    Local,
    /// `-R`：远端绑定，转发回本地侧。
    Remote,
    /// `-D`：本地起一个 SOCKS5，目标由客户端给。
    Dynamic,
}

impl From<Direction> for ForwardDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Local => Self::Local,
            Direction::Remote => Self::Remote,
            Direction::Dynamic => Self::Dynamic,
        }
    }
}

impl ForwardDirection {
    /// 稳定短名。**与上面 `rename_all = "camelCase"` 生成的取值逐字相同** ——
    /// 错误消息里说的词与前端 `detail` 里拿到的词因此是同一个（同一件事不该有两种说法）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Dynamic => "dynamic",
        }
    }
}

impl std::fmt::Display for ForwardDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 界面看得见的一条转发规则。
///
/// `target_host` / `target_port` 是 `Option`：`dynamic`（SOCKS5）**没有目标** ——
/// 那是库里 `CHECK` 拦着的不变量，这里如实照搬，而不是填一个看起来像真的空串。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ForwardEntry {
    /// 池里的行 id（`tunnel_open` 要的就是它）。
    pub id: ForwardId,
    pub name: String,
    pub direction: ForwardDirection,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
    /// 这条规则属于哪台主机（`hosts.id`）—— 隧道**连的就是它**。
    pub host_id: HostId,
    /// 会话建立时是否自动起这条转发（plan 0601 只读取，不据此自动开）。
    pub autostart: bool,
}

/// 库里转发规则池的全部行（按名字排序 —— 顺序确定，界面才不会每次刷新换一个样）。
///
/// 库锁着 → [`VaultError::Locked`]：规则在库里，没有别的来路。
#[tauri::command]
#[specta::specta]
pub fn vault_forwards(vault: State<'_, Vault>) -> Result<Vec<ForwardEntry>, VaultError> {
    let rows = vault
        .with_conn(akasha_store::pools::forwards::forwards)
        .map_err(VaultError::from_conn)?;
    rows.into_iter()
        .map(|row| {
            Ok(ForwardEntry {
                id: forward_id(row.id)?,
                name: row.name,
                direction: ForwardDirection::from(row.direction),
                bind_host: row.bind_host,
                bind_port: row.bind_port,
                target_host: row.target_host,
                target_port: row.target_port,
                host_id: host_id(row.host_id)?,
                autostart: row.autostart,
            })
        })
        .collect()
}
