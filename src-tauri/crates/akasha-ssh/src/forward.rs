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

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use russh::client::{self, Handle};
use russh::{ChannelStream, Disconnect};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::runtime::Handle as RuntimeHandle;

use crate::error::{SshError, forward_failed};
use crate::handshake::{self, Handler, SshConnect};
use crate::target::SshTarget;

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

/// 一条**已认证、没有通道**的 SSH 连接。**持有它就是让这条连接活着**（`Handle` 一 drop，
/// 上游的会话任务随之收工）。
///
/// 为什么单独一个类型：跳板链上的每一跳、以及阶段 6/7 的转发会话，要的都是"一条连接"
/// 本身，而不是"一个带 pty 的 shell 终端"（那是 [`crate::SshTransport`]）。
pub struct SshConnection {
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
        let session = handshake::handshake(options, stream).await?;
        Ok(Self {
            session,
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
        let session = handshake::handshake(options, stream).await?;
        Ok(Self {
            session,
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
        mut options: SshConnect,
    ) -> Result<Self, SshError> {
        if RuntimeHandle::try_current().is_ok() {
            return Err(SshError::BlockingInsideRuntime);
        }
        runtime.block_on(async {
            let mut under = hops_chain(hops).await?;
            let mut target = match under.last() {
                None => Self::connect(&mut options).await?,
                Some(previous) => previous.over(&mut options).await?,
            };
            // 整条链交给目标那条连接持有 —— 见 `under` 字段的文档。
            std::mem::swap(&mut target.under, &mut under);
            Ok(target)
        })
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
