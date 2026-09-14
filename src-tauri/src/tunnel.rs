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
//! ## 掉线之后怎么办（plan 0605）
//!
//! 一条**已经连上过**的隧道掉线之后，由它自己的**看护任务**按 ADR-0003 D13 的预算重连：
//! 3 次、退避 `1s → 2s → 4s`，耗尽就落到 `失败`（**发出事件**，托盘与界面因此看得见）。
//! 这个任务在首次连上之后才起 —— 一次连接都没成功过的隧道没有"重连"可言，
//! 那一次失败是同步报给用户的（命令返回里带原因，界面上就是那条失败文案）。
//!
//! ⚠️ **重连是"重新走一遍准备 + 连接 + 起转发"**，不是接着用：连接是那条转发的命根子
//! （转发任务持有 `SshConnection`），连接没了转发也就没了。三个方向的差别因此在这里
//! 又出现一次：`-L` / `-D` 重新绑本机端口，`-R` **重新发一次 `tcpip_forward`**
//! —— 远端监听是服务端那条连接的资源，连接一断它就被撤销了。
//!
//! ⚠️ **一条隧道只有一个停止信号**（[`TunnelStop`]，`watch` 的一对）。停止 / 重试 /
//! 退出都从这里进去（D5 的"关闭 Session 立刻关闭连接"要能到这），而等在它上面的那一方
//! （在途的一次尝试、或者看护循环）在**每一次 `await`** 上回应它 —— 所以停止不必等一次
//! 握手的 10 秒。
//!
//! ## 关闭一条转发 `Session`（plan 0606）
//!
//! 转发 `Session` 的"关闭"就是 [`tunnel_stop`]（面板上那个「停止」；终端那条 `close_session`
//! 是另一回事，见 `scope.md` §5.2 / §5.6）。它要同时做到三件事，而三件事各自都**可以被读出来**：
//!
//! 1. 状态推到 `已停止` 并发事件 —— 界面上因此看得到这一步；
//! 2. 手上的动作停下（在途的尝试 / 看护循环），**转发**回收（停止监听 → 收在途连接 →
//!    礼貌断开连接）—— 收尾在 runtime 上做，命令发完信号即返回；
//! 3. 从注册表摘掉（幂等：已经不在册就是"已经关了"）。
//!
//! 判据是"连接数与重连任务数都归零"（ROADMAP），所以**这两个数必须有人报**：
//! `akasha_ssh::live_connections()`（连接对象还活着几条）与 [`watch_tasks`]（看护循环还跑着
//! 几条），由 `residue` 探针读出来。⚠️ 不能拿实体表当证据 —— 命令自己就会把实体摘掉。
//!
//! ⚠️ **在途的尝试也要能被停止**（同上）：一条**正在握手**的隧道手上只有一个 socket，
//! 停止它唯一的办法是让那次尝试从中间停下（见 [`attempt_with_stop`]）。

