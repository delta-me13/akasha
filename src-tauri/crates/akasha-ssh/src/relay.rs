//! **本地转发 `-L`**（plan 0602）：本地监听 → 每条入站连接一条 `direct-tcpip` 通道。
//!
//! 这是 [`crate::SshConnection::direct_tcpip`]（D9 的原语）的第三个消费者 —— 前两处是
//! 跳板（原语当下一跳的底层流）与 SFTP 的 B 档（在它上面跑数据面），三处都不重写原语。
//! 这里的一段搬运就是 `copy_bidirectional`：一条 `AsyncRead + AsyncWrite` 的流与一个
//! `TcpStream` 之间来回拷字节，不自己写缓冲区、不解析协议。
//!
//! ## 先绑定，后连接
//!
//! [`LocalListener::bind`] 与 [`LocalListener::serve`] 刻意分成两步：绑定失败是本类功能
//! 最常见的一类失败（端口被占用），而它**必须发生在握手之前** —— 否则用户要先答完主机密钥
//! 与口令，才被告知端口没拿到；那两轮提问因此全是白费的，而且用户还会以为"连上了"。
//!
//! ## 谁持有那条连接
//!
//! [`LocalListener::serve`] **拿走** [`SshConnection`] 的所有权，由转发任务持有到结束：
//! "转发还活着"与"那条 SSH 连接还活着"因此是同一件事，不需要谁来记得配对。
//! 结束时有两条路，都通向同一处收尾：
//!
//! 1. [`LocalForward::shutdown`]（或 [`LocalForward`] 被 drop）→ 任务停止接受新连接、
//!    收掉在途任务、**礼貌断开**那条连接（`disconnect` 会把收尾原因发给对端）；
//! 2. 任务自己结束（只有上面那条路会让它结束 —— 接受出错**不**结束监听，见下）。
//!
//! ## 由谁 spawn
//!
//! 任务落在**调用方给的** `RuntimeHandle` 上（库不自建 runtime，ADR-0003 D2）：
//! 隧道那条连接的一切（握手、通道、收尾）都该在同一个 runtime 上，
//! 而 app 的那一个 runtime 是它在启动时建的（`crate::Ssh`）。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle as RuntimeHandle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::error::{SshError, listen_failed};
use crate::forward::SshConnection;
use crate::target::host_and_port;

/// 接受失败之后歇多久再试。
///
/// 存在的理由具体：`accept` 出错**不结束监听**（用尽 fd、连接在握手期被对端撤掉，
/// 都会走到这条路上），而一个持续失败的 `accept` 会立刻返回 —— 不歇一下就是一个
/// 占满一个 worker 的空转循环。100 ms 足够让暂时性错误过去，也让停止信号最多晚 100 ms 被看到。
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// 转发去往的目标：**对端**（那台 SSH 服务器）要连的地址。
///
/// 它是"给人看的规则"，不是我们自己的连接目标：这个字符串会被原样送进 `direct_tcpip`，
/// 由对端去解析 —— 在本地解析就等于绕开跳板机（D9 / `scope.md` §2.2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardTarget {
    host: String,
    port: u16,
}

impl ForwardTarget {
    /// 构造一个目标。
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    /// 对端要去连的地址（**不**在本机解析）。
    pub fn host(&self) -> &str {
        &self.host
    }

    /// 对端要去连的端口。
    pub const fn port(&self) -> u16 {
        self.port
    }
}

impl std::fmt::Display for ForwardTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&host_and_port(&self.host, self.port))
    }
}

/// 已绑定、**还没开始转发**的本地监听。
///
/// 中间状态是刻意的：调用方拿到它就可以先去连接（几秒），而端口从这一刻起已经归这条规则
/// —— 端口被别人抢走的窗口因此不存在。连接失败时把它 drop 掉，端口立刻还给系统。
#[derive(Debug)]
pub struct LocalListener {
    listener: TcpListener,
    bound: SocketAddr,
}

