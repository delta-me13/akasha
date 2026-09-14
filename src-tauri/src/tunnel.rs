//! **隧道实体、三条命令与本地转发**（plan 0601 / 0602）—— 端口转发的资源模型落地处。
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
//! | 转发怎么做 | `akasha_ssh::relay`（本地监听 + 每条入站连接一条 `direct_tcpip` 通道） |
//! | `-D` 的目标从哪来 | `akasha_ssh::socks5`（无认证的 SOCKS5 服务端，plan 0603） |
//! | 事件与 probe | 本模块（`tunnel_state` / `tunnels`） |
//!
//! ## 一条"已连接"的隧道现在意味着什么
//!
//! 从 plan 0602 起，`已连接` 意味着两件事同时成立：**本地端口在监听**，
//! 且**那条 SSH 连接活着**。两者合成一件事 —— 转发任务持有连接，端口与被转发的字节
//! 都只属于它（`akasha_ssh::relay` 的模块文档写了收尾的两条路径）。
//!
//! ## 三个方向差在哪
//!
//! `local`（plan 0602）与 `dynamic`（plan 0603，SOCKS5）**只差目标从哪来**：前者写在规则里
//! （`Ingress::Fixed`），后者由客户端在握手里逐条说（`Ingress::Socks5`）。这一处差别用
//! `Ingress` 表达，绑定、连接、停止、probe 这些路径**两者共用**。
//!
//! `remote`（plan 0604）是**另一套机制**（ADR-0003 D10）：端口开在**服务端**，
//! 通道由服务端发起。所以它没有"先绑本机端口"这一步（那个端口在服务端，
//! 要先有连接才能请求），而 `已连接` 的含义两边一样：**那一侧端口在听 + 连接活着**。
//! 两档在这里的分岔只有一处：`Rule::prepare` 给出的 [`Prepared`]。
//!
//! ⚠️ **`-D` 只允许绑回环地址**：SOCKS5 这一侧无认证，绑到 `0.0.0.0` 等于把"经这台跳板机
//! 访问远端网络"的能力交给同网段的所有人。拒绝发生在**绑定之前**（`SshError::NotLoopback`），
//! 所以那样一条监听根本建不出来。
//!
//! ⚠️ **`-R` 的绑定地址不做这种检查**：那个端口开在服务端，能不能开在非回环地址上是
//! **它的**策略（`sshd` 的 `GatewayPorts` 默认只允许回环）。规则里写什么就请求什么。
//!
//! ⚠️ **`重连中` 在本步仍不由真实路径产生**：驱动它的重连循环是 plan 0605。状态与那条边
//! 已经存在（`akasha_core::TunnelState` 的用例覆盖了它），但没有代码会走到它 ——
//! 文档与判据都不得假装它已被验证。
//!
//! ## 停止 = 停止 + 注销
//!
//! `tunnel_stop` 把状态推到 `已停止`（**发出事件**），随后把这一个 `Session` 从注册表
//! 摘掉 —— D5 的"关闭 Session 立刻关闭连接，无宽限期"与"已停止"这一态因此不冲突：
//! 用户看到的是它消失了，而事件序列里留着那一步。残留的半开状态（"已停止但仍占着注册表"）
//! 是 stage 6 的 plan 0606 要处理的那种东西，本步不引入。
//!
//! ⚠️ **停止是异步收尾**：摘掉实体之后调 [`LocalForward::shutdown`] —— 它发完信号即返回，
//! 停止监听、收掉在途连接与断开那条 SSH 连接都在 runtime 上做（同 D5 的"立刻"，
//! 只是"立刻"发生在另一个线程上；要观察结果的地方看对端的连接计数，见 E2E）。

use akasha_core::{SessionId, TunnelState, TunnelTransitionError};
use akasha_ssh::{ForwardTarget, Ingress, LocalForward, LocalListener, RemoteForward, SshError};
use akasha_store::StoreError;
use akasha_store::pools::forwards::Direction;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_specta::Event;

use crate::pools::{ForwardDirection, ForwardId, HostId};
use crate::session::{IpcError, SessionHandle, Sessions};
use crate::ssh::{Ssh, SshFailureKind, SshIpcError};
use crate::vault::{ConnError, Vault};

