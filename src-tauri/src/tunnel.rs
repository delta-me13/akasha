//! **隧道实体与三条命令**（plan 0601）—— 端口转发的资源模型落地处。
//!
//! `docs/scope.md` §2.2 / ADR-0003 D6 把这件事定得很死：**一条转发规则是一个独立的
//! `Session`**（不是某个终端会话的附属物），它自己持有一条连接，关它只关它自己。
//! 本模块就是那个实体的登记与驱动：
//!
//! | 事 | 在哪 |
//! |---|---|
//! | 五态与"哪些转移合法" | `akasha_core::TunnelState`（纯逻辑，零 Tauri） |
//! | 实体表与注册表 | [`crate::session::Sessions`] —— **同一张注册表**（D6），不另立一份 |
//! | 连接怎么建 | [`crate::ssh::connect_connection`]（已认证、没有通道的 `SshConnection`） |
//! | 事件与 probe | 本模块（`tunnel_state` / `tunnels`） |
//!
//! ## 本步做到哪为止
//!
//! 建出来的是"**一条已连接的隧道**"：它持有一条真实的 SSH 连接，**但还不转发任何字节**。
//! 三种转发机制分别在 plan 0602（`-L`）/ 0603（`-D`）/ 0604（`-R`），它们都在这条连接上
//! 按需开通道 —— 那正是 ADR-0003 D9 把"连接"与"通道"分开的理由。
//!
//! ⚠️ **`重连中` 在本步不由真实路径产生**：驱动它的重连循环是 plan 0605。状态与那条边
//! 已经存在（`akasha_core::TunnelState` 的用例覆盖了它），但这里没有任何代码会走到它 ——
//! 文档与判据都不得假装它已被验证。
//!
//! ## 停止 = 停止 + 注销
//!
//! `tunnel_stop` 把状态推到 `已停止`（**发出事件**），随后把这一个 `Session` 从注册表
//! 摘掉 —— D5 的"关闭 Session 立刻关闭连接，无宽限期"与"已停止"这一态因此不冲突：
//! 用户看到的是它消失了，而事件序列里留着那一步。残留的半开状态（"已停止但仍占着注册表"）
//! 是 stage 6 的 plan 0606 要处理的那种东西，本步不引入。

use akasha_core::{SessionId, TunnelState, TunnelTransitionError};
use akasha_ssh::SshConnection;
use akasha_store::StoreError;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_specta::Event;

use crate::pools::{ForwardId, HostId};
use crate::session::{IpcError, SessionHandle, Sessions};
use crate::ssh::{Ssh, SshFailureKind, SshIpcError};
use crate::vault::{ConnError, Vault};

/// 一条隧道。
///
/// 字段都是**资源归属**那一类（D6 的"自持有条目"）：规则是谁、连的是哪台、现在什么状态、
/// 已经重试了几次、以及**那条连接**。没有"用户在界面上选中了它"这类信息 ——
/// 后端不编码呈现方式（`AGENTS.md` §3.1）。
pub struct Tunnel {
    id: SessionId,
    /// 池里的转发规则行 id（`forwards.id`）。
    rule_id: i64,
    /// 规则名（托盘与界面要显示"哪一条失败了"）。
    rule_name: String,
    /// 它的 SSH 连接连的那台主机（`forwards.host_id` → `hosts.id`）。
    host_id: HostId,
    state: TunnelState,
    /// 已经重试过几次。手动重试清零（D12），进入 `重连中(n)` 时等于 `n`。
    attempts: u32,
    /// 已认证、没有通道的连接（ADR-0003 D9 的类型）。`None` = 还没连上 / 已经断开。
    connection: Option<SshConnection>,
}

impl Tunnel {
    /// 新实体：**刚登记、还没连接**。
    pub(crate) fn new(id: SessionId, rule_id: i64, rule_name: String, host_id: HostId) -> Self {
        Self {
            id,
            rule_id,
            rule_name,
            host_id,
            state: TunnelState::Connecting,
            attempts: 0,
            connection: None,
        }
    }

