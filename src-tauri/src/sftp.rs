//! **SFTP 实体、命令与两侧连接**（plan 0701 / 0702）—— 双栏会话的资源模型落地处。
//!
//! `docs/scope.md` §4 / §5.1 与 ADR-0006 D3 / D7 把这件事定得很死：
//! **一个 SFTP 会话拥有两侧**，每侧一条**独立**的 SSH 连接；它不依赖任何终端 `Session`
//! （"SFTP 不需要先开终端"）。本模块就是那个实体的登记与驱动：
//!
//! | 事 | 在哪 |
//! |---|---|
//! | 两侧的模型与状态 | 本模块（[`Sftp`] / [`Side`]） |
//! | 实体表与注册表 | [`crate::session::Sessions`] —— **同一张注册表**（ADR-0003 D6），不另立一份 |
//! | 连接怎么建 | [`crate::ssh::connect_connection`]（已认证、没有通道的 `SshConnection`，含跳板链） |
//! | 会话怎么开 | `akasha_ssh::SftpClient`（在一条流上的 SFTP 会话，ADR-0006 D2） |
//! | 文件怎么搬 | `akasha_ssh::transfer`（只认两个 [`Endpoint`]，ADR-0006 D4） |
//! | 过 IPC 的形状与 probe | 本模块（`sftp` 探针） |
//!
//! ## 三条边界
//!
//! 1. **`side` 只是"哪一栏"**（ADR-0006 D7）：它是唯一进入契约的呈现概念，后端不给它
//!    别的含义 —— 资源归那个 `Session`，不归某一侧。所以它只出现在命令参数与探针里，
//!    不出现在任何"归还 / 回收"的判断中。
//! 2. **一侧失败不影响另一侧**：两侧各持各的连接，失败落在**那一侧**
//!    （[`SftpSideState::Failed`] 与 [SftpSideInfo::failure]），另一侧照样可用 ——
//!    这正是"两侧独立"在可断言形式下的样子。
//! 3. **一侧连的是什么**是 [`SftpOrigin`]：本机文件系统，或者池里的一台主机。
//!    两者在传输引擎眼里是**同一件事的两端**（plan 0702），差别只在那一次
//!    [`Endpoint`] 的构造。
//!
//! ## 落盘不变量归端点，取消只有一条路径
//!
//! "临时名 + 原子重命名"在**端点**里（`scope.md` §4.2 / ADR-0006 D4），本模块只做三件事：
//! 把两侧各变成一个端点、把两条端点交给引擎、把引擎的结局记成**可读的状态**。
//! 于是"用户取消"与"关闭 `Session`"是同一个动作 —— 推那个 [`Cancel`]。

use std::sync::{Arc, Mutex};

use akasha_core::SessionId;
use akasha_ssh::transfer::{Cancel, Endpoint, Listing, Progress, TransferRequest};
use akasha_ssh::{InFlight, LocalEndpoint, SftpClient, SshConnection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tokio::runtime::Handle as RuntimeHandle;
use tokio::task::JoinHandle;

use crate::pools::HostId;
use crate::session::{IpcError, SessionHandle, Sessions};
use crate::ssh::{Ssh, SshFailureKind, SshIpcError};
use crate::vault::{ConnError, Vault};

/// 两栏里的哪一栏。**唯一进入契约的呈现概念**（ADR-0006 D7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SftpSide {
    Left,
    Right,
}

impl SftpSide {
    /// 两侧在实体里的下标。
    const fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }

    /// 日志与探针用的稳定短名（**不是** IPC 的表示 —— 那个由 `serde` 定）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// 对侧。B 档要看"另一栏连的是哪台"（plan 0703 的 [`via_host`]）。
    pub const fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// 两侧都要走一遍的地方用它。
    pub const ALL: [SftpSide; 2] = [SftpSide::Left, SftpSide::Right];
}

/// 一侧连的**是什么**：本机文件系统，还是池里的一台主机（plan 0702）。
///
/// `侧` 与 `源` 的关系是"这一栏此刻对着哪一边"，而传输的两个方向都是**侧到侧** ——
/// 于是"本机 ↔ 主机"与将来的"主机 ↔ 主机"（plan 0703）在契约上是同一个形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SftpOrigin {
    /// 本机文件系统。没有连接、没有认证、没有失败档 —— 连上就是"这一栏可以用了"。
    Local,
    /// 池里的一台主机。
    Host { id: HostId },
}

/// 一侧的连接状态。
///
/// 只有四个取值，而且**没有**"重连中"：SFTP 没有重连循环（那是隧道的事，ADR-0003 D13）。
/// 一次连接失败就停在 `失败`，重试是用户的动作（再点一次「连接」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SftpSideState {
    /// 还没连（初始状态），或者连过之后被断开。
    Disconnected,
    /// 正在建连接 / 认证 / 开 sftp 子系统。
    Connecting,
    /// 会话可用。
    Connected,
    /// 上一次尝试失败（原因在 [`SftpSideInfo::failure`]）。
    Failed,
}

/// 一侧的**过 IPC 表示**。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpSideInfo {
    pub side: SftpSide,
    /// 这一栏连的是什么（还没选就是 `None`）。
    pub origin: Option<SftpOrigin>,
    /// 那一台的名字（界面上那一栏的标题；还没选就是空串）。
    pub name: String,
    pub state: SftpSideState,
    /// 上一次失败的原因（`state = failed` 时才有）。**字段值不虚构**：
    /// 没有失败就不写它（`None`），不填一句 "unknown"（`docs/logging.md` 的口径）。
    pub failure: Option<String>,
    /// 当前目录（连上之后才有）。
    pub path: Option<String>,
    /// 这一侧的连接是**经哪台直通**到达的（plan 0703 的 B 档）。`None` = 本机直接连过去。
    ///
    /// **字段值不虚构**：不是直通就不写它，不填一个"0"或空串顶替（`docs/logging.md` 的口径）。
    pub through: Option<HostId>,
    /// 试过直通、没成时那条链的失败原因（改了直连并成功的证据）。
    ///
    /// 与 [`Self::failure`] 分开：那是"这一栏连不上"，这是"原本想走的那条路没走成"。
    /// 两者同时为空是常态；两条都不成时**两个都会有**（各说各的那一次尝试）。
    pub through_failure: Option<String>,
}