/// 一条隧道。
///
/// 字段都是**资源归属**那一类（D6 的"自持有条目"）：规则是谁、连的是哪台、现在什么状态、
/// 已经重试了几次、以及**那条转发**（它持有连接，见 `akasha_ssh::relay`）。
/// 没有"用户在界面上选中了它"这类信息 —— 后端不编码呈现方式（`AGENTS.md` §3.1）。
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
    /// 那一侧的监听 + 那条连接（类型见 [`ActiveForward`]）。`None` = 还没连上 / 已经断开。
    forward: Option<ActiveForward>,
}

/// 一条**已经起来的**转发：本机监听（`-L` / `-D`）或远端监听（`-R`）。
///
/// 两者由同一个实体持有，是因为它们对隧道而言是同一件事："已连接"= **那一侧端口在听**
/// 且**那条连接活着**。停止也走同一条路（各自的 `shutdown` 都是"发完信号即返回"），
/// 差别只在 `bound` 从哪一侧读。
pub enum ActiveForward {
    Local(LocalForward),
    Remote(RemoteForward),
}

impl ActiveForward {
    /// 实际监听地址。
    ///
    /// `-R` 的那一侧在**服务端**（地址按服务端解释），且 `port = 0` 时这里是它挑的那个
    /// 而不是规则里写的 0 —— 也就是说这个字段无论哪一档都只说**真的在听的那个地址**。
    fn bound(&self) -> String {
        match self {
            Self::Local(forward) => forward.bound().to_string(),
            Self::Remote(forward) => forward.bound_display(),
        }
    }

    /// 停止：发完信号即返回（收尾在 runtime 上做，见各自的文档）。
    fn shutdown(self) {
        match self {
            Self::Local(forward) => forward.shutdown(),
            Self::Remote(forward) => forward.shutdown(),
        }
    }
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
            forward: None,
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

    /// 挂上刚起来的转发（那一侧的监听 + 那条连接）。
    pub(crate) fn attach(&mut self, forward: ActiveForward) {
        self.forward = Some(forward);
    }

    /// 取走转发（重试 / 停止要在**锁外**收掉它：停监听 + 断连接）。
    pub(crate) fn take_forward(&mut self) -> Option<ActiveForward> {
        self.forward.take()
    }

