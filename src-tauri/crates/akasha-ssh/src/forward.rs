//! **D9 的原语**：`direct-tcpip` —— 一条流，三处复用（ADR-0003 §7）。
//!
//! `scope.md` §2.2 把这件事定成"**只实现一次**"，因为三处消费者要的是同一个东西：
//! **跳板 / ProxyJump**（本阶段）、**SSH 本地转发 `-L`**（阶段 6）、**SFTP host↔host 的 B 档**
//! （阶段 7）。形状因此不是"一个能开通道的函数"，而是**一条 `AsyncRead + AsyncWrite` 的流** ——
//! 跳板把这条流交给 `client::connect_stream` 当下一跳的底层，`-L` 把它接到本地监听 socket 上，
//! SFTP 在它上面跑数据面。三处都只是这条流的消费者。
//!
//! ## `Handle` 归谁（0504 留下的那个问题）
//!
//! [`SshConnection`] **自己持有** `Handle`，所以句柄不出这个 crate，也不需要 [`crate::SshTransport`]
//! 开一条"受控借用口"（借用口把"谁在什么时候能碰这个句柄"变成**可以问错**的问题）。
//! 跳板那条路上，每一跳都是一个 `SshConnection`；它们被带进最终那条连接的 `pump` task 里
//! （见 `handshake::Established::carriers`），于是"task 结束 = 整条链结束"。
//!
//! ## 为什么这一层只有异步形态
//!
//! 三个消费者**都在 runtime 里**：跳板链在 `SshTransport::connect_via` 的 `block_on` 之内，
//! 转发任务与 SFTP 是长驻 task。同步门面（`SshTransport::connect`）留给 app 的命令边界 ——
//! 那里没有 runtime 上下文，而 `Handle::block_on` 在 tokio 上下文里会 panic（D3）。

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use russh::client::{self, Handle};
use russh::{ChannelStream, Disconnect};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::runtime::Handle as RuntimeHandle;

use crate::error::{SshError, forward_failed, remote_listen_failed};
use crate::handshake::{self, Handler, SshConnect};
use crate::remote::Inbound;
use crate::sftp::SftpClient;
use crate::target::{SshTarget, host_and_port};

/// `direct-tcpip` 通道的客户端一侧 —— **就是 D9 说的那条流**。
///
/// 为什么是新类型而不是把 `russh::ChannelStream` 直接漏进签名：`russh` 是 0.x，而"我们对外
/// 承诺的"是 `AsyncRead + AsyncWrite`（RFC 4254 §7.2 的通道语义，稳定）。与 [`crate::HostKey`]
/// 把上游公钥类型留在私有字段里是同一条纪律 —— 上游换 API 时，改动止步于这里。
pub struct SshStream {
    inner: ChannelStream<client::Msg>,
}

impl SshStream {
    fn new(inner: ChannelStream<client::Msg>) -> Self {
        Self { inner }
    }
}

impl AsyncRead for SshStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for SshStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// `direct-tcpip` 要带上的"发起方"地址（RFC 4254 §7.2 的 `originator address/port`）。
///
/// 我们唯一**真知道**的是最外层那条 TCP 连接的本地地址；往下每一跳都复用它（跳板链上
/// "我们"始终是同一个客户端）。拿不到就报"不知道"：一个看着像真的、其实错的地址会进对端的
/// 日志与审计，比留空更坏。
#[derive(Debug, Clone)]
struct Originator {
    address: String,
    port: u32,
}

impl Originator {
    fn of(stream: &TcpStream) -> Self {
        match stream.local_addr() {
            Ok(addr) => Self {
                address: addr.ip().to_string(),
                port: u32::from(addr.port()),
            },
            Err(_) => Self {
                address: String::new(),
                port: 0,
            },
        }
    }
}

/// 进程里活着的 [`SshConnection`] 个数（plan 0606）。
///
/// 为什么要数：判据"关闭 `Session` 之后**连接数**归零"要能被断言，而实体表**不能**充当证据
/// —— 关闭命令自己就会把实体摘掉，"表里没了"只是那条命令的效果。连接对象归**转发任务**持有
/// （与实体表无关），所以它还活着就说明连接还活着。
///
/// 用 RAII 计数而不是"登记 + 注销两张表"：增减只发生在构造与析构上，**不可能与事实分叉**，
/// 也就不需要谁记得去注销。
static LIVE_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

/// **还活着的凭据**：一条 `SshConnection` 持有它，对象析构即减一。
struct LiveConnection;