/// 一次传输的编号（后端分配，进程内唯一）。
///
/// `u32` 而不是 `u64`：编号要过 IPC，而生成器**拒绝把 64 位的整数导出成 `number`**
/// （`specta` 的 BigInt 禁令：JS 的 `number` 装不下它，悄悄截断比报错更糟）。
/// 二十亿次传输够用，而"悄悄少一位"永远不会好用。
pub type TransferId = u32;

/// 一次传输的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SftpTransferState {
    /// 正在搬。
    #[default]
    Running,
    /// 成功 —— **目标端点的重命名已经落地**（"看到最终名就等于成功"，`scope.md` §4.2）。
    Done,
    /// 失败（原因在 [`SftpTransfer::failure`]）。
    Failed,
    /// 被取消（用户点的，或者 `Session` 被关）。
    Cancelled,
}

/// 一次传输的**过 IPC 表示**。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpTransfer {
    pub id: TransferId,
    /// 从哪一栏、哪一个路径。
    pub from: SftpSide,
    pub from_path: String,
    /// 到哪一栏、哪一个路径。
    pub to: SftpSide,
    pub to_path: String,
    pub state: SftpTransferState,
    /// 已经搬过去的字节数（传输中也读得到 —— 进度就是它）。
    ///
    /// ⚠️ `f64` 而不是 `u64`：同一个 BigInt 禁令（见 [`TransferId`]）。字节数是整数，
    /// 而 `f64` 到 2^53（九千太字节）都精确 —— 传不完的风险不存在，
    /// 换来的是前端不必碰 `BigInt`（那是 `JSON` 里根本没有的类型）。
    pub done: f64,
    /// 源文件的大小（还没打开源之前是 0）。
    pub total: f64,
    /// 失败原因（`state = failed` 时才有）。**字段值不虚构**（同 [`SftpSideInfo::failure`]）。
    pub failure: Option<String>,
    /// 目标那一栏是**经哪台直通**到达的（plan 0703 的 B 档）；`None` = 本机内存中转。
    ///
    /// 记在这次传输上，而不是让读的人去看那一栏的现状：传输是历史记录，而一侧的连接事后
    /// 可能被换掉（换主机 / 重连）—— "这次走的是哪一档"不该跟着变。
    pub via: Option<HostId>,
}

/// 一个 SFTP 会话的**过 IPC 表示**（只读命令与探针共用）。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpSummary {
    pub handle: SessionHandle,
    /// 两侧，**永远两项**且顺序固定（`left` 在前）—— 前端因此不必处理"缺了一侧"。
    pub sides: Vec<SftpSideInfo>,
    /// 这个会话发起过的传输，**新的在前**。
    pub transfers: Vec<SftpTransfer>,
    /// 这个会话的并发读数（plan 0704 的 ADR-0006 D6）。
    pub in_flight: SftpInFlight,
}

/// 并发上限的三个读数（plan 0704）。
///
/// 它们一起答一个问题："上限真的在起作用吗" —— `live` 是此刻在搬的文件数，`peak` 是这个
/// 会话见过的最多同时几个（**会话生命期内**，不回落）。⚠️ 排队中的传输在
/// [`SftpTransfer`] 里与"正在搬"长得一样（状态枚举只有"还没结束"这一档），
/// 分辨它们靠 `live` 比"还没结束的条数"少。
#[derive(Debug, Clone, Copy, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpInFlight {
    /// 上限（来自配置，见 `akasha_core::Transfer::in_flight`）。
    pub limit: u32,
    /// 此刻有几个文件在搬。
    pub live: u32,
    /// 这个会话见过的最多同时几个。
    pub peak: u32,
}

/// 一条目录条目的类型（过 IPC 的稳定短名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SftpEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

impl From<akasha_ssh::transfer::EntryKind> for SftpEntryKind {
    fn from(kind: akasha_ssh::transfer::EntryKind) -> Self {
        match kind {
            akasha_ssh::transfer::EntryKind::File => Self::File,
            akasha_ssh::transfer::EntryKind::Directory => Self::Directory,
            akasha_ssh::transfer::EntryKind::Symlink => Self::Symlink,
            akasha_ssh::transfer::EntryKind::Other => Self::Other,
        }
    }
}

/// 一条目录条目。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntry {
    pub name: String,
    pub kind: SftpEntryKind,
}

/// 一次列目录的结果。
///
/// `path` 是**端点规范化之后**的路径（远端是 `realpath`，本机是 `canonicalize`）：
/// 前端拿它当"当前目录"，于是"返回上一级"不必在前端拼字符串。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpListing {
    pub path: String,
    pub entries: Vec<SftpEntry>,
}

impl From<Listing> for SftpListing {
    fn from(listing: Listing) -> Self {
        Self {
            path: listing.path,
            entries: listing
                .entries
                .into_iter()
                .map(|entry| SftpEntry {
                    name: entry.name,
                    kind: entry.kind.into(),
                })
                .collect(),
        }
    }
}