    /// 实体被注销时交出转发（调用方在锁外 `shutdown`）。
    pub(crate) fn into_forward(self) -> Option<ActiveForward> {
        self.forward
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
            // 实际监听地址 —— "有没有端口在听"是这条功能的唯一对外事实，
            // 它只能从转发那里读（没有它，界面与 probe 就只能说"已连接"而说不清连到哪一步）。
            bind: self.forward.as_ref().map(ActiveForward::bound),
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
    /// 实际监听地址（`127.0.0.1:46010`）。`None` = 还没有端口（连接中 / 失败 / 已停止）。
    ///
    /// `-R` 的规则报的是**服务端**那一侧的监听地址（地址按服务端解释）。
    pub bind: Option<String>,
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

    /// 本地端口没拿到：被占用、无权限、绑定地址不可用。
    ///
    /// ⚠️ 它在一类失败里出现得最多（端口被占用），而且**发生在握手之前** ——
    /// 用户不必先答完凭据才被告知端口没拿到。
    #[error("本地监听 {address} 绑定失败：{message}")]
    Bind { address: String, message: String },

    /// 远端监听没拿到：服务端那个端口被它自己占着，或它不允许远端转发。
    ///
    /// 与 [`Self::Bind`] 分开：那一条说的是"**本机**的端口没拿到"，用户腾一个端口就好；
    /// 这一条要动的地方在**服务端** —— 在本机上做什么都没用。
    #[error("远端监听 {address} 没拿到：{message}")]
    RemoteBind { address: String, message: String },

    /// 动态转发（SOCKS5）的绑定地址不是回环地址。
    ///
    /// 与 [`Self::Bind`] 分开：那一条是"这个端口没拿到"，换个端口就好；这一条是
    /// "这个地址**不许**绑" —— 换端口没有用，要改的是绑定的网卡范围。
    /// 判据与理由见 `akasha_ssh::socks5`（这一侧无认证）。
    #[error("SOCKS5 监听不能绑到 {address}：这一侧无认证，只允许绑回环地址")]
    NotLoopback { address: String },

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

impl TunnelError {
    /// 绑定那一步的两档说法（`akasha-ssh` 的 [`SshError::Listen`] 与
    /// [`SshError::NotLoopback`]）。
    ///
    /// 绑定失败要单独一档：用户对它的下一步动作是"腾出端口 / 换端口 / 换绑定地址"，
    /// 与"连接失败"（查网络与远端）完全不同 —— 压进 `Failed` 会让界面把两件事说成一件。
    /// 「地址不许绑」再单列一档：那一条换端口没有用。
    fn from_bind(err: SshError) -> Self {
        match err {
            SshError::Listen { address, reason } => Self::Bind {
                address,
                message: reason,
            },
            SshError::NotLoopback { address } => Self::NotLoopback { address },
            other => Self::Internal {
                message: other.to_string(),
            },
        }
    }

    /// 远端监听那一步的说法（`akasha-ssh` 的 [`SshError::RemoteListen`]）。
    ///
    /// 与 [`Self::from_bind`] 分开：一个要动本机，一个要动服务端。
    fn from_remote(err: SshError) -> Self {
        match err {
            SshError::RemoteListen { address, reason } => Self::RemoteBind {
                address,
                message: reason,
            },
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

/// 打开一条隧道：读池里的规则 → **绑定本地端口** → 登记 → 建连接 → 起转发 → `已连接`。
///
/// ⚠️ **async**：命令体里有一次会阻塞几秒的握手（最长 `connect_timeout`，跳板链再乘以
/// 跳数）。同步命令在处理 IPC 请求的那条线程上，挡住它就等于挡住全部 IPC ——
/// 包括用户回答问题要用的那三条（同 `open_ssh_session`）。
///
/// 顺序是刻意的（plan 0602 的第一条）：**先绑定、后连接**。端口被占用是本类功能最常见的
/// 一类失败，用户此时还不该回答任何凭据询问 —— 拿不到端口就先报错，握手与提问都不发生。
///
/// `Err` 只在**没登记成**时返回（库锁着 / 规则不在池里 / 方向不是 `-L` / **端口没拿到**）；
/// 连不上属于 [`TunnelAttempt::failure`]（隧道已在册、可重试）。
#[tauri::command]
#[specta::specta]
pub async fn tunnel_open(
    app: AppHandle,
    forward_id: ForwardId,
) -> Result<TunnelAttempt, TunnelError> {
    let rule = load_rule(&app, forward_id)?;
    // 方向与目标先校验，**能提前做的那一步也在这里做掉**（`-L` / `-D` 的绑定必须在
    // 握手之前，plan 0602）：拿不到端口就不该先登记一个句柄，更不该先问凭据。
    let prepared = rule.prepare(&app).await?;

    let sessions = app.state::<Sessions>().inner().clone();
    let handle = sessions
        .open_tunnel(rule.id, rule.name.clone(), rule.host_id)
        .map_err(TunnelError::from)?;
    // 登记本身就是进入「连接中」（见 `Sessions::open_tunnel`）—— 这里只把那条既定事实
    // 通告给前端：连接要花几秒，界面与托盘都该立刻看到"它在连"。
    // ⚠️ 不再走一次状态机：`连接中 → 连接中` 是非法边（同态转移），会被正确拒绝。
    announce(&app, handle, TunnelState::Connecting);
    drive(&app, handle, &rule, prepared).await
}

/// 手动重试（D12：`失败 / 已停止 → 连接中`，尝试次数清零）。
///
/// 规则**重新读一遍**：端口与目标可能在上一次失败之后被改过，而重试的用户意图正是
/// "按现在的配置再来一次"。旧的那条转发（如果还在）先收掉 —— 重试是"重来一次"，
/// 不是"再来一条"。
///
/// `Err` 只在"这个句柄不是一条隧道 / 方向不对 / 端口没拿到 / 状态推不动"时返回；
/// **又没连上**属于 [`TunnelAttempt::failure`]。
#[tauri::command]
#[specta::specta]
pub async fn tunnel_retry(
    app: AppHandle,
    handle: SessionHandle,
) -> Result<TunnelAttempt, TunnelError> {
    let sessions = app.state::<Sessions>().inner().clone();
    let (rule_id, _) = sessions
        .tunnel_origin(handle)
        .ok_or(TunnelError::NotATunnel { handle })?;

    if let Some(forward) = sessions.take_tunnel_forward(handle) {
        forward.shutdown();
    }

    let rule_id = ForwardId::try_from(rule_id).map_err(|_| TunnelError::Internal {
        message: format!("转发规则 id 超出可表示范围（{rule_id}）"),
    })?;
    let rule = load_rule(&app, rule_id)?;
    // 与 `tunnel_open` 同一顺序：先校验方向与目标、先把能提前做的绑定做掉，
    // 再把状态推到 `连接中`（绑定失败时它留在原状态，用户看到的是那条错误，不是一个空转的
    // `连接中`）。
    let prepared = rule.prepare(&app).await?;

    apply(&app, &sessions, handle, TunnelState::Connecting)?;
    drive(&app, handle, &rule, prepared).await
}

/// 停止一条隧道：`已停止`（发事件）→ 收掉转发（停止监听 + 断开连接）→ 从注册表摘掉。
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
    if let Some(forward) = tunnel.into_forward() {
        // 发完信号即返回：收尾（停监听、收在途连接、断开连接）在 runtime 上做。
        forward.shutdown();
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
                // 实际监听地址。没有端口时是 null（不是空串）：空串看着像"绑在空地址上"。
                "bind": entry.bind,
            })
        })
        .collect();
    serde_json::json!(entries)
}

/// 池里那一行规则里，这条隧道要用的字段。
struct Rule {
    id: i64,
    name: String,
    host_id: HostId,
    direction: Direction,
    bind_host: String,
    bind_port: u16,
    target_host: Option<String>,
    target_port: Option<u16>,
}

/// 一条规则被翻译成"要做什么"，连同**已经提前做掉的那一步**。
///
/// `-L` / `-D` 能提前做的是绑定本机端口 —— 那一步必须在握手之前（plan 0602）：
/// 端口被占用是本类功能最常见的一类失败，用户不该先答完两轮凭据才被告知。
/// `-R` **没有**能提前做的事：那个端口在服务端，要先有连接才能请求。
/// 这是方向本身带来的差别，不是取舍。
enum Prepared {
    /// 本机监听已经拿到（`LocalListener` 里带着"入站连接怎么处理"，还没起转发任务）。
    Local(LocalListener),
    /// 请服务端监听：`bind` 是**服务端**那一侧的地址，`target` 是**本机**服务。
    Remote {
        bind: ForwardTarget,
        target: ForwardTarget,
    },
}

impl Rule {
    /// 这条规则里写的目标。
    ///
    /// `local` 与 `remote` 都有目标（库的 `CHECK` 保证），`dynamic` 没有 —— 调用点只在
    /// 前两种方向上用它。缺了就说明那一行数据不合不变量：**报出来而不是 panic**，
    /// 一行坏数据不该带走整个 app。
    fn target(&self) -> Result<ForwardTarget, TunnelError> {
        match (&self.target_host, self.target_port) {
            (Some(host), Some(port)) => Ok(ForwardTarget::new(host.clone(), port)),
            _ => Err(TunnelError::Internal {
                message: format!(
                    "规则 {}（{}）没有目标地址",
                    self.id,
                    ForwardDirection::from(self.direction)
                ),
            }),
        }
    }

