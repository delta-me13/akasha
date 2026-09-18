//! 建立一条连接：TCP → 握手（含主机密钥校验）→ 认证 → 开一个 shell 通道（ADR-0003 D3 / D11）。
//!
//! 这一层只做"**把连接弄起来**"，把 `Handle` 与通道两半交出去；上面的门面（字节怎么进出、
//! 收尾怎么做）在 [`crate::ssh::transport`]。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use akasha_pty::TerminalSize;
use russh::client::{self, Config, Handle, Msg};
use russh::keys::{HashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{Channel, ChannelReadHalf, ChannelWriteHalf};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::ssh::auth;
use crate::ssh::credential::{CredentialCache, CredentialProvider};
use crate::ssh::error::{SshError, channel_failed, connect_failed};
use crate::ssh::forward::{SshConnection, SshStream};
use crate::ssh::keys::SshAuth;
use crate::ssh::remote::{Inbound, Incoming};
use crate::ssh::target::SshTarget;

/// 服务端的主机密钥 —— **判定材料是密钥本体，指纹只给人看**（ADR-0003 D11）。
///
/// 单独一个类型而不是直接漏出 `russh` 的公钥类型：`russh` 是 0.x，
/// 而"我们要核对的东西"是稳定的（RFC 4253 的密钥本体与指纹）。
/// 上游那个公钥类型留在**私有字段**里 —— 对外只有 blob / 指纹 / 算法名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKey {
    algorithm: String,
    fingerprint: String,
    blob: Vec<u8>,
    /// 上游的公钥本体。`check_known_hosts_path` 要它（那样才认得哈希主机名那类形态），
    /// 所以留着而不是每次从 blob 重新解析。**不公开**：上层不该跟着 `russh` 的 0.x API 走。
    public: PublicKey,
}

impl HostKey {
    /// 密钥算法（`ssh-ed25519` 一类）。给人看；**参与判定的是类型对不对**，
    /// 因为"同一台主机的不同类型"不算同一条记录（D11 / `schema.rs` 的 `UNIQUE`）。
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    /// `SHA256:…` 指纹。这是用户能在服务器上核对的那串东西。**不参与判定**。
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// SSH 线格式的密钥本体 —— **判定的材料**（逐字节比）。
    pub fn blob(&self) -> &[u8] {
        &self.blob
    }

    /// 上游的公钥本体（内部用：`check_known_hosts_path` 与横向比较）。
    pub(crate) fn public_key(&self) -> &PublicKey {
        &self.public
    }

    /// 从上游的公钥形态取值。
    ///
    /// `Err` = 这把密钥**用不了**（编不出线格式本体 = 拿不到判定材料）——
    /// 调用方必须**拒绝**，而不是拿一段空字节顶替。
    pub(crate) fn from_public(key: &PublicKey) -> Result<Self, SshError> {
        let blob = key.to_bytes().map_err(|err| SshError::HostKeyUnusable {
            algorithm: key.algorithm().as_str().to_owned(),
            reason: err.to_string(),
        })?;
        Ok(Self {
            algorithm: key.algorithm().as_str().to_owned(),
            // SHA256 而不是 MD5：OpenSSH 的现代默认，且 MD5 指纹早已不该用于核对。
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            blob,
            public: key.clone(),
        })
    }
}

/// 主机密钥的**策略**。
///
/// 上游 `russh` 的默认实现**拒绝一切**，必须覆写（ADR-0003 D11 记的正是这一条）——
/// 所以这里不给"接受一切"的默认实现：调用方必须**明确**说出它认什么。
///
/// 本 plan 只提供 [`PinnedHostKey`]；`~/.ssh/known_hosts` 的只读校验与库内缓存
/// 是 plan 0505 的落点（那时的实现也是这个 trait 的另一个实现）。
pub trait HostKeyVerifier: Send + Sync {
    /// 认不认这把密钥。`Err` 一律**拒绝连接**（不静默接受、也不静默改写）。
    fn verify(&self, target: &SshTarget, key: &HostKey) -> Result<(), SshError>;
}