/// IPC 边界的 SFTP 错误。变体按**用户的下一步动作**分（同 `VaultError` / `SshIpcError`）。
#[derive(Debug, thiserror::Error, Serialize, specta::Type)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum SftpError {
    /// 库没解锁。主机在库里，**没有别的来路**。
    #[error("库是锁着的：SFTP 要先解锁（要连的主机在库里）")]
    Locked,

    /// 池里没有这台主机。
    #[error("主机池里没有这台主机（id {id}）")]
    NoSuchHost { id: HostId },

    /// 这个句柄不是一个 SFTP 会话（已经关闭 / 从来不存在）。
    #[error("会话 {handle} 不是一个 SFTP 会话（已关闭或未打开）")]
    NotAnSftp { handle: SessionHandle },

    /// 这一侧还没有连接。**与失败分开**：那是"连过但没连上"，这是"还没连"。
    #[error("SFTP 的 {side} 这一侧还没有连接（先连接，再列目录或传输）")]
    NotConnected { side: String },

    /// 没有这个编号的传输（已经结束并从表里清掉了，或者编号本来就错）。
    #[error("没有编号为 {id} 的传输")]
    NoSuchTransfer { id: TransferId },

    /// 连接这条路失败。`kind` 是给界面分辨**警报**用的（同 `SshIpcError`）。
    #[error("{message}")]
    Failed {
        kind: SshFailureKind,
        message: String,
    },

    /// 内部状态不可用。
    #[error("内部状态不可用：{message}")]
    Internal { message: String },
}

impl From<SshIpcError> for SftpError {
    fn from(err: SshIpcError) -> Self {
        match err {
            SshIpcError::Locked => Self::Locked,
            SshIpcError::NoSuchHost { id } => Self::NoSuchHost { id },
            SshIpcError::Failed { kind, message } => Self::Failed { kind, message },
            SshIpcError::Internal { message } => Self::Internal { message },
        }
    }
}

impl From<akasha_ssh::SshError> for SftpError {
    fn from(err: akasha_ssh::SshError) -> Self {
        // 分类只有一份：`SshIpcError` 已经按"用户的下一步动作"分好了 ——
        // SFTP 与终端 / 隧道在这件事上的判据完全一致（主机密钥变了就是警报、
        // 认证失败就是不重试），各写一份 `match` 只会慢慢漂移。
        Self::from(SshIpcError::from(err))
    }
}

impl From<ConnError> for SftpError {
    fn from(err: ConnError) -> Self {
        match err {
            ConnError::Locked => Self::Locked,
            ConnError::Store(akasha_store::StoreError::NoSuchRow { .. }) => {
                Self::NoSuchHost { id: 0 }
            }
            ConnError::Store(other) => Self::Internal {
                message: other.to_string(),
            },
            ConnError::Internal(message) => Self::Internal { message },
        }
    }
}

impl From<IpcError> for SftpError {
    fn from(err: IpcError) -> Self {
        match err {
            IpcError::NotFound { handle } => Self::NotAnSftp { handle },
            other => Self::Internal {
                message: other.to_string(),
            },
        }
    }
}

/// 一侧的连接：**连接本体 + 会话句柄**。
///
/// 两者同生共死：会话承载在这条连接的通道上（ADR-0006 D2），连接没了会话也就没了。
/// 所以它们在一个结构里，回收时也一起交出去（[`SftpLink::close`]）。
///
/// `pub(crate)`：换主机 / 关会话时，注册表把"上一次那条连接"整个交出来，
/// 由调用方在**锁外**收掉（在锁里做收尾会把会话表冻住）。
pub(crate) struct SftpLink {
    connection: SshConnection,
    client: SftpClient,
}

impl SftpLink {
    /// 收尾：**显式断开**（不靠 drop，`AGENTS.md` §3.3）。
    ///
    /// 断开要在 runtime 上做，而这条命令（`sftp_close`）是同步的 —— 所以只把任务排上去，
    /// 与 `tunnel_stop` 的做法一致。runtime 起不来时退化成"丢掉连接"：
    /// `Handle` 一 drop，上游的会话任务随之结束，服务端看到的是 TCP 断开。
    fn close(self, runtime: Option<&RuntimeHandle>) {
        let Self { connection, client } = self;
        // 先放掉会话句柄：通道随那条流关闭，服务端因此先看到 SFTP 那一头结束，
        // 再看到连接断开。
        drop(client);
        match runtime {
            Some(handle) => {
                handle.spawn(async move {
                    connection.disconnect().await;
                });
            }
            None => drop(connection),
        }
    }
}

/// 一侧的全部状态。
struct Side {
    origin: Option<SftpOrigin>,
    name: String,
    state: SftpSideState,
    failure: Option<String>,
    path: Option<String>,
    /// 这一侧的连接实际是经哪台直通到达的（plan 0703 的 B 档）。
    through: Option<HostId>,
    /// 试过直通但没成的原因（回退到本机直连的证据，plan 0703）。
    through_failure: Option<String>,
    link: Option<SftpLink>,
}

impl Default for Side {
    fn default() -> Self {
        Self {
            origin: None,
            name: String::new(),
            state: SftpSideState::Disconnected,
            failure: None,
            path: None,
            through: None,
            through_failure: None,
            link: None,
        }
    }
}

impl Side {
    fn info(&self, side: SftpSide) -> SftpSideInfo {
        SftpSideInfo {
            side,
            origin: self.origin,
            name: self.name.clone(),
            state: self.state,
            failure: self.failure.clone(),
            path: self.path.clone(),
            through: self.through,
            through_failure: self.through_failure.clone(),
        }
    }
}