    /// 把规则翻成"要做什么"，并把能提前做的那一步做掉（见 [`Prepared`]）。
    ///
    /// `local` 与 `dynamic` 的差别**只有目标从哪来**：前者从规则里取（`target_host` /
    /// `target_port` 被原样送进 `direct_tcpip`，由**对端**解析 —— 在本地解析就等于绕开
    /// 跳板机），后者由客户端在 SOCKS5 握手里说。`remote` 的绑定在**服务端**、
    /// 目标在**本机**（同两个字段，两侧的角色正好相反）。
    async fn prepare(&self, app: &AppHandle) -> Result<Prepared, TunnelError> {
        match self.direction {
            Direction::Local => {
                let ingress = Ingress::Fixed(self.target()?);
                let listener = bind_local(app, self, ingress).await?;
                Ok(Prepared::Local(listener))
            }
            // `dynamic` 的目标由客户端逐条说 —— 库的 `CHECK` 也保证它没有目标，
            // 所以这里不需要（也不该）读 `target_*`。
            Direction::Dynamic => {
                let listener = bind_local(app, self, Ingress::Socks5).await?;
                Ok(Prepared::Local(listener))
            }
            // `-R`：绑定地址按**服务端**解释（它的 `localhost` 是它自己），
            // 目标由**本机**解析（对端的 `localhost` 是它自己）。
            Direction::Remote => Ok(Prepared::Remote {
                bind: ForwardTarget::new(self.bind_host.clone(), self.bind_port),
                target: self.target()?,
            }),
        }
    }
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
        direction: row.direction,
        bind_host: row.bind_host,
        bind_port: row.bind_port,
        target_host: row.target_host,
        target_port: row.target_port,
    })
}