impl LocalListener {
    /// 绑定 `host:port`（`port = 0` 表示由内核挑一个，实际地址见 [`Self::bound`]）。
    pub async fn bind(host: &str, port: u16) -> Result<Self, SshError> {
        // 先修空格：带尾随空格的绑定地址是一个坏地址，而它的报错会指向"解析失败"。
        let host = host.trim();
        let address = host_and_port(host, port);
        if host.is_empty() {
            // `:46010` 交给 `TcpListener::bind` 只会得到一句"无效的 socket 地址"，
            // 而真正的问题是**规则里没填绑定地址**（`forwards.bind_host` 允许空串）。
            return Err(listen_failed(
                &address,
                "绑定地址是空的：填 127.0.0.1（只本机）或 0.0.0.0（同网段可达）",
            ));
        }
        let listener = TcpListener::bind(address.as_str())
            .await
            .map_err(|err| listen_failed(&address, err))?;
        let bound = listener
            .local_addr()
            .map_err(|err| listen_failed(&address, err))?;
        Ok(Self { listener, bound })
    }

    /// 实际绑定到的地址（`port = 0` 时这是内核分配的那个端口）。
    pub const fn bound(&self) -> SocketAddr {
        self.bound
    }

    /// 起转发任务：在 `runtime` 上接受连接，每条入站连接开一条 `direct_tcpip` 通道。
    ///
    /// 拿走 `connection` 的所有权（见模块文档的"谁持有那条连接"）。
    pub fn serve(
        self,
        runtime: &RuntimeHandle,
        connection: SshConnection,
        target: ForwardTarget,
    ) -> LocalForward {
        let Self { listener, bound } = self;
        let (shutdown, mut stopped) = oneshot::channel::<()>();
        let worker = runtime.clone();
        runtime.spawn(async move {
            let connection = Arc::new(connection);
            // 在途的每条入站连接各一条任务：停止时要能**一起**收掉 ——
            // 只停监听会留下已建立的通道，那条 SSH 连接也就跟着活到最后一个客户端走为止。
            let mut live: Vec<JoinHandle<()>> = Vec::new();
            loop {
                tokio::select! {
                    // 先看停止信号：已经被要求停止时不该再接下一条连接。
                    _ = &mut stopped => break,
                    accepted = listener.accept() => match accepted {
                        Ok((socket, peer)) => {
                            live.retain(|task| !task.is_finished());
                            let connection = Arc::clone(&connection);
                            let target = target.clone();
                            live.push(worker.spawn(relay(connection, socket, target, peer)));
                        }
                        Err(err) => {
                            // **不结束监听**：`accept` 的失败多半是暂时的（用尽 fd、
                            // 对端在握手期撤了）。结束监听会静默撤掉一个界面上仍写着
                            // "已连接"的转发 —— 那个谎比一条 warn 难查得多。
                            tracing::warn!(%err, "local forward accept failed");
                            tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                        }
                    },
                }
            }
            for task in &live {
                task.abort();
            }
            for task in live {
                // 等它们真的结束：abort 只是请求，而下面要拿回连接的所有权。
                let _ = task.await;
            }
            // 到这一步 Arc 应当只剩我们这一份（在途任务都已结束）。
            // 万一还有别人拿着，就让它随最后一个持有者一起走 —— 不 panic。
            if let Ok(connection) = Arc::try_unwrap(connection) {
                connection.disconnect().await;
            }
        });
        LocalForward { bound, shutdown }
    }
}

/// 一条**跑起来的**本地转发。
///
/// 它活着就等于"端口在监听、连接在手上"。停止有两条等价的入口：调用 [`Self::shutdown`]，
/// 或者直接把它 drop（信号端一 drop，转发任务就走同一条收尾路径）。
pub struct LocalForward {
    bound: SocketAddr,
    shutdown: oneshot::Sender<()>,
}

impl LocalForward {
    /// 实际监听地址。
    pub const fn bound(&self) -> SocketAddr {
        self.bound
    }