/// 一次传输的**活的**状态：引擎那条任务写它，命令层与探针读它。
///
/// 它刻意**不放在会话表里**：搬字节的那条任务在另一条线上跑，让它每次都去抢会话表的锁
/// 等于把"进度可读"变成"整个 SFTP 会话卡住"。于是这里只有两样东西：
/// 一个原子读数（[`Progress`]）与一把只护状态字符串的短锁。
pub(crate) struct Tracked {
    from: SftpSide,
    from_path: String,
    to: SftpSide,
    to_path: String,
    /// 这次传输的目标端点是经哪台直通到达的（`None` = 本机内存中转，plan 0703）。
    ///
    /// 登记那一刻从目标那一栏抄下来，此后**不再变**：它是这次传输的属性，不是那一栏的现状。
    via: Option<HostId>,
    /// 引擎写、探针读（`AtomicU64` × 2）。
    progress: Progress,
    /// 状态与失败原因。锁只在读写它时持有，里面没有 `await`。
    outcome: Mutex<TrackedOutcome>,
    /// 中止信号 —— **用户取消与关闭 `Session` 推的是同一个**（ADR-0006 D4）。
    cancel: Cancel,
}

#[derive(Default)]
struct TrackedOutcome {
    state: SftpTransferState,
    failure: Option<String>,
}

impl Tracked {
    fn new(
        from: SftpSide,
        from_path: String,
        to: SftpSide,
        to_path: String,
        via: Option<HostId>,
    ) -> Self {
        Self {
            from,
            from_path,
            to,
            to_path,
            via,
            progress: Progress::default(),
            outcome: Mutex::new(TrackedOutcome::default()),
            cancel: Cancel::new(),
        }
    }

    /// 引擎回来了：把结局记下来（**成功 = 目标端点的重命名已经落地**）。
    fn finish(&self, outcome: Result<u64, akasha_ssh::SshError>) {
        let mut slot = match self.outcome.lock() {
            Ok(slot) => slot,
            // 锁中毒只可能是别的线程 panic 过 —— 那不该让"这次传输的结局"丢失，
            // 所以取回内层值继续写（这一条路径上没有任何需要保持一致的不变量）。
            Err(poisoned) => poisoned.into_inner(),
        };
        match outcome {
            Ok(_) => slot.state = SftpTransferState::Done,
            Err(akasha_ssh::SshError::Cancelled) => slot.state = SftpTransferState::Cancelled,
            Err(err) => {
                slot.state = SftpTransferState::Failed;
                slot.failure = Some(err.to_string());
            }
        }
    }

    /// 结局与失败原因（一次锁拿到两个，避免"状态是新的、原因是旧的"这种拼接）。
    fn outcome(&self) -> (SftpTransferState, Option<String>) {
        let slot = match self.outcome.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        (slot.state, slot.failure.clone())
    }
}

/// 登记表里的一条：状态 + 搬字节的那条任务。
///
/// 留着 `JoinHandle` 只为一件事：**关 `Session` 时要等清理落地**（`scope.md` §4.2 把
/// "关闭 `Session`"与"失败 / 取消"列在同一格）。不等的话，`sftp_close` 会在远端临时文件
/// 还没删掉的时候就断开连接 —— 用户看到的是"会话关了，而那个 `.part` 留下了"。
struct Transfer {
    /// 编号在**登记那一刻**由会话分配（`Tracked` 里没有它：那个结构是引擎与探针共用的，
    /// 而编号是登记表的概念）。
    id: TransferId,
    tracked: Arc<Tracked>,
    /// `None` = 任务已经等过了（`stop_transfers` 只等一次）。
    task: Option<JoinHandle<()>>,
}

impl Transfer {
    fn info(&self) -> SftpTransfer {
        let (state, failure) = self.tracked.outcome();
        SftpTransfer {
            id: self.id,
            from: self.tracked.from,
            from_path: self.tracked.from_path.clone(),
            to: self.tracked.to,
            to_path: self.tracked.to_path.clone(),
            state,
            done: self.tracked.progress.done() as f64,
            total: self.tracked.progress.total() as f64,
            failure,
            via: self.tracked.via,
        }
    }
}

/// 一个 SFTP 会话：**两栏各自一条连接**（ADR-0006 D3）。
pub struct Sftp {
    id: SessionId,
    sides: [Side; 2],
    /// 发起过的传输，**新的在前**（探针与界面都按这个顺序读）。
    transfers: Vec<Transfer>,
    /// 下一个传输编号。
    next_transfer: TransferId,
    /// **并发上限**（plan 0704，ADR-0006 D6）：这个会话同时搬几个文件。
    ///
    /// 归**会话**而不是归某一次传输：上限要跨任务共享，而"一个文件一条命令"意味着
    /// 一次调用看不见别的调用。克隆出去的是同一个（内部 `Arc`）。
    in_flight: Arc<InFlight>,
}

impl Sftp {
    /// 新建一个两侧都还没连的会话。`in_flight` 是同时搬几个文件的上限。
    pub(crate) fn new(id: SessionId, in_flight: u32) -> Self {
        Self {
            id,
            sides: [Side::default(), Side::default()],
            transfers: Vec::new(),
            next_transfer: 1,
            in_flight: Arc::new(InFlight::new(in_flight)),
        }
    }

    /// 并发上限本身（克隆 = 同一个）。传输那条任务拿它去占空位。
    pub(crate) fn in_flight(&self) -> Arc<InFlight> {
        Arc::clone(&self.in_flight)
    }

    /// 三个读数（探针与界面读它）。
    pub(crate) fn in_flight_info(&self) -> SftpInFlight {
        SftpInFlight {
            limit: self.in_flight.limit(),
            live: self.in_flight.live(),
            peak: self.in_flight.peak(),
        }
    }

    /// 注册表里的名字（注销用）。
    pub(crate) fn id(&self) -> SessionId {
        self.id
    }

    pub(crate) fn sides(&self) -> Vec<SftpSideInfo> {
        SftpSide::ALL
            .iter()
            .map(|side| self.sides[side.index()].info(*side))
            .collect()
    }

    /// 这个会话发起过的传输（新的在前）。
    pub(crate) fn transfers(&self) -> Vec<SftpTransfer> {
        self.transfers.iter().rev().map(Transfer::info).collect()
    }