impl LiveConnection {
    fn new() -> Self {
        LIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for LiveConnection {
    fn drop(&mut self) {
        LIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// 进程里活着的 SSH 连接数（[`SshConnection`] 的个数）。
///
/// 数的是**已认证的连接**（D9 那个类型的定义）。一次还在握手中的尝试不算 —— 它还没有一个
/// `SshConnection` 可言，而它的 socket 归那次尝试的 future 持有，由发起方的停止入口收掉
/// （app 侧见 `tunnel_open` / `tunnel_retry` 的 `select!`）。
pub fn live_connections() -> usize {
    LIVE_CONNECTIONS.load(Ordering::Relaxed)
}

/// 一条**已认证、没有通道**的 SSH 连接。**持有它就是让这条连接活着**（`Handle` 一 drop，
/// 上游的会话任务随之收工）。
///
/// 为什么单独一个类型：跳板链上的每一跳、以及阶段 6/7 的转发会话，要的都是"一条连接"
/// 本身，而不是"一个带 pty 的 shell 终端"（那是 [`crate::SshTransport`]）。
pub struct SshConnection {
    /// 存活凭据（[`live_connections`] 的计数来源）。**没有别的地方读它** —— 它的作用就是
    /// 与这条连接共生共死：名字前的下划线说的正是这件事，不是"暂时没用"。
    _live: LiveConnection,
    session: Handle<Handler>,
    target: SshTarget,
    originator: Originator,
    /// 承载这条连接的**下层链**（空 = 直连）。最外层在前，紧挨目标的那个在最后。
    ///
    /// 为什么挂在这里而不是由调用方另存一个 `Vec`：链的存活不是一个可以"记得"的东西
    /// —— 中间任何一跳 drop 掉，它上面那条 `direct-tcpip` 通道就跟着消失，而我们手上
    /// 这条连接的"网络"正是那条通道。挂进来之后，**谁活着这条链就活着**，
    /// 而收尾的顺序（最内层先断）也由 [`SshConnection::disconnect`] 一处定死。
    under: Vec<SshConnection>,
    /// 服务端发起的 `forwarded-tcpip` 通道要交到哪（ADR-0003 D10，plan 0604）。
    ///
    /// 与 `session` **同源**：两者都是握手时造出来的（那个回调在连接的消息循环里，
    /// 所以这个入口必须与连接一起构造）。挂在这里而不是由调用方另存，理由同 `under`：
    /// 谁持有连接谁就能收到入站通道，不需要谁记得配对。
    inbound: Arc<Inbound>,
}

/// 把 `hops` 逐跳搭起来（`[最外层, …, 紧挨目标的那个]`）。
///
/// 抽成函数的理由只有一个：**建链只有一份实现**。`SshTransport::connect_via` 与
/// [`SshConnection::connect_via`] 的差别在终点（那边还要开一个 shell 通道），
/// 而"逐跳搭链"这一段完全相同 —— 抄第二份的下场是其中一条慢慢长歪。
pub(crate) async fn hops_chain(hops: Vec<SshConnect>) -> Result<Vec<SshConnection>, SshError> {
    let mut under: Vec<SshConnection> = Vec::new();
    for mut hop in hops {
        let connection = match under.last() {
            // 第一跳：自己建 TCP。
            None => SshConnection::connect(&mut hop).await?,
            // 之后的每一跳：在上一跳上开一条 `direct-tcpip` 通道，**它就是这一跳的网络**。
            Some(previous) => previous.over(&mut hop).await?,
        };
        under.push(connection);
    }
    Ok(under)
}

impl SshConnection {
    /// 直连：自己建 TCP，然后握手 + 认证。
    pub async fn connect(options: &mut SshConnect) -> Result<Self, SshError> {
        let stream = handshake::tcp_stream(options).await?;
        let originator = Originator::of(&stream);
        let authenticated = handshake::handshake(options, stream).await?;
        Ok(Self {
            _live: LiveConnection::new(),
            session: authenticated.session,
            inbound: authenticated.inbound,
            target: options.target.clone(),
            originator,
            under: Vec::new(),
        })
    }

    /// **经这条连接**到 `options` 描述的那台：在它上面开一条 `direct-tcpip` 通道，
    /// 在那条通道上握手 + 认证。于是这条连接成了下一跳的"下层"。
    pub async fn over(&self, options: &mut SshConnect) -> Result<Self, SshError> {
        let stream = self
            .direct_tcpip(options.target.host(), options.target.port())
            .await?;
        let authenticated = handshake::handshake(options, stream).await?;
        Ok(Self {
            _live: LiveConnection::new(),
            session: authenticated.session,
            inbound: authenticated.inbound,
            target: options.target.clone(),
            originator: self.originator.clone(),
            under: Vec::new(),
        })
    }

    /// **同步门面**：建一条到 `options` 的连接，经 `hops` 这条跳板链（空链 = 直连）。
    ///
    /// 与 [`crate::SshTransport::connect_via`] 同一形状、同一约束（**不得在 tokio
    /// 上下文里调用**，见 ADR-0003 D3），区别只在终点：那边在目标上再开一个 **shell 通道**
    /// （终端），这边**只要连接本身**。端口转发（plan 0602 起）在这条连接上按需开通道 ——
    /// 那正是 D9 把"连接"与"通道"分开的理由。
    pub fn connect_via(
        runtime: &RuntimeHandle,
        hops: Vec<SshConnect>,
        options: SshConnect,
    ) -> Result<Self, SshError> {
        Self::connect_via_until(runtime, hops, options, std::future::pending())
    }

    /// 同上，但这次尝试**可以被中途叫停**（`cancel` 就绪即返回 [`SshError::Cancelled`]）。
    ///
    /// ⚠️ **为什么需要第二个入口**：隧道那条路把这次调用放在**阻塞线程**上（app 侧的
    /// `spawn_sync`），而"扔掉正在 await 它的那个 future"**取消不了**它 —— 阻塞任务照跑
    /// 到底，它建起来的那个 socket 也就一直开着（最长一个 `connect_timeout`，D15 的 10 s）。
    /// 所以"关闭 `Session` 立刻断连"（plan 0606）必须把信号送进**这次调用本身**：
    /// 下面 `block_on` 的 future 里 `select!` 一下，整条建链（连同它的 socket）
    /// 就随那个 future 一起结束了。
    pub fn connect_via_until(
        runtime: &RuntimeHandle,
        hops: Vec<SshConnect>,
        options: SshConnect,
        cancel: impl Future<Output = ()>,
    ) -> Result<Self, SshError> {
        if RuntimeHandle::try_current().is_ok() {
            return Err(SshError::BlockingInsideRuntime);
        }
        runtime.block_on(async {
            tokio::select! {
                result = Self::chain(hops, options) => result,
                () = cancel => Err(SshError::Cancelled),
            }
        })
    }

    /// 逐跳搭链，终点是"这条连接自己"（整条链交给它持有 —— 见 `under` 字段的文档）。
    async fn chain(hops: Vec<SshConnect>, mut options: SshConnect) -> Result<Self, SshError> {
        let mut under = hops_chain(hops).await?;
        let mut target = match under.last() {
            None => Self::connect(&mut options).await?,
            Some(previous) => previous.over(&mut options).await?,
        };
        std::mem::swap(&mut target.under, &mut under);
        Ok(target)
    }

    /// **D9 的原语本体**：在一条已认证的连接上开一条 `direct-tcpip` 通道，交出一条流。
    ///
    /// `host` / `port` 是**对端**（跳板机）去连的地址 —— 它在跳板机的网络里解析，
    /// 与我们的 DNS 无关（这正是跳板的意义）。
    pub async fn direct_tcpip(&self, host: &str, port: u16) -> Result<SshStream, SshError> {
        tracing::debug!(
            host,
            port,
            via = %self.target,
            "ssh direct-tcpip opening"
        );
        let channel = self
            .session
            .channel_open_direct_tcpip(
                host,
                u32::from(port),
                self.originator.address.clone(),
                self.originator.port,
            )
            .await
            .map_err(|err| forward_failed(host, port, &err))?;
        Ok(SshStream::new(channel.into_stream()))
    }

    /// 这条连接连的是谁。
    pub fn target(&self) -> &SshTarget {
        &self.target
    }

    /// 在这条连接上打开一个 **SFTP 会话**（plan 0701，ADR-0006 **D2**）。
    ///
    /// 它是"连接"与"通道"分开（D9）之后自然长出来的第四种用法：会话跑在一条新通道上，
    /// 而这条连接就是它的底层。host↔host 的 B 档（plan 0703）用的是同一句话 ——
    /// 区别只在**那时这条连接本身**来自 [`Self::direct_tcpip`]。
    pub async fn sftp(&self) -> Result<SftpClient, SshError> {
        self.sftp_with_timeout(crate::sftp::SFTP_REQUEST_TIMEOUT_SECS)
            .await
    }

    /// 同上，但把"等对端第一条回复"的期限交出来（秒）。
    ///
    /// 为什么把它做成公开的：对端**没有开 SFTP** 时，客户端能看到的唯一现象就是
    /// "等不到第一条回复"—— 那个期限因此直接决定用户要等多久才知道自己配错了。
    /// 默认值（[`crate::sftp::SFTP_REQUEST_TIMEOUT_SECS`] = 10 s）对真人够用，
    /// 而把它写死会让"这条路上会失败"的用例只能靠等满 10 秒来证明。
    pub async fn sftp_with_timeout(&self, timeout_secs: u64) -> Result<SftpClient, SshError> {
        SftpClient::open_with_timeout(&self.session, &self.target, timeout_secs).await
    }
    /// 这条连接**是不是已经没了**（对端断开、保活耗尽、网络中断）。
    ///
    /// 上游只给了这一个**同步**的问法（`Handle::is_closed`，背后是"消息循环的接收端还在不在"），
    /// 没有可 `await` 的关闭信号 —— 所以用它的是转发任务的定时检查，不是"等在这里"（plan 0605）。
    ///
    /// ⚠️ 它对"半死"的连接**不敏感**：TCP 没断、对端也不回话时，要等保活耗尽
    /// （[`crate::SshConfig::keepalive_interval`] × `keepalive_max`，默认约 90 秒）才会变真。
    pub fn is_closed(&self) -> bool {
        self.session.is_closed()
    }

    /// 这条连接上的入站路由（服务端发起的 `forwarded-tcpip` 交给谁）。
    ///
    /// 只有 [`crate::RemoteForward::open`] 用它 —— 那里也是**唯一**会往里面登记的地方。
    pub(crate) fn inbound(&self) -> &Arc<Inbound> {
        &self.inbound
    }

    /// **D10 的请求那一半**：请服务端在 `address:port` 上监听，返回它实际监听的端口。
    ///
    /// `port = 0` 时由服务端挑一个（它的回复里带着那个端口，D10 要求使用返回值）。
    /// `address` 按**服务端**那一侧解释 —— 要不要在非回环地址上开是它的策略，我们只请求。
    ///
    /// ⚠️ 与 [`Self::direct_tcpip`] 一样，这是**另一套机制**：那边是"请对端连出去"，
    /// 这边是"请对端听起来"。两者的失败也分属不同的错误档
    /// （[`SshError::Forward`] 与 [`SshError::RemoteListen`]）。
    pub(crate) async fn remote_listen(&self, address: &str, port: u16) -> Result<u16, SshError> {
        tracing::debug!(address, port, via = %self.target, "ssh tcpip-forward requested");
        let reported = self
            .session
            .tcpip_forward(address, u32::from(port))
            .await
            .map_err(|err| remote_listen_failed(&host_and_port(address, port), &err))?;

        // ⚠️ 服务端**只在请求的就是 0 端口时**才在回复里带端口（RFC 4254 §7.1），
        // 而上游把"回复里没有端口字段"表示成 `0`（`client/encrypted.rs` 的原话：
        // *If a specific port was requested, the reply has no data* → `Some(0)`；
        // 服务端那一侧同样只在 `port == 0` 时才写这个字段）。所以这个返回值有两种含义，
        // **必须按请求的是什么来解**：
        //   * 请求了具体端口 → 那就是它（此时的 `0` 不是"绑到了 0 端口"）；
        //   * 请求了 0 → 只能用回复里的那个；它也是 0 说明服务端没给（协议上不该发生）。
        // 读错的后果不是显示错一个数字：撤销监听要用这个端口，用 0 去撤销等于**没撤销**。
        if port != 0 {
            return Ok(port);
        }
        u16::try_from(reported)
            .ok()
            .filter(|reported| *reported != 0)
            .ok_or_else(|| SshError::RemoteListen {
                address: host_and_port(address, port),
                reason: format!("请求由服务端挑端口，但它没有回报端口（回报值 {reported}）"),
            })
    }

    /// 撤销 [`Self::remote_listen`] 请来的那条监听。
    ///
    /// 收尾时调用；失败只记一条 warn（那条连接随后就被断开，服务端会一并撤掉
    /// 它的监听 —— 这条请求只是让对端**先**知道，且日志里留下原因）。
    pub(crate) async fn cancel_remote_listen(
        &self,
        address: &str,
        port: u16,
    ) -> Result<(), SshError> {
        tracing::debug!(address, port, via = %self.target, "ssh cancel-tcpip-forward requested");
        self.session
            .cancel_tcpip_forward(address, u32::from(port))
            .await
            .map_err(|err| remote_listen_failed(&host_and_port(address, port), &err))
    }

    /// 收尾：**显式**断开（`Handle` 一 drop 也会结束连接，但那次是"悄悄走"，
    /// 服务端只会看到 TCP 断了；这一句让它能记下原因）。
    ///
    /// 下层链**从最内层往外**断（与 `SshTransport` 的收尾同一条理由）：反过来会把承载
    /// 后面每一跳的那条通道先踩掉，那些 `disconnect` 就都发在一条已经死掉的连接上。
    pub async fn disconnect(mut self) {
        let _ = self
            .session
            .disconnect(Disconnect::ByApplication, "", "")
            .await;
        let mut under = std::mem::take(&mut self.under);
        while let Some(mut hop) = under.pop() {
            let _ = hop
                .session
                .disconnect(Disconnect::ByApplication, "", "")
                .await;
            // 保险：万一某一跳自己也挂着下层（本 crate 现在不这么用），一并按序断掉。
            under.append(&mut hop.under);
        }
    }
}
