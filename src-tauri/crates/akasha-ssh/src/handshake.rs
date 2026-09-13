//! 建立一条连接：TCP → 握手（含主机密钥校验）→ 认证 → 开一个 shell 通道（ADR-0003 D3 / D11）。
//!
//! 这一层只做"**把连接弄起来**"，把 `Handle` 与通道两半交出去；上面的门面（字节怎么进出、
//! 收尾怎么做）在 [`crate::transport`]。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use akasha_pty::TerminalSize;
use russh::client::{self, Config, Handle};
use russh::keys::{HashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelReadHalf, ChannelWriteHalf};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::auth;
use crate::credential::{CredentialCache, CredentialProvider};
use crate::error::{SshError, channel_failed, connect_failed};
use crate::keys::SshAuth;
use crate::target::SshTarget;

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
    /// 用哪些认证材料（D7 的顺序在 [`crate::auth`] 里）。
    pub auth: SshAuth,
    /// 凭据缓存（同一台主机的第二次连接靠它不再问）。
    pub cache: Arc<CredentialCache>,
    /// 没命中缓存时去哪问。
    pub provider: Arc<dyn CredentialProvider>,
    /// 认不认服务端的密钥。
    pub host_keys: Arc<dyn HostKeyVerifier>,
    /// 取值。
    pub config: SshConfig,
    /// 初始窗口尺寸（`request_pty` 要）。
    pub size: TerminalSize,
}

/// 客户端回调：`russh` 唯一会回头看我们的地方。
///
/// 现在只有主机密钥一件事；阶段 6 的 `-R` 会往这里加入站通道的入口（ADR-0003 D10）——
/// 那时它必须能拿到"回到 Session"的一条通道，这也是它必须与连接一起被构造的原因。
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
}

/// 连上、认证、开一个带 pty 的 shell 通道。
pub(crate) async fn establish(options: &mut SshConnect) -> Result<Established, SshError> {
    // 目标的**副本**：下面要可变借 `options`（认证要读受保护页），
    // 而日志与错误消息整条路都要用目标。一个目标的克隆换掉一整串借用冲突，值。
    let target = options.target.clone();

    tracing::debug!(
        host = target.host(),
        port = target.port(),
        user = target.user(),
        "ssh connecting"
    );

    // 自己建 TcpStream 而不是用 `client::connect`：这样超时是**我们的**，
    // 而且以后接跳板时这里换成"任意流"就行（ADR-0003 D9 已经把 `connect_stream` 的用法
    // 定成"一条 `AsyncRead + AsyncWrite`"，跳板正是拿 channel 当这条流）。
    let stream = timeout(
        options.config.connect_timeout,
        TcpStream::connect(target.address()),
    )
    .await
    .map_err(|_| SshError::ConnectTimeout {
        target: target.clone(),
        after: options.config.connect_timeout,
    })?
    .map_err(|err| connect_failed(&target, err))?;

    let mut config = Config::default();
    config.keepalive_interval = options.config.keepalive_interval;
    config.keepalive_max = options.config.keepalive_max;
    // 交互式终端要关掉 Nagle：它会把小包攒起来等确认，而用户敲一个键就是一个小包
    // （上游默认 `false`，所以这一行是**我们的**选择，不是继承来的）。
    config.nodelay = true;
    let config = Arc::new(config);

    let rejection = Arc::new(Mutex::new(None));
    let handler = Handler {
        target: target.clone(),
        verifier: Arc::clone(&options.host_keys),
        rejection: Arc::clone(&rejection),
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
    })
}
