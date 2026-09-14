//! **SFTP 实体、四条命令与两侧连接**（plan 0701）—— 双栏会话的资源模型落地处。
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
//! | 过 IPC 的形状与 probe | 本模块（`sftp` 探针） |
//!
//! ## 两条边界
//!
//! 1. **`side` 只是"哪一栏"**（ADR-0006 D7）：它是唯一进入契约的呈现概念，后端不给它
//!    别的含义 —— 资源归那个 `Session`，不归某一侧。所以它只出现在命令参数与探针里，
//!    不出现在任何"归还 / 回收"的判断中。
//! 2. **一侧失败不影响另一侧**：两侧各持各的连接，失败落在**那一侧**
//!    （[`SftpSideState::Failed`] 与 [SftpSideInfo::failure]），另一侧照样可用 ——
//!    这正是"两侧独立"在可断言形式下的样子。
//!
//! ## 本阶段只做**读**
//!
//! 判据是"两侧各自列目录成功"，所以命令只有开 / 连 / 列 / 关四条。
//! 上传下载属于 plan 0702（它要在**端点**那一层定形状，ADR-0006 D4）。

use akasha_core::SessionId;
use akasha_ssh::{SftpClient, SshConnection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tokio::runtime::Handle as RuntimeHandle;

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

    /// 两侧都要走一遍的地方用它。
    pub const ALL: [SftpSide; 2] = [SftpSide::Left, SftpSide::Right];
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
    /// 池里那一台的行 id（还没选就是 `None`）。
    pub host_id: Option<HostId>,
    /// 那一台的名字（界面上那一栏的标题；还没选就是空串）。
    pub name: String,
    pub state: SftpSideState,
    /// 上一次失败的原因（`state = failed` 时才有）。**字段值不虚构**：
    /// 没有失败就不写它（`None`），不填一句 "unknown"（`docs/logging.md` 的口径）。
    pub failure: Option<String>,
    /// 当前目录（连上之后才有）。
    pub path: Option<String>,
}

/// 一个 SFTP 会话的**过 IPC 表示**（只读命令与探针共用）。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpSummary {
    pub handle: SessionHandle,
    /// 两侧，**永远两项**且顺序固定（`left` 在前）—— 前端因此不必处理"缺了一侧"。
    pub sides: Vec<SftpSideInfo>,
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