    /// 停止：**发完信号即返回**。
    ///
    /// 收尾（停止监听 → 收掉在途连接 → 礼貌断开那条 SSH 连接）要在 runtime 上做，
    /// 而调用它的地方是同步的停止命令 —— 在那里等收尾会把 IPC 线程压在一条可能很慢的
    /// `disconnect` 上。要观察收尾结果的地方（E2E）看对端的连接计数，不看这个调用的返回。
    pub fn shutdown(self) {
        let _ = self.shutdown.send(());
    }
}

/// 一条入站连接的搬运：开通道 → 双向拷字节。
///
/// 任何一步失败都只是**这一条**连接的事：通道开不出来（对端拒绝转发 / 目标不可达）
/// 会经 [`SshConnection::direct_tcpip`] 报 `SshError::Forward`，到这里记一条 warn 并让
/// socket 随函数结束而关闭 —— 客户端因此看到连接被关掉，而不是挂在那儿不动。
async fn relay(
    connection: Arc<SshConnection>,
    mut socket: TcpStream,
    target: ForwardTarget,
    peer: SocketAddr,
) {
    let mut stream = match connection.direct_tcpip(target.host(), target.port()).await {
        Ok(stream) => stream,
        Err(err) => {
            tracing::warn!(
                host = target.host(),
                port = target.port(),
                %peer,
                %err,
                "local forward channel failed"
            );
            return;
        }
    };
    match copy_bidirectional(&mut socket, &mut stream).await {
        Ok((to_remote, to_client)) => {
            tracing::debug!(to_remote, to_client, %peer, "local forward closed");
        }
        // 收尾那一下报错是常态（两边任一方撤了），所以是 debug 而不是 warn。
        Err(err) => tracing::debug!(%err, %peer, "local forward relay ended"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[tokio::test]
    async fn the_kernel_assigned_port_is_reported() {
        // `port = 0` 时"绑到了哪"只有内核知道 —— 必须能从 `bound()` 读出来，
        // 否则界面与 probe 只能说"绑在 0 端口"（那是错的：0 不是监听端口）。
        let listener = LocalListener::bind("127.0.0.1", 0).await.unwrap();
        assert_eq!(listener.bound().ip().to_string(), "127.0.0.1");
        assert_ne!(listener.bound().port(), 0);
    }

    #[tokio::test]
    async fn an_occupied_port_is_reported_with_the_address_it_wanted() {
        // 占用一个端口（**不**用 tokio）：另一个监听 socket 在同一地址上会拿到 EADDRINUSE。
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();

        let err = LocalListener::bind("127.0.0.1", port).await.unwrap_err();
        match err {
            SshError::Listen { address, reason } => {
                assert_eq!(
                    address,
                    format!("127.0.0.1:{port}"),
                    "错误里必须有想绑的地址"
                );
                assert!(
                    !reason.is_empty(),
                    "操作系统的原话必须带上（它是唯一能指认原因的）"
                );
            }
            other => panic!("端口被占用必须报 Listen，而不是 {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_empty_bind_address_is_refused_with_an_explanation() {
        // 空绑定地址是**规则里的问题**（`forwards.bind_host` 允许空串），
        // 报出来的话必须指向"填什么"，而不是底层那句"无效的 socket 地址"。
        let err = LocalListener::bind("  ", 46_030).await.unwrap_err();
        match err {
            SshError::Listen { address, reason } => {
                assert_eq!(address, ":46030");
                assert!(reason.contains("127.0.0.1"), "要说清填什么：{reason}");
            }
            other => panic!("空地址必须报 Listen，而不是 {other:?}"),
        }
    }

    #[test]
    fn a_target_prints_as_host_and_port() {
        // 日志与错误消息里的形态统一（IPv6 字面量带方括号，同 `SshTarget`）。
        assert_eq!(
            ForwardTarget::new("akasha-target.invalid", 8080).to_string(),
            "akasha-target.invalid:8080"
        );
        assert_eq!(ForwardTarget::new("::1", 8080).to_string(), "[::1]:8080");
    }
}