    /// 这一侧要开始连了：记住它选的是什么，并把状态推到 `连接中`。
    ///
    /// ⚠️ **先把上一次的连接交出去**（返回给调用方在锁外收掉）：换一台主机等于放弃
    /// 上一条连接，两条连接同时挂着会漏掉一条 —— 而它没有主人，也就没人回收。
    pub(crate) fn prepare_connect(
        &mut self,
        side: SftpSide,
        origin: SftpOrigin,
    ) -> Option<SftpLink> {
        let slot = &mut self.sides[side.index()];
        let previous = slot.link.take();
        slot.origin = Some(origin);
        slot.state = SftpSideState::Connecting;
        slot.failure = None;
        slot.path = None;
        // 直通那一档是**上一次**的事实：这一栏现在还没连上任何地方。
        // `through_failure` 一起清掉 —— 它说的是"这一次尝试想走直通、没走成"。
        slot.through = None;
        slot.through_failure = None;
        previous
    }

    /// 这一侧连上了（一台主机）：把名字、连接与**它实际是怎么到达的**挂上。
    ///
    /// `through` 是 B 档的落点（plan 0703）：`Some(A)` = 这条连接是"本机 → A → 这一台"，
    /// `None` = 本机直接连过去。
    pub(crate) fn attach(
        &mut self,
        side: SftpSide,
        name: String,
        connection: SshConnection,
        client: SftpClient,
        through: Option<HostId>,
    ) {
        let slot = &mut self.sides[side.index()];
        slot.name = name;
        slot.state = SftpSideState::Connected;
        slot.failure = None;
        slot.through = through;
        slot.link = Some(SftpLink { connection, client });
    }

    /// 这一侧连上了（**本机**，plan 0702）：没有连接可挂，只有起点目录。
    pub(crate) fn attach_local(&mut self, side: SftpSide, name: String, path: String) {
        let slot = &mut self.sides[side.index()];
        slot.name = name;
        slot.state = SftpSideState::Connected;
        slot.failure = None;
        slot.link = None;
        slot.path = Some(path);
        slot.through = None;
    }

    /// 试过直通、没成：把那一次的原因记下来（plan 0703 的回退证据）。
    ///
    /// 只动 `through_failure`：这一侧接下来还要走本机直连，成功或失败由 [`Self::attach`] /
    /// [`Self::fail`] 说了算。
    pub(crate) fn note_through_failure(&mut self, side: SftpSide, reason: String) {
        self.sides[side.index()].through_failure = Some(reason);
    }

    /// 这一侧失败了。**只落这一侧** —— 另一侧照样可用（ADR-0006 D3）。
    pub(crate) fn fail(&mut self, side: SftpSide, reason: String) {
        let slot = &mut self.sides[side.index()];
        slot.state = SftpSideState::Failed;
        slot.failure = Some(reason);
        slot.link = None;
        // 没有连接就没有"经谁直通"这件事；而 `through_failure` 留着 —— 两条路都不成时，
        // 用户要能看到**两次尝试各说了什么**。
        slot.through = None;
    }

    /// 这一侧的**端点**（引擎眼里的那一端，ADR-0006 D4）。
    ///
    /// `None` = 这一侧还没连上。返回的是可以脱离会话表使用的东西：
    /// 本机端点没有状态，远端端点是会话句柄的一个克隆（上游内部是 `Arc`）——
    /// 于是命令层可以放掉锁再去 `await`。
    pub(crate) fn endpoint(&self, side: SftpSide) -> Option<Box<dyn Endpoint>> {
        let slot = &self.sides[side.index()];
        match (slot.state, slot.origin) {
            (SftpSideState::Connected, Some(SftpOrigin::Local)) => {
                Some(Box::new(LocalEndpoint::new()))
            }
            (SftpSideState::Connected, Some(SftpOrigin::Host { .. })) => slot
                .link
                .as_ref()
                .map(|link| Box::new(link.client.clone()) as Box<dyn Endpoint>),
            _ => None,
        }
    }

    /// 记下这一侧当前在哪个目录（列目录成功后调用）。
    pub(crate) fn set_path(&mut self, side: SftpSide, path: String) {
        self.sides[side.index()].path = Some(path);
    }

    /// 这一侧的连接是**经哪台直通**到达的（plan 0703 的 B 档）；`None` = 本机直连。
    pub(crate) fn through(&self, side: SftpSide) -> Option<HostId> {
        self.sides[side.index()].through
    }

    /// 登记一次传输，返回它的编号。
    ///
    /// 调用方在**起完任务之后**才登记（`sftp_transfer`）：编号要到这一刻才存在，
    /// 而搬字节的那条任务不需要知道自己的编号。
    pub(crate) fn register_transfer(
        &mut self,
        tracked: Arc<Tracked>,
        task: JoinHandle<()>,
    ) -> TransferId {
        let id = self.next_transfer;
        self.next_transfer += 1;
        self.transfers.push(Transfer {
            id,
            tracked,
            task: Some(task),
        });
        id
    }