    /// 走一步状态机，并把尝试次数跟着状态调整。
    ///
    /// 次数**跟着状态走**而不是各处手改：`重连中(n)` 的 `n` 与"已重试几次"必须是同一个数，
    /// 分开维护就是两份账（`docs/STATUS.md` 的已知问题里那类"两张表分叉"）。
    pub(crate) fn transition(
        &mut self,
        next: TunnelState,
    ) -> Result<TunnelState, TunnelTransitionError> {
        let applied = self.state.advance(next)?;
        self.state = applied;
        self.attempts = match applied {
            TunnelState::Reconnecting { attempt } => attempt,
            // 重试的定义就是"次数清零"（D12）—— 首次连接同样从 0 起。
            TunnelState::Connecting => 0,
            _ => self.attempts,
        };
        Ok(applied)
    }

    pub(crate) fn attach(&mut self, connection: SshConnection) {
        self.connection = Some(connection);
    }

    /// 取走连接（重试 / 停止要在**锁外**显式断开它）。
    pub(crate) fn take_connection(&mut self) -> Option<SshConnection> {
        self.connection.take()
    }

    /// 实体被摘牌时交出连接。
    pub(crate) fn into_connection(self) -> Option<SshConnection> {
        self.connection
    }

    pub(crate) const fn id(&self) -> SessionId {
        self.id
    }

    pub(crate) const fn state(&self) -> TunnelState {
        self.state
    }

    /// `(规则 id, 主机 id)`：重试要照原样再连一次。
    pub(crate) const fn origin(&self) -> (i64, HostId) {
        (self.rule_id, self.host_id)
    }

    /// 给托盘与 probe 的只读投影。
    pub(crate) fn summary(&self, handle: SessionHandle) -> TunnelSummary {
        TunnelSummary {
            handle,
            rule_id: self.rule_id,
            name: self.rule_name.clone(),
            state: self.state,
            attempt: self.attempts,
        }
    }
}

/// 一条隧道的**只读快照**（托盘菜单与 `tunnels` probe 共用同一份，见 `Sessions::tunnel_entries`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelSummary {
    pub handle: SessionHandle,
    pub rule_id: i64,
    pub name: String,
    pub state: TunnelState,
    pub attempt: u32,
}

/// 状态过 IPC 的形状。
///
/// 与 `akasha_core::TunnelState` 分开：`Reconnecting` 的次数在那边是**载荷**，在这边是
/// 事件里的另一个字段（`attempt`）—— 前端因此不必对"每种状态长什么样"分支。
/// 映射写成穷尽 `match`：core 加一个状态时**这里编译不过**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TunnelStateName {
    Connecting,
    Connected,
    Reconnecting,
    Failed,
    Stopped,
}

impl TunnelStateName {
    /// 稳定短名。**与 `TunnelState::as_str` 逐字相同**（同一件事不该有两种说法），
    /// 界面与前端按它分支。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

impl From<TunnelState> for TunnelStateName {
    fn from(state: TunnelState) -> Self {
        match state {
            TunnelState::Connecting => Self::Connecting,
            TunnelState::Connected => Self::Connected,
            TunnelState::Reconnecting { .. } => Self::Reconnecting,
            TunnelState::Failed => Self::Failed,
            TunnelState::Stopped => Self::Stopped,
        }
    }
}

/// **状态变化事件**（ADR-0003 D12 的"状态变化发事件"，事件名见 [`Self::NAME`]）。
///
/// 载荷带 `handle`（按 `SessionId` 路由的落点）而不只是状态：前端与托盘要能回答
/// "是**哪一条**隧道失败了"—— 只报状态的话，多隧道并行时那份信息就丢了。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStateChanged {
    /// 哪条隧道。
    pub handle: SessionHandle,
    /// 变成什么状态了。
    pub state: TunnelStateName,
    /// 重连次数（只有 `reconnecting` 带着它）。
    pub attempt: Option<u32>,
}