/// 绑定这条规则的本地监听。**在握手之前**（顺序的理由见 [`tunnel_open`]）。
///
/// `ingress` 一路传到绑定这一层：绑定地址的**合规范围**由它决定（`Socks5` 只允许回环），
/// 于是那样一条监听根本建不出来 —— 放在"绑上之后再检查"就等于端口已经在听了。
///
/// 绑定在 SSH 的 runtime 上做：那个端口随后要在那条 runtime 上被接受循环轮询，
/// 而 tokio 的 I/O 资源归创建它的 driver（在别处绑、在这儿接受，是把两个 runtime 的
/// 生命周期绑在一起 —— 不必要且难查）。
async fn bind_local(
    app: &AppHandle,
    rule: &Rule,
    ingress: Ingress,
) -> Result<LocalListener, TunnelError> {
    let runtime = app
        .state::<Ssh>()
        .runtime_handle()
        .ok_or_else(|| TunnelError::Internal {
            message: "SSH runtime 不可用：它没建起来，端口也无从绑定".to_owned(),
        })?;
    let host = rule.bind_host.clone();
    let port = rule.bind_port;
    runtime
        .spawn(async move { LocalListener::bind(&host, port, ingress).await })
        .await
        .map_err(|err| TunnelError::Internal {
            message: format!("绑定本地端口的那条任务没有回话：{err}"),
        })?
        .map_err(TunnelError::from_bind)
}

/// 把一条隧道从 `连接中` 推到 `已连接`（或 `失败`）。
///
/// 三个方向在这里分岔，而分岔只有一处：[`Prepared`] 决定连接起来之后是
/// "把连接交给本机监听"还是"请服务端监听"。
async fn drive(
    app: &AppHandle,
    handle: SessionHandle,
    rule: &Rule,
    prepared: Prepared,
) -> Result<TunnelAttempt, TunnelError> {
    let Some(runtime) = app.state::<Ssh>().runtime_handle() else {
        // 没有 runtime 就连不上（`connect_connection` 也是这个前提）。隧道已经登记着，
        // 因此要和"连不上"一样落到 `失败` 并把原因交出去 —— 而不是停在永远不动的 `连接中`。
        return failed(
            app,
            handle,
            TunnelError::Internal {
                message: "SSH runtime 不可用：连接建立不起来".to_owned(),
            },
        );
    };

    let outcome = {
        let ssh = app.state::<Ssh>();
        let vault = app.state::<Vault>();
        crate::ssh::connect_connection(&ssh, &vault, rule.host_id).await
    };
    let connection = match outcome {
        Ok(connection) => connection,
        Err(err) => return failed(app, handle, TunnelError::from(err)),
    };

    // 连接与监听合成一件事：转发任务持有两者，"已连接"因此意味着
    // **那一侧端口在听**且**连接活着**。
    let forward = match prepared {
        Prepared::Local(listener) => ActiveForward::Local(listener.serve(&runtime, connection)),
        Prepared::Remote { bind, target } => {
            // `-R` 的"绑定"发生在这里而不是更早 —— 它是对那条连接的一次请求，
            // 没有连接就没有可请求的对象（plan 0604 / D10）。
            let opened =
                RemoteForward::open(&runtime, connection, bind.host(), bind.port(), target).await;
            match opened {
                Ok(forward) => ActiveForward::Remote(forward),
                Err(err) => return failed(app, handle, TunnelError::from_remote(err)),
            }
        }
    };

    let sessions = app.state::<Sessions>().inner().clone();
    sessions
        .attach_tunnel_forward(handle, forward)
        .map_err(TunnelError::from)?;
    apply(app, &sessions, handle, TunnelState::Connected)?;
    Ok(TunnelAttempt {
        handle,
        failure: None,
    })
}

/// 落到 `失败`（**发出事件**）并把原因交出去。
///
/// `scope.md` §2.2 的"失败必须可见"靠的就是这一步：用户不会去读 probe，
/// 托盘与界面读的是状态。
fn failed(
    app: &AppHandle,
    handle: SessionHandle,
    failure: TunnelError,
) -> Result<TunnelAttempt, TunnelError> {
    let sessions = app.state::<Sessions>().inner().clone();
    let _ = apply(app, &sessions, handle, TunnelState::Failed);
    Ok(TunnelAttempt {
        handle,
        failure: Some(failure),
    })
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