/// 只认一把钉住的密钥（指纹按 `SHA256:…` 比对）。
#[derive(Debug, Clone)]
pub struct PinnedHostKey {
    fingerprint: String,
}

impl PinnedHostKey {
    /// 钉住一个指纹。
    pub fn new(fingerprint: impl Into<String>) -> Self {
        Self {
            fingerprint: fingerprint.into(),
        }
    }
}

impl HostKeyVerifier for PinnedHostKey {
    fn verify(&self, _target: &SshTarget, key: &HostKey) -> Result<(), SshError> {
        if self.fingerprint == key.fingerprint() {
            Ok(())
        } else {
            Err(SshError::HostKeyRejected {
                fingerprint: key.fingerprint().to_owned(),
            })
        }
    }
}

/// 连接的取值。**默认值写在这里**，不散在代码各处（ADR-0003 §12 把"保活取值"挂给了本 plan）。
#[derive(Debug, Clone)]
pub struct SshConfig {
    /// TCP + 握手的期限。超了报 [`SshError::ConnectTimeout`]，不无限等。
    ///
    /// 10 秒是"够慢的网络也来得及、又不至于让用户以为卡死"的取值；
    /// 它与阶段 6 的重连退避（1s / 2s / 4s）是两件事：这一条管**一次**尝试活多久。
    pub connect_timeout: Duration,
    /// 保活间隔。上游默认 `None`（= 永不发保活），我们必须给一个值，
    /// 否则半死的连接（对端不响应、TCP 也没断）会一直挂着，而"挂着"的表现是
    /// 用户以为还连着。
    ///
    /// 30 秒 + 3 次 = 约 90 秒发现半死连接；对交互式终端，这个量级的延迟是能接受的，
    /// 而更短会让长连接设备上多出可观的空包。实测回填见 plan 0502 的实施记录。
    pub keepalive_interval: Option<Duration>,
    /// 连续多少个保活没有回应就判定连接死了。上游默认 3，照用。
    pub keepalive_max: usize,
    /// `TERM` 的值。
    pub term: String,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            term: "xterm-256color".to_owned(),
        }
    }
}

/// 一次连接需要的全部输入。
pub struct SshConnect {
    /// 连谁。
    pub target: SshTarget,
    /// 用哪些认证材料（D7 的顺序在 [`crate::ssh::auth`] 里）。
    pub auth: SshAuth,
    /// 凭据缓存（同一台主机的第二次连接靠它不再问）。
    pub cache: Arc<CredentialCache>,
    /// 没命中缓存时去哪问。
    pub provider: Arc<dyn CredentialProvider>,
    /// 认不认服务端的密钥。
    pub host_keys: Arc<dyn HostKeyVerifier>,
    /// 取值。
    pub config: SshConfig,
    /// 初始窗口尺寸（`request_pty` 要）。⚠️ **跳板那几跳用不到它**（它们不开 pty，只被借来
    /// 开通道），但类型不变 —— 一份 `SshConnect` 就是"连一台要的全部输入"，为跳板单独造一个
    /// 少一个字段的类型只会让调用方多写一处转换。
    pub size: TerminalSize,
}

/// 客户端回调：`russh` 唯一会回头看我们的地方。
///
/// 两件事：主机密钥（D11）与服务端发起的通道（D10 的 `-R`）。
/// 为什么入站通道的入口必须**与连接一起**被构造：那个回调在连接的消息循环里，
/// 而"通道交给哪条转发"这件事要等到连接建好、`tcpip_forward` 请求发出之后才知道 ——
/// 所以载体（[`Inbound`]）在握手时造好，一次交给 [`Handler`]，一次随 [`Authenticated`]
/// 交出去（plan 0604）。
pub(crate) struct Handler {
    target: SshTarget,
    verifier: Arc<dyn HostKeyVerifier>,
    /// 被拒的**原因**（有就说明为什么连不上）。
    ///
    /// 为什么要有这个格子：`check_server_key` 只能回一个 `bool`，而上游拿到 `false`
    /// 之后给的是一个笼统的"未知主机密钥"。可"为什么"正是用户要的东西 ——
    /// 指纹（要拿去核对）、是"没见过"还是"**变了**"（后者是警报），全靠它带出来。
    /// 所以这里存**整个错误**，而不是只存一个指纹字符串：三态在 `establish` 那边
    /// 原样浮现，不被压成一句话。
    rejection: Arc<Mutex<Option<SshError>>>,
    /// 服务端发起的 `forwarded-tcpip` 通道要交到哪（D10 / plan 0604）。
    inbound: Arc<Inbound>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let reject = |rejection: &Arc<Mutex<Option<SshError>>>, err: SshError| {
            *rejection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(err);
        };