// 手写这个 impl 而不是 `#[derive(tauri_specta::Event)]`：与 `SessionEnded` 同一条理由 ——
// 为一个"一个常量 + 其余全默认方法"的 trait 多拉一个 proc-macro 依赖不划算。
impl tauri_specta::Event for TunnelStateChanged {
    const NAME: &'static str = "tunnel_state";
}

/// IPC 边界的隧道错误。变体按**用户的下一步动作**分（同 `VaultError` / `SshIpcError`）。
#[derive(Debug, thiserror::Error, Serialize, specta::Type)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum TunnelError {
    /// 库没解锁。规则在库里，**没有别的来路**。
    #[error("库是锁着的：转发规则在库里（先解锁）")]
    Locked,

    /// 池里没有这一条规则。
    #[error("转发规则池里没有这一条（id {id}）")]
    NoSuchForward { id: ForwardId },

    /// 规则指向的那台主机不在池里。
    #[error("主机池里没有这台主机（id {id}）")]
    NoSuchHost { id: HostId },

    /// 这个句柄不是一条隧道（已经停止 / 从来不存在）。
    #[error("会话 {handle} 不是一条隧道（已停止或未打开）")]
    NotATunnel { handle: SessionHandle },

    /// 连接这条路失败。`kind` 是给界面分辨**警报**用的（同 `SshIpcError`）。
    #[error("{message}")]
    Failed {
        kind: SshFailureKind,
        message: String,
    },

    /// 状态转移被拒。多半是"另一条路已经把它推到终态了"，刷新一下即可。
    #[error("隧道状态转移被拒：{message}")]
    Transition { message: String },

    /// 内部状态不可用。
    #[error("内部状态不可用：{message}")]
    Internal { message: String },
}

impl From<SshIpcError> for TunnelError {
    fn from(err: SshIpcError) -> Self {
        match err {
            SshIpcError::Locked => Self::Locked,
            SshIpcError::NoSuchHost { id } => Self::NoSuchHost { id },
            SshIpcError::Failed { kind, message } => Self::Failed { kind, message },
            SshIpcError::Internal { message } => Self::Internal { message },
        }
    }
}

impl From<IpcError> for TunnelError {
    fn from(err: IpcError) -> Self {
        match err {
            IpcError::NotFound { handle } => Self::NotATunnel { handle },
            IpcError::Tunnel { message } => Self::Transition { message },
            other => Self::Internal {
                message: other.to_string(),
            },
        }
    }
}

/// 一次"打开 / 重试"的结果。
///
/// 为什么不是 `Result<handle, error>`：连接失败时这条隧道**仍然登记着**（状态 `失败`，
/// 可手动重试 —— D12），界面因此必须拿到 `handle` 才说得清"是哪一条失败了"。
/// 把"根本没登记成"（库锁着 / 规则不存在）与"登记了但连不上"混进同一个 `Err`，
/// 前者会退化成一个没人能重试的死胡同。
#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TunnelAttempt {
    /// 这条隧道的句柄（无论连上没有，它都登记着）。
    pub handle: SessionHandle,
    /// 连接失败的原因；`None` = 已连接。
    pub failure: Option<TunnelError>,
}

/// 打开一条隧道：读池里的规则 → 登记 → 建连接 → `已连接`。
///
/// ⚠️ **async**：命令体里有一次会阻塞几秒的握手（最长 `connect_timeout`，跳板链再乘以
/// 跳数）。同步命令跑在处理 IPC 请求的那条线程上，挡住它就等于挡住全部 IPC ——
/// 包括用户回答问题要用的那三条（同 `open_ssh_session`）。
///
/// `Err` 只在**没登记成**时返回（库锁着 / 规则不在池里 / id 装不下）；连不上属于
/// [`TunnelAttempt::failure`]（隧道已在册、可重试）。
#[tauri::command]
#[specta::specta]
pub async fn tunnel_open(
    app: AppHandle,
    forward_id: ForwardId,
) -> Result<TunnelAttempt, TunnelError> {
    let rule = load_rule(&app, forward_id)?;
    let sessions = app.state::<Sessions>().inner().clone();
    let handle = sessions
        .open_tunnel(rule.id, rule.name, rule.host_id)
        .map_err(TunnelError::from)?;
    // 登记本身就是进入「连接中」（见 `Sessions::open_tunnel`）—— 这里只把那条既定事实
    // 通告给前端：连接要花几秒，界面与托盘都该立刻看到"它在连"。
    // ⚠️ 不再走一次状态机：`连接中 → 连接中` 是非法边（同态转移），会被正确拒绝。
    announce(&app, handle, TunnelState::Connecting);
    connect_and_attach(&app, handle, rule.host_id).await
}