use akasha_core::{Reconnect, SessionId, TunnelState, TunnelTransitionError};
use akasha_ssh::{
    ForwardEnd, ForwardEnding, ForwardTarget, Ingress, LocalForward, LocalListener, RemoteForward,
    SshError,
};
use akasha_store::StoreError;
use akasha_store::pools::forwards::Direction;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::{AppHandle, Emitter, Manager};
use tauri_specta::Event;
use tokio::sync::watch;

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
    /// 这条隧道的停止信号（[`TunnelStop`]）：**实体一登记就有**，手上的动作各订一份接收端。
    ///
    /// 它与 `forward` **分开**：转发本体归实体表（停止与 probe 要它），而"停下手上的动作"
    /// 由这个信号负责（在途的尝试与看护循环各自在 `select!` 里等它）。
    stop: TunnelStop,
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
            stop: TunnelStop::new(),
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

    /// 这条隧道的停止信号（一次尝试 / 看护循环各订一份接收端）。
    pub(crate) fn signal(&self) -> TunnelStopSignal {
        self.stop.signal()
    }

    /// 让手上那个动作停下（重试要用：先停掉旧的，再按现在的配置来一次）。
    pub(crate) fn stop_action(&self) {
        self.stop.stop();
    }

    /// **收掉这条隧道**（plan 0606）：先让它的动作停下，再收掉转发。
    ///
    /// 只有这一份实现，`tunnel_stop` 与 `Sessions::shutdown_all` 都走它 —— 收尾的顺序
    /// （停止信号先、转发后）是这条隧道能不能干净收场的一部分，抄第二份的下场是其中一条
    /// 慢慢长歪（同 `hops_chain` 的理由）。
    ///
    /// ⚠️ 收尾本身是**异步**的（停止转发 = 发信号，收尾在 runtime 上做）；要看结果的地方
    /// 读 `akasha_ssh::live_connections()` / 对端的连接计数，不看这个调用的返回。
    pub(crate) fn reclaim(self) {
        // 动作**先**停：它是唯一会在这条隧道已经决定关闭之后再把它连起来的东西。
        self.stop.stop();
        if let Some(forward) = self.forward {
            forward.shutdown();
        }
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

/// **这条隧道的停止信号**（plan 0606）：发送端归**实体自己**，接收端发出去给每一个动作。
///
/// 一条隧道在任何时刻最多只有一个在跑的动作 —— 一次连接尝试，或者连上之后的看护循环
/// （等它掉线、按预算重连）。两者**用同一个信号**，因为"停止这条隧道"对它们而言是同一件事：
/// 停下手上的动作，别再碰这条隧道的任何资源。
///
/// ⚠️ **为什么是 `watch` 而不是 0605 那个 `oneshot`**：`oneshot` 的发送端被移动/替换时，
/// 接收端会立刻收到"通道关闭"，于是 `select!` 的两个分支同时就绪，`tokio::select!` 会
/// **随机挑一个** —— 一次**成功**的连接会因此有大约一半的机会被报成"已停止"。
/// 发送端归实体之后，接受端只在两件事上被唤醒：**真的被要求停止**，或者**实体没了**。
///
/// ⚠️ 尝试也要订它：不订的话，一条**正在握手**的隧道被停止后，那次尝试仍握着一个 socket，
/// 要等握手超时（D15 的 10 s）才结束 —— 与"关闭 `Session` 立刻断连"（`scope.md` §2.2）不符。
#[derive(Debug)]
pub struct TunnelStop {
    stop: watch::Sender<bool>,
}

impl TunnelStop {
    /// 建一对：实体持有返回的发送端，动作各订一个接收端。
    fn new() -> Self {
        Self {
            stop: watch::channel(false).0,
        }
    }

    /// 给一个动作（一次尝试 / 看护循环）一份接收端。
    ///
    /// ⚠️ **订的时候把"当前值"记为已见**：重试要先停掉上一个动作再起新的，那一次停止
    /// 不该把新动作也一起停掉（D12 的"重试 = 再来一次"）。
    pub(crate) fn signal(&self) -> TunnelStopSignal {
        TunnelStopSignal(self.stop.subscribe())
    }

    /// 让它停下（发完即返回）。`Send` 失败 = 接收端都已经走了 —— 那不是错误。
    ///
    /// 值始终是 `true`，但**每次都通知**（`watch::Sender::send` 不比对新旧值）——
    /// 所以"停了之后又起一个动作、再停它"照样有效。
    pub(crate) fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

/// 停止信号的接收端：等到"这条隧道被要求停下"。
///
/// **可以克隆**：一次在途的尝试要用两份 —— 一份在命令的 `select!` 里（停止它），
/// 一份送进那次握手的阻塞线程（见 [`connect_once`] —— 扔掉 await 取消不了阻塞任务）。
#[derive(Clone)]
pub struct TunnelStopSignal(watch::Receiver<bool>);

impl TunnelStopSignal {
    /// 等到要求停止（或者实体没了）。
    ///
    /// **可在 `select!` 里随便丢**：上游 `changed()` 是取消安全的，别的分支先就绪时
    /// 不会把一次真实的通知吞掉。
    pub(crate) async fn stopped(&mut self) {
        // 返回 `Err` = 发送端没了（实体已经不在）= 没有东西还需要接着跑了。
        let _ = self.0.changed().await;
    }
}

/// 进程里正在跑的**隧道动作**个数（看护循环与在途尝试，plan 0606）。
///
/// 为什么要数：判据"关闭 `Session` 之后**重连任务数**归零"要能被断言。实体表不能充当证据
/// —— 关闭命令自己就会把实体摘掉；而这个数说的是"还有几条循环/尝试在跑"，与实体表无关。
///
/// 用 RAII 计数而不是"登记 + 注销两张表"：增减只发生在任务的起止上，**不可能与事实分叉**。
static WATCH_TASKS: AtomicUsize = AtomicUsize::new(0);

/// **还在跑的凭据**：看护任务持有它，任务返回即减一。
struct WatchTask;

impl WatchTask {
    fn new() -> Self {
        WATCH_TASKS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for WatchTask {
    fn drop(&mut self) {
        WATCH_TASKS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// 进程里正在跑的隧道看护任务数（掉线之后负责重连的那条循环）。
pub fn watch_tasks() -> usize {
    WATCH_TASKS.load(Ordering::Relaxed)
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

    /// 这次失败之后**还要不要自动再来一次**（ADR-0003 D13 的判据表）。
    ///
    /// 按"哪一层坏了"分：
    ///
    /// * **传输层**（连不上 / 超时 / 跳板那条线断了）→ 重试，那正是网络抖动；
    /// * **端口没拿到**（本机或服务端那一侧）→ 重试：端口是会被别人临时占住的资源；
    /// * **认证 / 主机密钥 / 配置 / 内部状态** → **不重试**。D13 给的后果很具体：
    ///   以错误的口令连续尝试三次正是账号锁定的经典成因；而主机密钥变了要的是
    ///   用户显式确认，不是我们再试三遍。
    fn retryable(&self) -> bool {
        match self {
            Self::Bind { .. } | Self::RemoteBind { .. } => true,
            Self::Failed { kind, .. } => {
                matches!(kind, SshFailureKind::Connect | SshFailureKind::Jump)
            }
            _ => false,
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
    attempt_with_stop(&app, handle, &rule, prepared).await
}

/// 手动重试（D12：`失败 / 已停止 → 连接中`，尝试次数清零）。
///
/// 规则**重新读一遍**：端口与目标可能在上一次失败之后被改过，而重试的用户意图正是
/// "按现在的配置再来一次"。旧的那条转发与看护任务先收掉 —— 重试是"重来一次"，
/// 不是"再来一条"。
///
/// ⚠️ **只有终态能重试，而这条检查排在绑定之前**：`重连中` 也在等下一次尝试，
/// 但那是看护任务的事（它有自己的次数与退避），用户能做的重试只在 `失败 / 已停止` 上
/// 有意义（D12）。放在绑定之前是因为"先占一个端口再报状态不对"会在屏幕上留下一个
/// 一闪而过的失败信息，而用户真正要看的是"现在还轮不到你重试"。
///
/// `Err` 只在"这个句柄不是一条隧道 / 状态不是终态 / 方向不对 / 端口没拿到 / 状态推不动"
/// 时返回；**又没连上**属于 [`TunnelAttempt::failure`]。
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
    let state = sessions
        .tunnel_state(handle)
        .ok_or(TunnelError::NotATunnel { handle })?;
    if let Err(err) = state.retry() {
        return Err(TunnelError::Transition {
            message: err.to_string(),
        });
    }

    // 先让还在跑的那个动作停下，再收转发：反过来的话，那条循环可能在我们拆它的同时
    // 把这条隧道重新连起来（重试是"按现在的配置再来一次"，不是"再多一条"）。
    sessions.stop_tunnel_action(handle)?;
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
    attempt_with_stop(&app, handle, &rule, prepared).await
}

/// 停止一条隧道（**转发 `Session` 的"关闭"就是这里**，plan 0606）：`已停止`（发事件）→
/// 停下手上的动作（在途的一次尝试 / 看护循环）→ 回收转发（停止监听 + 断开连接）→ 从注册表摘掉。
///
/// 摘牌是**幂等**的：重复点击、或这条已经被别的路径收掉时返回 `Ok`，而不是报一个
/// 用户没有下一步动作可做的错。
#[tauri::command]
#[specta::specta]
pub fn tunnel_stop(app: AppHandle, handle: SessionHandle) -> Result<(), TunnelError> {
    let sessions = app.state::<Sessions>().inner().clone();
    // 状态**先**推到 `已停止`（发事件），再摘牌：摘牌之后那个句柄已经不在表里，状态机无从
    // 走这一步，事件也就永远不会发出去 —— 而界面上"已停止"这一态正是它。
    match apply(&app, &sessions, handle, TunnelState::Stopped) {
        Ok(_) => {}
        // 已经不在册（重复点击 / 被别的路径收掉了）：这就是"已经关了"，不是错误。
        Err(TunnelError::NotATunnel { .. }) => return Ok(()),
        Err(err) => return Err(err),
    }

    let Some(tunnel) = sessions.remove_tunnel(handle).map_err(TunnelError::from)? else {
        return Ok(());
    };
    tunnel.reclaim();
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

/// **首次尝试**：连上就起看护任务；连不上就落到 `失败`（那条隧道仍在册，可手动重试）。
///
/// 尝试外面包一层"能被停止"（plan 0606）：见 [`attempt_with_stop`]。
async fn attempt_with_stop(
    app: &AppHandle,
    handle: SessionHandle,
    rule: &Rule,
    prepared: Prepared,
) -> Result<TunnelAttempt, TunnelError> {
    let sessions = app.state::<Sessions>().inner().clone();
    // 订停止信号**在开始之前**：反过来的话，一个正在握手的句柄有一小段谁也叫不停的时间，
    // 而"关闭 `Session` 立刻断连"没有宽限期（`scope.md` §2.2）。
    let Some(mut stop) = sessions.tunnel_signal(handle) else {
        return Err(TunnelError::NotATunnel { handle });
    };

    let attempt_signal = stop.clone();
    tokio::select! {
        outcome = attempt_open(app, handle, rule, prepared, attempt_signal) => outcome,
        // 被停止：把这次尝试连它的 future 一起丢掉 —— 那个 socket 是这条 future 造的，
        // 随它一起关闭（不丢的话要等握手超时，D15 的 10 s）。返回"不是一条隧道"是因为
        // 那时句柄**确实**已经不在册（`tunnel_stop` 先摘牌、后发这个信号），而这条错误的
        // 文案本来就写着"已停止或未打开"。
        () = stop.stopped() => Err(TunnelError::NotATunnel { handle }),
    }
}

/// 连上就起看护任务；连不上就落到 `失败`（那条隧道仍在册，可手动重试）。
async fn attempt_open(
    app: &AppHandle,
    handle: SessionHandle,
    rule: &Rule,
    prepared: Prepared,
    signal: TunnelStopSignal,
) -> Result<TunnelAttempt, TunnelError> {
    match connect_once(app, handle, rule, prepared, signal).await {
        Ok(ending) => {
            start_watch(app, handle, rule.id, ending);
            Ok(TunnelAttempt {
                handle,
                failure: None,
            })
        }
        Err(err) => failed(app, handle, err),
    }
}

/// 连一次：建连接 → 起转发 → 挂上 → `已连接`。返回值是那次转发的**结束信号**。
///
/// 三个方向在这里分岔，而分岔只有一处：[`Prepared`] 决定连接起来之后是
/// "把连接交给本机监听"还是"请服务端监听"。
///
/// 首次尝试与每一次重连走的都是这一条 —— 重连不是"接着用那条连接"，而是**重新来一遍**。
async fn connect_once(
    app: &AppHandle,
    handle: SessionHandle,
    rule: &Rule,
    prepared: Prepared,
    signal: TunnelStopSignal,
) -> Result<ForwardEnding, TunnelError> {
    let Some(runtime) = app.state::<Ssh>().runtime_handle() else {
        // 没有 runtime 就连不上（`connect_connection` 也是这个前提）。
        return Err(TunnelError::Internal {
            message: "SSH runtime 不可用：连接建立不起来".to_owned(),
        });
    };

    let outcome = {
        let ssh = app.state::<Ssh>();
        let vault = app.state::<Vault>();
        // ⚠️ 信号**跟着这次握手一起**进那条阻塞线程：这次尝试若在握手中被停止，
        // 叫停它必须靠这个信号（扔掉 await 取消不了阻塞任务，见 `connect_via_until`）。
        let mut until_stopped = signal;
        let cancel = async move { until_stopped.stopped().await };
        crate::ssh::connect_connection(&ssh, &vault, rule.host_id, cancel).await
    };
    let connection = outcome.map_err(TunnelError::from)?;

    // 连接与监听合成一件事：转发任务持有两者，"已连接"因此意味着
    // **那一侧端口在听**且**连接活着**。
    let (forward, ending) = match prepared {
        Prepared::Local(listener) => {
            let (forward, ending) = listener.serve(&runtime, connection);
            (ActiveForward::Local(forward), ending)
        }
        Prepared::Remote { bind, target } => {
            // `-R` 的"绑定"发生在这里而不是更早 —— 它是对那条连接的一次请求，
            // 没有连接就没有可请求的对象（plan 0604 / D10）。
            // ⚠️ 重连时这一步**必须再做一次**：远端监听是那条连接的资源，连接一断它
            // 就没了（见模块文档）。不重新请求的话，重连会"成功"，而端口不在听。
            let (forward, ending) =
                RemoteForward::open(&runtime, connection, bind.host(), bind.port(), target)
                    .await
                    .map_err(TunnelError::from_remote)?;
            (ActiveForward::Remote(forward), ending)
        }
    };

    let sessions = app.state::<Sessions>().inner().clone();
    sessions
        .attach_tunnel_forward(handle, forward)
        .map_err(TunnelError::from)?;
    apply(app, &sessions, handle, TunnelState::Connected)?;
    Ok(ending)
}

/// 起**看护任务**：这条隧道的连接断掉之后，由它按 D13 的预算重连。
///
/// 它在首次连上之后才起（那时才有连接可以断），停在 SSH 的 runtime 上（与转发任务同一个
/// runtime：这条隧道的一切都在那里）。
fn start_watch(app: &AppHandle, handle: SessionHandle, rule_id: i64, ending: ForwardEnding) {
    let Some(runtime) = app.state::<Ssh>().runtime_handle() else {
        // 走到这里说明连接刚刚建立过，而它一定用过这个 runtime —— 拿不到就只记一条。
        tracing::warn!(handle, reason = "no-runtime", "tunnel watch not started");
        return;
    };
    // 订一份停止信号。实体已经不在册（用户在我们连上它的同时把它停了）就没有要看护的东西。
    let sessions = app.state::<Sessions>().inner().clone();
    let Some(signal) = sessions.tunnel_signal(handle) else {
        tracing::debug!(handle, "tunnel watch skipped");
        return;
    };
    let app = app.clone();
    let policy = crate::config::reconnect(&app);
    runtime.spawn(watch(app, handle, rule_id, policy, ending, signal));
}

/// 看护一条隧道：等这次转发结束，然后按预算重连（ADR-0003 D13）。
///
/// ⚠️ **只有"实体仍然已连接"才谈得上重连。** 停止、重试、退出都会把它推走 ——
/// 那些情形下"转发结束"是我们让它结束的，再连一遍就是把用户刚停掉的东西拉起来。
/// 这一条判据同时兜住了与停止命令的竞争（先推状态、后收转发）。
async fn watch(
    app: AppHandle,
    handle: SessionHandle,
    rule_id: i64,
    policy: Reconnect,
    mut ending: ForwardEnding,
    mut stop: TunnelStopSignal,
) {
    // 上账排在**第一行**：任务的起止就是这笔账的全部依据（`watch_tasks`，plan 0606）。
    let _running = WatchTask::new();
    loop {
        // 这一次转发结束了（或者我们被要求停止）。
        let end = tokio::select! {
            end = ending.ended() => end,
            () = stop.stopped() => return,
        };
        if end == ForwardEnd::Stopped {
            return;
        }

        let sessions = app.state::<Sessions>().inner().clone();
        if sessions.tunnel_state(handle) != Some(TunnelState::Connected) {
            return;
        }
        // 那次转发已经没有用途了：收掉它，probe 因此不会再报一个并不存在的监听地址。
        if let Some(forward) = sessions.take_tunnel_forward(handle) {
            forward.shutdown();
        }

        let Some(next) = reconnect(&app, handle, rule_id, &policy, &mut stop).await else {
            return;
        };
        ending = next;
    }
}

/// 按 D13 的预算重连，返回**新那次转发**的结束信号。
///
/// `None` = 不再继续：次数耗尽（已经落到 `失败`）、出现不该重试的失败（认证 / 主机密钥 /
/// 配置）、或者被要求停止。
async fn reconnect(
    app: &AppHandle,
    handle: SessionHandle,
    rule_id: i64,
    policy: &Reconnect,
    stop: &mut TunnelStopSignal,
) -> Option<ForwardEnding> {
    let sessions = app.state::<Sessions>().inner().clone();
    let mut last: Option<TunnelError> = None;

    for attempt in 1..=policy.max_attempts {
        // `重连中(n)` 的含义就是"正在等第 n 次"（`akasha_core::TunnelState`），
        // 所以状态先走、退避在后。
        let Some(delay) = policy.delay(attempt) else {
            break;
        };
        if apply(
            app,
            &sessions,
            handle,
            TunnelState::Reconnecting { attempt },
        )
        .is_err()
        {
            return None;
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            () = stop.stopped() => return None,
        }
        if apply(app, &sessions, handle, TunnelState::Connecting).is_err() {
            return None;
        }

        let outcome = tokio::select! {
            outcome = attempt_again(app, handle, rule_id, stop.clone()) => outcome,
            () = stop.stopped() => return None,
        };
        let failure = match outcome {
            Ok(ending) => {
                tracing::info!(handle, attempt, "tunnel reconnected");
                return Some(ending);
            }
            Err(err) => err,
        };
        // 实体没了 = 用户在重连期间把这条隧道停掉了：不再往它身上写状态，静默退出
        // （用 `?` 是因为"它不在册"与下面那条"不再继续"本来就是同一个返回值）。
        sessions.tunnel_state(handle)?;

        if !failure.retryable() {
            // D13：认证失败、主机密钥不匹配、配置 / 协议错误**不重试** ——
            // 以错误的口令连续尝试三次正是账号锁定的经典成因。
            give_up(app, &sessions, handle, attempt, &failure);
            return None;
        }
        last = Some(failure);
    }

    // 次数耗尽（或者预算本来就是 0）：`失败` **可见**，原因进日志。
    give_up_last(app, &sessions, handle, policy.max_attempts, last.as_ref());
    None
}

/// 重连时的一次尝试：规则**重新读一遍**（端口与目标可能在这期间被改过）。
async fn attempt_again(
    app: &AppHandle,
    handle: SessionHandle,
    rule_id: i64,
    signal: TunnelStopSignal,
) -> Result<ForwardEnding, TunnelError> {
    let rule_id = ForwardId::try_from(rule_id).map_err(|_| TunnelError::Internal {
        message: format!("转发规则 id 超出可表示范围（{rule_id}）"),
    })?;
    let rule = load_rule(app, rule_id)?;
    let prepared = rule.prepare(app).await?;
    connect_once(app, handle, &rule, prepared, signal).await
}

/// 落到 `失败` 并记下**为什么**（可见的那一半由状态承担，原因只有日志里说得清）。
fn give_up(
    app: &AppHandle,
    sessions: &Sessions,
    handle: SessionHandle,
    attempt: u32,
    failure: &TunnelError,
) {
    let _ = apply(app, sessions, handle, TunnelState::Failed);
    tracing::warn!(
        handle,
        attempt,
        reason = %failure,
        "tunnel reconnect abandoned"
    );
}

/// 次数耗尽：同样是 `失败`，但说法不同（"还剩次数"与"次数用完"是两件事）。
fn give_up_last(
    app: &AppHandle,
    sessions: &Sessions,
    handle: SessionHandle,
    attempts: u32,
    last: Option<&TunnelError>,
) {
    let _ = apply(app, sessions, handle, TunnelState::Failed);
    match last {
        Some(failure) => tracing::warn!(
            handle,
            attempts,
            reason = %failure,
            "tunnel reconnect exhausted"
        ),
        None => tracing::warn!(handle, attempts, "tunnel reconnect exhausted"),
    }
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

#[cfg(test)]
mod tests {
    //! **D13 那张判据表的机器可读那一半**：哪些失败会自动重试，哪些不。
    //!
    //! 它值得单测，是因为分错档的后果不对称：该重试的没重试只是少一次自动恢复，
    //! 而**不该重试的却重试了**会以错误的口令连试三次 —— 那正是账号锁定的经典成因。

    use super::*;

    #[test]
    fn transport_and_port_failures_are_retried() {
        for kind in [SshFailureKind::Connect, SshFailureKind::Jump] {
            let err = TunnelError::Failed {
                kind,
                message: "x".to_owned(),
            };
            assert!(err.retryable(), "{kind:?} 是网络那一层的失败，应当重试");
        }
        for err in [
            TunnelError::Bind {
                address: "127.0.0.1:1".to_owned(),
                message: "被占着".to_owned(),
            },
            TunnelError::RemoteBind {
                address: "127.0.0.1:1".to_owned(),
                message: "被占着".to_owned(),
            },
        ] {
            assert!(err.retryable(), "端口是会被别人临时占住的资源：{err}");
        }
    }

    #[test]
    fn auth_and_configuration_failures_are_not_retried() {
        for kind in [
            SshFailureKind::Auth,
            SshFailureKind::HostKeyChanged,
            SshFailureKind::HostKeyRejected,
            SshFailureKind::HostKeyUnknown,
            SshFailureKind::HostKeyCache,
            SshFailureKind::Other,
        ] {
            let err = TunnelError::Failed {
                kind,
                message: "x".to_owned(),
            };
            assert!(!err.retryable(), "{kind:?} 不该自动重试");
        }
        for err in [
            TunnelError::Locked,
            TunnelError::NoSuchForward { id: 1 },
            TunnelError::NoSuchHost { id: 1 },
            TunnelError::NotATunnel { handle: 1 },
            TunnelError::NotLoopback {
                address: "0.0.0.0:1".to_owned(),
            },
            TunnelError::Transition {
                message: "x".to_owned(),
            },
            TunnelError::Internal {
                message: "x".to_owned(),
            },
        ] {
            assert!(!err.retryable(), "这一类没有自动重试的余地：{err}");
        }
    }

    /// 停止信号的两条性命攸关的性质（plan 0606），都是"客户端看到的行为"：
    ///
    /// 1. **订在停止之后的接收端不响**：重试要"先停旧的、再起新的"，那一次停止绝不能
    ///    把新起的动作也一起停掉（D12 的重试 = 按现在的配置再来一次）。
    /// 2. **第二次停止照样响**：`watch` 的值始终是 `true`，但每一次 `send` 都通知 ——
    ///    靠"值变了才通知"的话，第二条隧道动作就永远停不下来。
    #[tokio::test]
    async fn a_stop_signal_only_wakes_the_actions_that_were_running() {
        let stop = TunnelStop::new();

        let mut first = stop.signal();
        stop.stop();
        first.stopped().await; // 第一条：被叫停，立刻返回

        // 第二条是在停止**之后**起来的（重试那条路）：上一次的停止与它无关。
        let mut second = stop.signal();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), second.stopped())
                .await
                .is_err(),
            "重试新起的动作不该被上一次的停止叫停"
        );

        // 但它自己那一次停止必须有效（值没变，通知还是要有）。
        stop.stop();
        tokio::time::timeout(std::time::Duration::from_millis(200), second.stopped())
            .await
            .expect("第二次停止没有叫停第二个动作");
    }
}