        let key = match HostKey::from_public(&server_public_key.public_key()) {
            Ok(key) => key,
            Err(err) => {
                tracing::warn!(
                    host = self.target.host(),
                    port = self.target.port(),
                    %err,
                    "ssh host key unusable"
                );
                reject(&self.rejection, err);
                return Ok(false);
            }
        };

        match self.verifier.verify(&self.target, &key) {
            Ok(()) => {
                tracing::debug!(
                    host = self.target.host(),
                    port = self.target.port(),
                    algorithm = key.algorithm(),
                    fingerprint = key.fingerprint(),
                    "ssh host key accepted"
                );
                Ok(true)
            }
            Err(err) => {
                // **拒绝就是拒绝**：不重试、不改写用户的 known_hosts，把原因带出去。
                tracing::warn!(
                    host = self.target.host(),
                    port = self.target.port(),
                    fingerprint = key.fingerprint(),
                    %err,
                    "ssh host key rejected"
                );
                reject(&self.rejection, err);
                Ok(false)
            }
        }
    }

    /// 服务端发起了一条 `forwarded-tcpip` 通道（`-R`，RFC 4254 §7.2）。
    ///
    /// ⚠️ 这里**不做**任何 `await`：这个回调在连接的消息循环上被 `await`，
    /// 在这里等一条本机连接会让整条连接无响应（连保活都停）。接线（连本机服务、
    /// 接受或拒绝那条通道）全部在 [`crate::ssh::remote`] 的任务里做。
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.inbound.deliver(Incoming {
            channel,
            connected: (connected_address.to_owned(), connected_port),
            originator: (originator_address.to_owned(), originator_port),
            reply,
        });
        Ok(())
    }
}

/// 一条**已经认证过的**连接与它的入站入口（还没接上队列与线程）。
pub(crate) struct Authenticated {
    /// 控制用的句柄。**它一 drop，连接就结束**（上游的会话任务随之收工），
    /// 所以持有它的实体必须活到显式收尾为止。
    pub(crate) session: Handle<Handler>,
    /// 服务端发起的 `forwarded-tcpip` 通道的入口（D10）。[`Handler`] 里那一份是它的克隆。
    pub(crate) inbound: Arc<Inbound>,
}

/// 一条**已经认证过的**连接与它的 shell 通道（还没接上队列与线程）。
pub(crate) struct Established {
    /// 控制用的句柄。**它一 drop，连接就结束**（上游的会话任务随之收工），
    /// 所以门面必须持有它，直到显式收尾为止。
    pub(crate) session: Handle<Handler>,
    /// 通道的读半（`wait()` 拿 `ChannelMsg`：数据、`exit-status`、关闭）。
    pub(crate) read: ChannelReadHalf,
    /// 通道的写半（`data_bytes` / `window_change`）。
    pub(crate) write: ChannelWriteHalf<client::Msg>,
    /// 这条连接赖以存在的**下层连接**（跳板链，最外层在前）。
    ///
    /// 它们必须活到连接结束：承载我们的那条 `direct-tcpip` 通道长在**最后一条**上，
    /// 而 `Handle` 一 drop 那条连接就没了。所以它们跟着 `Established` 一起 move 进
    /// `pump` —— "task 结束 = 整条链结束"，收尾仍然只有一个出口（D9 / plan 0505）。
    pub(crate) carriers: Vec<SshConnection>,
}