/// 手动重试（D12：`失败 / 已停止 → 连接中`，尝试次数清零）。
///
/// `Err` 只在"这个句柄不是一条隧道 / 状态推不动"时返回；**又没连上**属于
/// [`TunnelAttempt::failure`]。
#[tauri::command]
#[specta::specta]
pub async fn tunnel_retry(
    app: AppHandle,
    handle: SessionHandle,
) -> Result<TunnelAttempt, TunnelError> {
    let sessions = app.state::<Sessions>().inner().clone();
    let (_, host_id) = sessions
        .tunnel_origin(handle)
        .ok_or(TunnelError::NotATunnel { handle })?;

    // 上一次那条连接（如果还在）先断开：重试是"重来一次"，不是"再来一条"。
    if let Some(connection) = sessions.take_tunnel_connection(handle) {
        disconnect(&app, connection);
    }
    apply(&app, &sessions, handle, TunnelState::Connecting)?;
    connect_and_attach(&app, handle, host_id).await
}

/// 停止一条隧道：`已停止`（发事件）→ 断开连接 → 从注册表摘掉。
///
/// 摘牌是**幂等**的：重复点击、或这条已经被别的路径收掉时返回 `Ok`，而不是报一个
/// 用户没有下一步动作可做的错。
#[tauri::command]
#[specta::specta]
pub fn tunnel_stop(app: AppHandle, handle: SessionHandle) -> Result<(), TunnelError> {
    let sessions = app.state::<Sessions>().inner().clone();
    match apply(&app, &sessions, handle, TunnelState::Stopped) {
        Ok(_) => {}
        Err(TunnelError::NotATunnel { .. }) => return Ok(()),
        Err(err) => return Err(err),
    }

    let Some(tunnel) = sessions.remove_tunnel(handle).map_err(TunnelError::from)? else {
        return Ok(());
    };
    if let Some(connection) = tunnel.into_connection() {
        disconnect(&app, connection);
    }
    Ok(())
}

/// `tunnels` probe 的快照：**同一份实体表**（`Sessions::tunnel_entries`），不另立一张账。
///
/// 为什么是列表而不是计数：`sessions` probe 那两个数回答"有没有人管得着"，
/// 而这一份要回答"**哪一条**现在是什么状态"—— 那正是 `scope.md` §2.2 说的"失败必须可见"。
pub fn snapshot(sessions: &Sessions) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = sessions
        .tunnel_entries()
        .into_iter()
        .map(|entry| {
            let name = TunnelStateName::from(entry.state);
            serde_json::json!({
                "handle": entry.handle,
                "ruleId": entry.rule_id,
                "name": entry.name,
                "state": name.as_str(),
                // 拿不到就不写这个字段（`docs/logging.md`：值不撒谎）——
                // 这里用 null 而不是 0：0 是一个真出现过的次数（首次连接）。
                "attempt": entry.state.attempt(),
            })
        })
        .collect();
    serde_json::json!(entries)
}

/// 池里那一行规则里，这条隧道要用的三个字段。
struct Rule {
    id: i64,
    name: String,
    host_id: HostId,
}