    /// 推一次中止信号。返回"有没有这个编号"。
    pub(crate) fn cancel_transfer(&self, id: TransferId) -> bool {
        match self.transfers.iter().find(|transfer| transfer.id == id) {
            Some(transfer) => {
                transfer.tracked.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// 中止全部传输并**等清理落地**（`sftp_close` 的收尾，plan 0702）。
    ///
    /// ⚠️ 必须在**断开连接之前**调用：临时文件是用那条连接删掉的 ——
    /// 先断连接的话，清理会永远做不到，而用户看到的是"会话关了，`.part` 留下了"。
    pub(crate) async fn stop_transfers(&mut self) {
        for transfer in &self.transfers {
            transfer.tracked.cancel.cancel();
        }
        // 等**全部**任务收工再返回（不是只等被取消的那些）：已经跑完的任务本来就已经
        // 结束了，等它们不需要时间；而"等来等去只等一部分"会让下面那句保证不成立。
        for transfer in &mut self.transfers {
            if let Some(task) = transfer.task.take() {
                // 忽略返回值：任务 panic 过一次不该让关会话变成失败（清理已经尽力）。
                let _ = task.await;
            }
        }
    }

    /// 回收：两侧的连接都**显式断开**（同 plan 0606 的纪律：drop 不能代替显式回收）。
    ///
    /// 传输在这里只**推信号 + 放弃任务句柄**：这是应用退出的路径，没有任何调用方
    /// 能等清理落地（`AGENTS.md` §3.3 的进程外兜底是看门狗，不是这里）。
    pub(crate) fn reclaim(mut self, runtime: Option<&RuntimeHandle>) {
        for transfer in &mut self.transfers {
            transfer.tracked.cancel.cancel();
            if let Some(task) = transfer.task.take() {
                task.abort();
            }
        }
        for side in &mut self.sides {
            if let Some(link) = side.link.take() {
                link.close(runtime);
            }
            side.state = SftpSideState::Disconnected;
        }
    }
}

/// 登记一个两栏 SFTP 会话。
///
/// **同步命令**：它只往注册表里放一个空实体（没有任何 I/O），连接是 [`sftp_connect`] 的事。
/// 收 `AppHandle` 只为读一次并发上限（ADR-0006 D6 的参数）—— 那个数在会话建立那一刻定下来，
/// 之后不随配置变（配置本身只在启动时读一次）。
#[tauri::command]
#[specta::specta]
pub fn sftp_open(
    app: AppHandle,
    sessions: State<'_, Sessions>,
) -> Result<SessionHandle, SftpError> {
    sessions
        .open_sftp(crate::config::in_flight(&app))
        .map_err(SftpError::from)
}

/// 让某一侧连上本机文件系统，或者池里的那一台主机。
///
/// 目标是主机时**先试 B 档**（plan 0703）：另一栏已经连上一台不同的主机 A 的话，先建
/// "本机 → A → 这一台"这条链；建不起来就回退本机直连（A 档），原因记在这一侧
/// （[`SftpSideInfo::through_failure`]）。哪一档成不成是**这一栏连接的结果**，
/// 于是"这次传输走的是哪一档"由端点怎么来的决定 —— 传输那个引擎一行都不用改（ADR-0006 D5）。
///
/// ⚠️ **async**：命令体里有两次会阻塞几秒的等待（握手 + 开子系统），而同步命令跑在
/// 处理 IPC 请求的那条线程上 —— 挡住它就等于挡住全部 IPC，包括用户回答问题要用的那三条
/// （同 `open_ssh_session` 的理由）。本机那一档没有等待，走同一条命令只是为了
/// "一栏只有一种连法"。
#[tauri::command]
#[specta::specta]
pub async fn sftp_connect(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    handle: SessionHandle,
    side: SftpSide,
    origin: SftpOrigin,
) -> Result<SftpSideInfo, SftpError> {
    // 上一次的连接（换主机时留下的那条）交出来在**锁外**收掉。
    if let Some(previous) = sessions.sftp_prepare_connect(handle, side, origin)? {
        previous.close(app.state::<Ssh>().runtime_handle().as_ref());
    }

    match origin {
        // 本机：没有连接、没有认证，**也没有会失败的地方** —— 直接把那一栏点亮，
        // 起点是用户的家目录（`LocalEndpoint::default_dir` 的三层兜底）。
        SftpOrigin::Local => {
            sessions.sftp_attach_local(
                handle,
                side,
                LOCAL_NAME.to_owned(),
                LocalEndpoint::default_dir(),
            )?;
        }
        SftpOrigin::Host { id } => {
            let ssh = app.state::<Ssh>();
            let vault = app.state::<Vault>();
            // 这一栏这次要经哪台直通（另一栏连上的那台），或者不试直通。
            let via = via_host(&sessions, handle, side, id);
            let attempt = match via {
                Some(route) => match connect_side_via(&ssh, &vault, route, id).await {
                    Ok(link) => Ok(link),
                    Err(err) => {
                        // 回退是**正常路径**（对端不认这条转发是常见配置），所以是 warn 而不是
                        // error —— 但它必须留下证据：用户要看出"原本想走直通、没走成"。
                        tracing::warn!(
                            via = route,
                            host = id,
                            reason = %err,
                            "sftp tunnel connect failed"
                        );
                        let _ = sessions.sftp_note_through_failure(handle, side, err.to_string());
                        connect_side(&ssh, &vault, id).await
                    }
                },
                None => connect_side(&ssh, &vault, id).await,
            };
            match attempt {
                Ok(link) => {
                    let SideLink {
                        name,
                        connection,
                        client,
                        through,
                    } = link;
                    sessions.sftp_attach(handle, side, name, connection, client, through)?;
                }
                Err(err) => {
                    // 失败**落在这一侧**：探针与界面因此都答得出"是左边还是右边没连上"。
                    // ⚠️ 忽略返回：会话可能在这次连接期间被用户关掉（`sftp_close`），
                    // 那时"哪一侧失败"这件事已经无处可记 —— 而真正要报的是下面这个 `Err`。
                    let _ = sessions.sftp_fail(handle, side, err.to_string());
                    return Err(err);
                }
            }
        }
    }

    sessions
        .sftp_side(handle, side)
        .ok_or(SftpError::NotAnSftp { handle })
}

/// 列某一侧某个目录。
#[tauri::command]
#[specta::specta]
pub async fn sftp_list(
    sessions: State<'_, Sessions>,
    handle: SessionHandle,
    side: SftpSide,
    path: String,
) -> Result<SftpListing, SftpError> {
    let endpoint = sessions
        .sftp_endpoint(handle, side)?
        .ok_or(SftpError::NotConnected {
            side: side.as_str().to_owned(),
        })?;
    // 从这里开始**不持会话表的锁**：`list` 会等在网络上，而别的命令照样要用那张表。
    let listing = endpoint.list(&path).await?;
    sessions.sftp_set_path(handle, side, listing.path.clone())?;
    Ok(listing.into())
}

/// 把某一侧的一个文件搬到另一侧的某个路径上（plan 0702 / 0703 / 0704）。
///
/// 返回一个**编号**而不是结果：搬运是后台任务（`scope.md` §4.1 要求 progress 可见，
/// 而一条几十秒的命令会把 IPC 堵住）。进度与结局走 [`sftp_transfers`] 与 `sftp` 探针读。
///
/// 两栏都是主机时走哪一档不在这里决定：那是**目标那一栏的端点怎么来的**（B 档 = 那条连接
/// 是经源那一栏的主机直通来的，见 [`sftp_connect`]），这里只把结果抄进这次传输的记录
/// （[`SftpTransfer::via`]）。于是两档共用同一个引擎（ADR-0006 D5）。
///
/// **并发上限在会话那一层**（plan 0704）：这条任务先在这个会话的空位上排一个队，再动端点。
/// 排队与"用户取消 / 关会话"是可抢占的 —— 排在队里就被取消的传输**一个端点都没碰过**。
#[tauri::command]
#[specta::specta]
pub fn sftp_transfer(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    handle: SessionHandle,
    from: SftpSide,
    from_path: String,
    to: SftpSide,
    to_path: String,
) -> Result<TransferId, SftpError> {
    let source = endpoint_of(&sessions, handle, from)?;
    let target = endpoint_of(&sessions, handle, to)?;
    // 目标那一栏是经哪台直通到达的 —— `None` = 本机直连，也就是本机内存中转那一档。
    let via = sessions.sftp_through(handle, to);
    let in_flight = sessions.sftp_in_flight(handle)?;

    let tracked = Arc::new(Tracked::new(from, from_path, to, to_path, via));
    let runtime = app
        .state::<Ssh>()
        .runtime_handle()
        .ok_or_else(|| SftpError::Internal {
            message: "SSH 的 runtime 还没起来，传输排不上去".to_owned(),
        })?;

    let task = {
        let tracked = Arc::clone(&tracked);
        runtime.spawn(async move {
            // 请求借用 `tracked` 里的两个路径 —— 所以它在**任务里面**构造，
            // 而不是在外面构造好再移进来（那样借的东西活不过 spawn）。
            let request = TransferRequest {
                source_path: tracked.from_path.as_str(),
                target_path: tracked.to_path.as_str(),
            };
            let outcome = in_flight
                .transfer(
                    &*source,
                    &*target,
                    &request,
                    &tracked.progress,
                    tracked.cancel.waiter(),
                )
                .await;
            tracked.finish(outcome);
        })
    };

    // 登记是**最后**一步：任务已经跑起来了，登记只是把它记进表里。
    // ⚠️ 会话可能在这期间被关掉 —— 那时实体已经摘牌，登记会失败；不能把这次传输丢下不管
    // （它已经在搬了），所以推一次中止让它自己收拾干净，再把失败报出去。
    match sessions.sftp_register_transfer(handle, tracked.clone(), task) {
        Ok(id) => Ok(id),
        Err(err) => {
            tracked.cancel.cancel();
            Err(SftpError::from(err))
        }
    }
}

/// 取消一次传输。
///
/// ⚠️ 它**只推信号就返回**：临时文件是引擎那条任务删的，而"删掉了"由任务自己写进状态 ——
/// 调用方要看的是 [`sftp_transfers`] 里那条不再是 `running`（`AGENTS.md` §7：
/// 等真正结束，而不是猜一段时间）。
#[tauri::command]
#[specta::specta]
pub fn sftp_transfer_cancel(
    sessions: State<'_, Sessions>,
    handle: SessionHandle,
    id: TransferId,
) -> Result<(), SftpError> {
    if sessions.sftp_cancel_transfer(handle, id)? {
        Ok(())
    } else {
        Err(SftpError::NoSuchTransfer { id })
    }
}

/// 这个会话发起过的传输（新的在前）。
#[tauri::command]
#[specta::specta]
pub fn sftp_transfers(
    sessions: State<'_, Sessions>,
    handle: SessionHandle,
) -> Result<Vec<SftpTransfer>, SftpError> {
    sessions.sftp_transfers(handle).map_err(SftpError::from)
}

/// 两侧的状态（只读命令用）。
#[tauri::command]
#[specta::specta]
pub fn sftp_sides(
    sessions: State<'_, Sessions>,
    handle: SessionHandle,
) -> Result<Vec<SftpSideInfo>, SftpError> {
    sessions.sftp_sides(handle).map_err(SftpError::from)
}

/// 列出后端已经登记的全部 SFTP 会话。
///
/// ⚠️ 它存在的理由是一条产品纪律：**关面板不等于结束会话**（`scope.md` §5.6 ——
/// 文件传输是仅渲染的视图）。面板必须能找回那个会话，否则"关前端不影响后端执行"
/// 就变成了"关前端等于失控"：会话还在，而没有任何地方能再操作它。
#[tauri::command]
#[specta::specta]
pub fn sftp_sessions(sessions: State<'_, Sessions>) -> Result<Vec<SftpSummary>, SftpError> {
    Ok(sessions.sftp_entries())
}

/// 关掉一个 SFTP 会话：两侧连接断开，会话从注册表摘掉。
///
/// ⚠️ **它是这个会话唯一的关闭入口**：面板是仅渲染的视图（`scope.md` §5.6），
/// 关面板不停后端会话 —— 停止是这里这个显式动作（与隧道那边 `tunnel_stop` 同一条纪律）。
/// 幂等：已经关过的句柄返回 `Ok`（不是失败）。
///
/// ⚠️ **顺序不能换**：先推中止、**等清理落地**，再断连接。临时文件是用那条连接删掉的，
/// 反过来做的话 `scope.md` §4.2 那一格（"关闭 `Session` → 删除临时文件"）永远做不到。
/// 等一下的上界是 SFTP 自己的请求期限（一次读或写、加上一次删除）—— 不会无限等。
#[tauri::command]
#[specta::specta]
pub async fn sftp_close(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    handle: SessionHandle,
) -> Result<(), SftpError> {
    if let Some(mut sftp) = sessions.remove_sftp(handle)? {
        sftp.stop_transfers().await;
        sftp.reclaim(app.state::<Ssh>().runtime_handle().as_ref());
    }
    Ok(())
}

/// 连接这一条路的结果：那一台的名字、连接本体、会话句柄，以及**它实际是怎么到达的**。
///
/// 它比元组多出来的正是 plan 0703 要看的那一件事：`through` 不是"想经哪台"，
/// 而是**真的经了哪台**（回退到本机直连时是 `None`）。
struct SideLink {
    name: String,
    connection: SshConnection,
    client: SftpClient,
    through: Option<HostId>,
}

/// 照池里的行建立一条连接，并在它上面开一个 SFTP 会话（本机直连这一档）。
///
/// ⚠️ 建连接走的是**与终端、隧道同一条路**（`connect_connection`，含跳板链与主机密钥校验）——
/// 三条路的差别只在连上之后开什么通道，而"照池里的行连过去要准备什么材料"只有一份实现。
async fn connect_side(ssh: &Ssh, vault: &Vault, host_id: HostId) -> Result<SideLink, SftpError> {
    let name = host_name(vault, host_id)?;
    // 取消信号传 `pending`：SFTP 的连接没有"被中途叫停"的动作（那是隧道关闭才需要的，
    // plan 0606）。用户要中止只能关掉这个会话 —— 那时命令已经返回了。
    let connection =
        crate::ssh::connect_connection(ssh, vault, host_id, std::future::pending()).await?;
    let client = connection.sftp().await.map_err(SftpError::from)?;
    Ok(SideLink {
        name,
        connection,
        client,
        through: None,
    })
}

/// 同上，但经 `via` 直通（plan 0703 的 B 档：本机 → `via` → `host_id`）。
///
/// 与 [`connect_side`] 只差建链那一步 —— 会话怎么开、失败怎么分类完全相同，
/// 因为那条链在外面看就是"一条到目标的连接"（`connect_connection_via` 的文档）。
async fn connect_side_via(
    ssh: &Ssh,
    vault: &Vault,
    via: HostId,
    host_id: HostId,
) -> Result<SideLink, SftpError> {
    let name = host_name(vault, host_id)?;
    let connection =
        crate::ssh::connect_connection_via(ssh, vault, via, host_id, std::future::pending())
            .await?;
    let client = connection.sftp().await.map_err(SftpError::from)?;
    Ok(SideLink {
        name,
        connection,
        client,
        through: Some(via),
    })
}

/// 这一栏这次要经哪台直通 —— `None` 就是不试直通。
///
/// 判据只有一条：**另一栏已经连上一台不同的主机**。为什么不看"另一栏在下拉框里选了什么"：
/// 那个选择要到 `sftp_connect` 才进后端，而"经那台直通"这句话成立的前提是那台**真的够得着**
/// —— "它已经连上了"就是现成的证据，不必另证一次。
///
/// 同一台主机不算：那条链是"连它、再从它连它"，白白多一跳。
fn via_host(
    sessions: &Sessions,
    handle: SessionHandle,
    side: SftpSide,
    id: HostId,
) -> Option<HostId> {
    let other = sessions.sftp_side(handle, side.other())?;
    if other.state != SftpSideState::Connected {
        return None;
    }
    match other.origin {
        Some(SftpOrigin::Host { id: other_id }) if other_id != id => Some(other_id),
        _ => None,
    }
}

/// 池里那一行的名字。
///
/// 单独读一次是因为**界面那两栏要有个标题**，而连接那条路只交回一台机器的地址
/// （`SshTarget`）—— 名字是**池**的概念，不是连接的概念。
fn host_name(vault: &Vault, host_id: HostId) -> Result<String, SftpError> {
    let row = vault
        .with_conn(|conn| akasha_store::pools::hosts::host(conn, i64::from(host_id)))
        .map_err(|err| match err {
            ConnError::Locked => SftpError::Locked,
            ConnError::Store(akasha_store::StoreError::NoSuchRow { .. }) => {
                SftpError::NoSuchHost { id: host_id }
            }
            other => SftpError::from(other),
        })?;
    Ok(row.name)
}

/// 本机那一栏在界面上的名字（后端给的默认名；用户可以不管它）。
const LOCAL_NAME: &str = "本机";

/// 取某一侧的端点，没连上就是 [`SftpError::NotConnected`]。
fn endpoint_of(
    sessions: &Sessions,
    handle: SessionHandle,
    side: SftpSide,
) -> Result<Box<dyn Endpoint>, SftpError> {
    sessions
        .sftp_endpoint(handle, side)?
        .ok_or(SftpError::NotConnected {
            side: side.as_str().to_owned(),
        })
}

/// 只读探针 `sftp`：全部 SFTP 会话、它们两侧的状态，以及各自发起过的传输
/// （`AGENTS.md` §7 的探针纪律）。
///
/// 为什么不是"塞进 `sessions` 探针"：那份说的是 `live` 与 `registered` **两个数必须相等**
/// 这条不变量，混进别的东西会让那句不变量失去意义（同 `tunnels` 与 `residue` 的分工）。
pub fn snapshot(sessions: &Sessions) -> serde_json::Value {
    serde_json::json!(sessions.sftp_entries())
}
