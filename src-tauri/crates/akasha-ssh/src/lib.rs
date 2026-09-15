//! akasha-ssh —— **SSH 客户端**：连接、认证，以及给会话层用的**同步 `Transport` 门面**。
//!
//! 这个 crate 是 [ADR-0003](../../../docs/adr/0003-ssh-stack-and-resource-model.md) 的落地。
//! 形状（`Transport` 映射、连接所有权、凭据缓存、认证顺序）都在那份 ADR 里定死了，
//! 这里只实现它，并把它**没写死**的几处（保活取值、`nodelay`、超时）在下面的
//! [`SshConfig`] 里给出默认值。
//!
//! ## 三件事，与它们各自的理由
//!
//! 1. **纯 Rust，不调系统 `ssh`**（`scope.md` §2.1）：握手、认证、终端通道全在
//!    [`russh`] 上，进程里没有 `ssh` 子进程。
//! 2. **库不自建 runtime**（ADR D2）：入口收 [`tokio::runtime::Handle`]，
//!    由 app 在启动时建**一个**专用 runtime。库偷偷建 runtime 的代价是
//!    "谁 drop、几个 worker、什么时候停"都变成库的隐式行为。
//! 3. **同步门面，异步在门后面**（ADR D3）：对外是 [`akasha_pty::Transport`]，
//!    SSH 因此**不改会话层、不改前端** —— 这正是"用 capability flag 而不是新 trait"的兑现。
//!
//! ## 凭据从哪来（ADR D7 / D8）
//!
//! 认证按 **agent → 密钥池 → keyboard-interactive → password** 的顺序尝试，
//! 每一档只在前一档失败之后才执行；服务端说"这一档过了、还要再来一种"
//! （`partial_success`）时**续接**而不是从头重来。
//!
//! "同一台主机开三个 Session 会被问三次"是不复用连接的已知副作用（`scope.md` §2.2），
//! 对策是 [`CredentialCache`]：键是 `(host, port, user, 认证方式)`，值是**口令**
//! （不是私钥）住在一页受保护内存里。**绝不落盘**，也**没有 TTL** ——
//! 超时后自动忘会让挂着的隧道在重连时突然弹问。
//!
//! ## 一句话说清边界
//!
//! 明文私钥在握手期间会短暂存在于普通堆：`russh` 要一个
//! [`ssh_key::PrivateKey`](russh::keys::PrivateKey) 才能签名，而它解析出来的明文
//! **我们放不进受保护页**。缓解 = 只在握手窗口内存在、签完即 drop、**不进缓存**。
//! 登录口令同理有一份 `String`（`russh` 的认证接口只收 `impl Into<String>`）。这两条副本
//! 照实记在 ADR-0003 D8 与 `docs/STATUS.md`，不假装它们不存在。
//!
//! ## 两个门面，一条是异步的
//!
//! [`SshTransport`] 是**同步**门面（装进 `akasha_pty::Transport`，给会话层用）；
//! [`SshConnection`] + [`SshStream`] 是**异步**那一层 —— `direct-tcpip` 原语（D9）住在那里，
//! 跳板 / `-L` / SFTP 的 B 档都按"[一条流](SshStream)"消费它。谁用哪一层、为什么，
//! 见 [`crate::SshConnection`] 的文档。

mod auth;
mod credential;
/// **一次转发怎么结束的**：`ForwardEnd` 与它的通知端（plan 0605 的重连循环的输入）。
mod ending;
mod error;
/// **D9 的原语**：`direct-tcpip` = 一条流（跳板 / `-L` / SFTP B 档复用）。
mod forward;
mod handshake;
mod keys;
mod known_hosts;
/// **本机端点**（plan 0702）：`scope.md` §4 的 "local ↔ host" 里的那个 local ——
/// 与一个 SFTP 会话并列的第二种 [`Endpoint`]。
mod local;
/// **本地转发 `-L` 与动态转发 `-D`**：本地监听 + 每条入站连接一条 `direct-tcpip` 通道
/// （plan 0602 / 0603）。
mod relay;
/// **远程转发 `-R`**：请服务端监听，把服务端发起的 `forwarded-tcpip` 通道接到本机服务
/// （plan 0604）—— 与 `-L` / `-D` 方向相反，因此不复用 `direct-tcpip` 原语（D10）。
mod remote;
/// **SFTP 会话**（plan 0701）：跑在一条流上的远端文件操作（ADR-0006 D2）。
mod sftp;
/// **动态转发 `-D` 的协议本体**：SOCKS5 的无认证 `CONNECT`（plan 0603）。
mod socks5;
mod target;
/// **测试脚手架**：进程内的 SSH 服务端（判据的另一半观察点）。
///
/// ⚠️ 生产代码不要用它 —— 它开监听端口、接受任何带对口令的连接。理由与用法见模块文档。
pub mod testing;
/// **传输引擎**（plan 0702，ADR-0006 D4）：它只认两个 [`Endpoint`]，
/// 于是"哪两个端点配对"就是三种拓扑的全部差别（D5）。
pub mod transfer;
mod transport;

pub use akasha_pty::{TerminalSize, Transport, TransportError};
pub use credential::{
    CacheKey, Credential, CredentialCache, CredentialKind, CredentialProvider, CredentialRequest,
    MAX_CREDENTIAL_LEN,
};
pub use ending::{ForwardEnd, ForwardEnding};
pub use error::{ForwardFailure, SshError};
pub use forward::{SshConnection, SshStream, live_connections};
pub use handshake::{HostKey, HostKeyVerifier, PinnedHostKey, SshConfig, SshConnect};
pub use keys::{KeyCandidate, SshAuth};
pub use known_hosts::{
    HostKeyCache, HostKeyPrompt, KnownHostsVerifier, RecordedHostKey, RecordedIn,
    user_known_hosts_file, user_ssh_config_file,
};
pub use local::LocalEndpoint;
pub use relay::{ForwardTarget, Ingress, LocalForward, LocalListener};
pub use remote::RemoteForward;
pub use sftp::SftpClient;
pub use target::SshTarget;
pub use transfer::{
    Cancel, CancelWaiter, Endpoint, Entry, EntryKind, Listing, PendingWrite, Progress,
    TransferRequest,
};
pub use transport::SshTransport;