/// 读池里的一行转发规则。**短借**：读完就把库的锁放掉（连接期间不能持锁，
/// 见 `crate::ssh` 的模块文档）。
fn load_rule(app: &AppHandle, forward_id: ForwardId) -> Result<Rule, TunnelError> {
    let vault = app.state::<Vault>();
    let row = vault
        .with_conn(|conn| akasha_store::pools::forwards::forward(conn, i64::from(forward_id)))
        .map_err(|err| match err {
            ConnError::Locked => TunnelError::Locked,
            ConnError::Store(StoreError::NoSuchRow { .. }) => {
                TunnelError::NoSuchForward { id: forward_id }
            }
            ConnError::Store(other) => TunnelError::Internal {
                message: other.to_string(),
            },
            ConnError::Internal(message) => TunnelError::Internal { message },
        })?;

    let host_id = HostId::try_from(row.host_id).map_err(|_| TunnelError::Internal {
        message: format!(
            "转发规则 {forward_id} 的主机 id 超出可表示范围（{}）",
            row.host_id
        ),
    })?;
    Ok(Rule {
        id: row.id,
        name: row.name,
        host_id,
    })
}

/// 建立连接并挂到实体上；失败则把状态推到 `失败`（**并发出事件**），把原因放进结果里。
async fn connect_and_attach(
    app: &AppHandle,
    handle: SessionHandle,
    host_id: HostId,
) -> Result<TunnelAttempt, TunnelError> {
    let outcome = {
        let ssh = app.state::<Ssh>();
        let vault = app.state::<Vault>();
        crate::ssh::connect_connection(&ssh, &vault, host_id).await
    };

    let sessions = app.state::<Sessions>().inner().clone();
    match outcome {
        Ok(connection) => {
            sessions
                .attach_tunnel_connection(handle, connection)
                .map_err(TunnelError::from)?;
            apply(app, &sessions, handle, TunnelState::Connected)?;
            Ok(TunnelAttempt {
                handle,
                failure: None,
            })
        }
        Err(err) => {
            // 失败**也要落到状态里并发出事件**：`scope.md` §2.2 的"失败必须可见"
            // 靠的就是这一条 —— 用户不会去读 probe，托盘与界面读的是状态。
            let _ = apply(app, &sessions, handle, TunnelState::Failed);
            Ok(TunnelAttempt {
                handle,
                failure: Some(TunnelError::from(err)),
            })
        }
    }
}

/// 走一步状态机，并在**锁外**发事件 + 记日志。返回实际生效的状态。
fn apply(
    app: &AppHandle,
    sessions: &Sessions,
    handle: SessionHandle,
    next: TunnelState,
) -> Result<TunnelState, TunnelError> {
    let applied = sessions
        .set_tunnel_state(handle, next)
        .map_err(TunnelError::from)?;
    announce(app, handle, applied);
    Ok(applied)
}

/// 一条状态变化的两条出口：前端事件（`tunnel_state`）与一条日志。
///
/// 合成一处是为了**不可能漏掉其中之一**：只发事件会在排查时没有线索，
/// 只记日志则前端永远不知道（D12 的原话是"界面未实现也先发"）。
fn announce(app: &AppHandle, handle: SessionHandle, state: TunnelState) {
    if let Some(attempt) = state.attempt() {
        tracing::info!(
            handle,
            state = state.as_str(),
            attempt,
            "tunnel state changed"
        );
    } else {
        tracing::info!(handle, state = state.as_str(), "tunnel state changed");
    }

    let event = TunnelStateChanged {
        handle,
        state: TunnelStateName::from(state),
        attempt: state.attempt(),
    };
    if let Err(err) = app.emit(TunnelStateChanged::NAME, event) {
        tracing::warn!(event = TunnelStateChanged::NAME, %err, "event emit failed");
    }
}

/// 显式断开一条连接（`Handle` 一 drop 也会结束它，但那一次没有道别）。
///
/// runtime 起不来时只剩"丢掉它"一条路 —— 收尾路径上不报错（`Ssh::runtime_handle` 的理由）。
fn disconnect(app: &AppHandle, connection: SshConnection) {
    match app.state::<Ssh>().runtime_handle() {
        Some(runtime) => {
            runtime.spawn(connection.disconnect());
        }
        None => {
            tracing::warn!(
                reason = "ssh-runtime-unavailable",
                "tunnel disconnect skipped"
            );
        }
    }
}