/// 建一条到目标的 **TCP** 连接。超时是**我们的**，不是上游的。
pub(crate) async fn tcp_stream(options: &SshConnect) -> Result<TcpStream, SshError> {
    let target = &options.target;
    let stream = timeout(
        options.config.connect_timeout,
        TcpStream::connect(target.address()),
    )
    .await
    .map_err(|_| SshError::ConnectTimeout {
        target: target.clone(),
        after: options.config.connect_timeout,
    })?
    .map_err(|err| connect_failed(target, err))?;

    // ⚠️ **必须在这里显式关 Nagle**：上游只在 `client::connect` 里看 `Config::nodelay`
    // （`client/mod.rs:1089`），而 `connect_stream` —— 我们两条路都用它 —— **不看**。
    // 也就是说"把 config.nodelay 设成 true"在 `connect_stream` 下是一句**空话**，
    // 而 Nagle 会让"一次按键一个小包"的交互式输入攒着等确认（`docs/STATUS.md` 问题 #120）。
    if let Err(err) = stream.set_nodelay(true) {
        tracing::warn!(%err, "ssh nodelay failed");
    }
    Ok(stream)
}

/// 一跳的**握手 + 认证**：底层流由调用方给。
///
/// 抽出来的理由就是 `direct-tcpip`（plan 0505）：跳板那条路上，第二跳的"网络"是第一跳上的
/// 一条通道，而它之后做的事（配 config、把被拒原因换回来、认证）与直连**一字不差**。
/// 两种底层流因此走同一条路 —— `client::connect_stream` 只要求 `AsyncRead + AsyncWrite`。
pub(crate) async fn handshake<S>(
    options: &mut SshConnect,
    stream: S,
) -> Result<Authenticated, SshError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let target = options.target.clone();

    tracing::debug!(
        host = target.host(),
        port = target.port(),
        user = target.user(),
        "ssh connecting"
    );

    let mut config = Config::default();
    config.keepalive_interval = options.config.keepalive_interval;
    config.keepalive_max = options.config.keepalive_max;
    let config = Arc::new(config);

    let rejection = Arc::new(Mutex::new(None));
    // 入站通道的入口在这里造：它必须与连接一起被构造（见 [`Handler`]）。
    let inbound = Arc::new(Inbound::default());
    let handler = Handler {
        target: target.clone(),
        verifier: Arc::clone(&options.host_keys),
        rejection: Arc::clone(&rejection),
        inbound: Arc::clone(&inbound),
    };
    let mut session = match client::connect_stream(config, stream, handler).await {
        Ok(session) => session,
        Err(err) => {
            // 主机密钥被拒时上游只会说"未知的主机密钥" —— 把**原因**换回来：
            // 指纹（用户唯一能拿去核对的东西），以及是"没见过"还是"**变了**"。
            let rejected = rejection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            return Err(rejected.unwrap_or_else(|| connect_failed(&target, err)));
        }
    };

    auth::authenticate(&mut session, options).await?;
    Ok(Authenticated { session, inbound })
}

/// 连上（直连或经跳板）、认证、开一个带 pty 的 shell 通道。
///
/// `under` 是"这条连接跑在哪条流上"：`None` = 自己建 TCP（直连），`Some` = 上一跳上的一条
/// `direct-tcpip` 通道；`carriers` 是那一条通道赖以存在的下层连接，原样带进 [`Established`]。
pub(crate) async fn establish(
    options: &mut SshConnect,
    under: Option<SshStream>,
    carriers: Vec<SshConnection>,
) -> Result<Established, SshError> {
    let target = options.target.clone();

    let authenticated = match under {
        Some(stream) => handshake(options, stream).await?,
        None => {
            let stream = tcp_stream(options).await?;
            handshake(options, stream).await?
        }
    };
    let session = authenticated.session;

    let channel = session
        .channel_open_session()
        .await
        .map_err(channel_failed)?;
    let size = options.size;
    channel
        .request_pty(
            true,
            &options.config.term,
            u32::from(size.cols),
            u32::from(size.rows),
            0,
            0,
            &[],
        )
        .await
        .map_err(channel_failed)?;
    channel.request_shell(true).await.map_err(channel_failed)?;

    let (read, write) = channel.split();
    tracing::debug!(
        host = target.host(),
        port = target.port(),
        "ssh shell ready"
    );
    Ok(Established {
        session,
        read,
        write,
        carriers,
    })
}