impl From<akasha_ssh::SftpKind> for SftpEntryKind {
    fn from(kind: akasha_ssh::SftpKind) -> Self {
        match kind {
            akasha_ssh::SftpKind::File => Self::File,
            akasha_ssh::SftpKind::Directory => Self::Directory,
            akasha_ssh::SftpKind::Symlink => Self::Symlink,
            akasha_ssh::SftpKind::Other => Self::Other,
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
/// `path` 是**服务端规范化之后**的路径（`realpath`）：前端拿它当"当前目录"，
/// 于是"返回上一级"不必在前端拼字符串。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpListing {
    pub path: String,
    pub entries: Vec<SftpEntry>,
}

impl From<akasha_ssh::SftpListing> for SftpListing {
    fn from(listing: akasha_ssh::SftpListing) -> Self {
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
    #[error("SFTP 的 {side} 这一侧还没有连接（先连接，再列目录）")]
    NotConnected { side: String },

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
/// 两者同生共死：会话跑在这条连接的通道上（ADR-0006 D2），连接没了会话也就没了。
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
        // 先放掉会话句柄：通道随那条流关闭，服务端因此先看到 SFTP 那一头收工，
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
    host_id: Option<HostId>,
    name: String,
    state: SftpSideState,
    failure: Option<String>,
    path: Option<String>,
    link: Option<SftpLink>,
}

impl Default for Side {
    fn default() -> Self {
        Self {
            host_id: None,
            name: String::new(),
            state: SftpSideState::Disconnected,
            failure: None,
            path: None,
            link: None,
        }
    }
}

impl Side {
    fn info(&self, side: SftpSide) -> SftpSideInfo {
        SftpSideInfo {
            side,
            host_id: self.host_id,
            name: self.name.clone(),
            state: self.state,
            failure: self.failure.clone(),
            path: self.path.clone(),
        }
    }
}

/// 一个 SFTP 会话：**两栏各自一条连接**（ADR-0006 D3）。
pub struct Sftp {
    id: SessionId,
    sides: [Side; 2],
}

impl Sftp {
    /// 新建一个两侧都还没连的会话。
    pub(crate) fn new(id: SessionId) -> Self {
        Self {
            id,
            sides: [Side::default(), Side::default()],
        }
    }

    /// 注册表里的名字（摘牌用）。
    pub(crate) fn id(&self) -> SessionId {
        self.id
    }

    pub(crate) fn sides(&self) -> Vec<SftpSideInfo> {
        SftpSide::ALL
            .iter()
            .map(|side| self.sides[side.index()].info(*side))
            .collect()
    }

    /// 这一侧要开始连了：记住它选的是哪台，并把状态推到 `连接中`。
    ///
    /// ⚠️ **先把上一次的连接交出去**（返回给调用方在锁外收掉）：换一台主机等于放弃
    /// 上一条连接，两条连接同时挂着会漏掉一条 —— 而它没有主人，也就没人回收。
    pub(crate) fn prepare_connect(&mut self, side: SftpSide, host_id: HostId) -> Option<SftpLink> {
        let slot = &mut self.sides[side.index()];
        let previous = slot.link.take();
        slot.host_id = Some(host_id);
        slot.state = SftpSideState::Connecting;
        slot.failure = None;
        slot.path = None;
        previous
    }

    /// 这一侧连上了：把名字与连接挂上。
    pub(crate) fn attach(
        &mut self,
        side: SftpSide,
        name: String,
        connection: SshConnection,
        client: SftpClient,
    ) {
        let slot = &mut self.sides[side.index()];
        slot.name = name;
        slot.state = SftpSideState::Connected;
        slot.failure = None;
        slot.link = Some(SftpLink { connection, client });
    }

    /// 这一侧失败了。**只落这一侧** —— 另一侧照样可用（ADR-0006 D3）。
    pub(crate) fn fail(&mut self, side: SftpSide, reason: String) {
        let slot = &mut self.sides[side.index()];
        slot.state = SftpSideState::Failed;
        slot.failure = Some(reason);
        slot.link = None;
    }

    /// 这一侧的会话句柄（列目录用）。
    ///
    /// 返回的是**克隆**（上游内部是 `Arc`）：命令拿到它就可以放掉会话表的锁再去 `await`，
    /// 而 `await` 期间别的命令照样能用这张表。
    pub(crate) fn client(&self, side: SftpSide) -> Option<SftpClient> {
        self.sides[side.index()]
            .link
            .as_ref()
            .map(|link| link.client.clone())
    }

    /// 记下这一侧当前在哪个目录（列目录成功后调用）。
    pub(crate) fn set_path(&mut self, side: SftpSide, path: String) {
        self.sides[side.index()].path = Some(path);
    }

    /// 回收：两侧的连接都**显式断开**（同 plan 0606 的纪律：drop 不能代替显式回收）。
    pub(crate) fn reclaim(mut self, runtime: Option<&RuntimeHandle>) {
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
#[tauri::command]
#[specta::specta]
pub fn sftp_open(sessions: State<'_, Sessions>) -> Result<SessionHandle, SftpError> {
    sessions.open_sftp().map_err(SftpError::from)
}

/// 让某一侧连上池里的那一台主机。
///
/// ⚠️ **async**：命令体里有两次会阻塞几秒的等待（握手 + 开子系统），而同步命令跑在
/// 处理 IPC 请求的那条线程上 —— 挡住它就等于挡住全部 IPC，包括用户回答问题要用的那三条
/// （同 `open_ssh_session` 的理由）。
#[tauri::command]
#[specta::specta]
pub async fn sftp_connect(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    handle: SessionHandle,
    side: SftpSide,
    host_id: HostId,
) -> Result<SftpSideInfo, SftpError> {
    // 上一次的连接（换主机时留下的那条）交出来在**锁外**收掉。
    if let Some(previous) = sessions.sftp_prepare_connect(handle, side, host_id)? {
        previous.close(app.state::<Ssh>().runtime_handle().as_ref());
    }

    let ssh = app.state::<Ssh>();
    let vault = app.state::<Vault>();
    match connect_side(&ssh, &vault, host_id).await {
        Ok((name, connection, client)) => {
            sessions.sftp_attach(handle, side, name, connection, client)?;
            sessions
                .sftp_side(handle, side)
                .ok_or(SftpError::NotAnSftp { handle })
        }
        Err(err) => {
            // 失败**落在这一侧**：探针与界面因此都答得出"是左边还是右边没连上"。
            // ⚠️ 忽略返回：会话可能在这次连接期间被用户关掉（`sftp_close`），
            // 那时"哪一侧失败"这件事已经无处可记 —— 而真正要报的是下面这个 `Err`。
            let _ = sessions.sftp_fail(handle, side, err.to_string());
            Err(err)
        }
    }
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
    let client = sessions
        .sftp_client(handle, side)?
        .ok_or(SftpError::NotConnected {
            side: side.as_str().to_owned(),
        })?;
    // 从这里开始**不持会话表的锁**：`list` 会等在网络上，而别的命令照样要用那张表。
    let listing = client.list(&path).await.map_err(SftpError::from)?;
    sessions.sftp_set_path(handle, side, listing.path.clone())?;
    Ok(listing.into())
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
#[tauri::command]
#[specta::specta]
pub fn sftp_close(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    handle: SessionHandle,
) -> Result<(), SftpError> {
    if let Some(sftp) = sessions.remove_sftp(handle)? {
        sftp.reclaim(app.state::<Ssh>().runtime_handle().as_ref());
    }
    Ok(())
}

/// 照池里的行建立一条连接，并在它上面开一个 SFTP 会话。
///
/// 返回三样：那一台的名字（界面上那一栏的标题）、连接本体、会话句柄。
///
/// ⚠️ 建连接走的是**与终端、隧道同一条路**（`connect_connection`，含跳板链与主机密钥校验）——
/// 三条路的差别只在连上之后开什么通道，而"照池里的行连过去要准备什么材料"只有一份实现。
async fn connect_side(
    ssh: &Ssh,
    vault: &Vault,
    host_id: HostId,
) -> Result<(String, SshConnection, SftpClient), SftpError> {
    let name = host_name(vault, host_id)?;
    // 取消信号传 `pending`：SFTP 的连接没有"被中途叫停"的动作（那是隧道关闭才需要的，
    // plan 0606）。用户要中止只能关掉这个会话 —— 那时命令已经返回了。
    let connection =
        crate::ssh::connect_connection(ssh, vault, host_id, std::future::pending()).await?;
    let client = connection.sftp().await.map_err(SftpError::from)?;
    Ok((name, connection, client))
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

/// 只读探针 `sftp`：全部 SFTP 会话与它们两侧的状态（`AGENTS.md` §7 的探针纪律）。
///
/// 为什么不是"塞进 `sessions` 探针"：那份说的是 `live` 与 `registered` **两个数必须相等**
/// 这条不变量，混进别的东西会让那句不变量失去意义（同 `tunnels` 与 `residue` 的分工）。
pub fn snapshot(sessions: &Sessions) -> serde_json::Value {
    serde_json::json!(sessions.sftp_entries())
}
