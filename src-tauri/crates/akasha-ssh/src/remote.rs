//! **远程转发 `-R`**：请服务端监听，把服务端发起的 `forwarded-tcpip` 通道接到本机服务。
//!
//! 这是 [ADR-0003 D10](../../../docs/adr/0003-ssh-stack-and-resource-model.md) 的落地，
//! 也是**唯一**一条方向相反的转发：`-L` / `-D` 在本机监听、对端去连（D9 的 `direct-tcpip`
//! 原语），`-R` 由**服务端**监听、服务端发起通道（RFC 4254 §7.2 的 `forwarded-tcpip`）。
//! 因此这里没有一行复用那个原语。共用的东西在别处：隧道实体的登记 / 状态 / 停止在
//! app 侧的 `tunnel.rs`，目标地址的类型在 [`crate::ForwardTarget`]。
//!
//! ## 三步，顺序是刻意的
//!
//! 1. **请求**：`tcpip_forward(address, port)`。`port = 0` 时由服务端挑一个并在回复里给回来
//!    （D10 写明了要使用返回值）。被拒在这里就能报出来，而且原因是**服务端那一侧**的。
//! 2. **登记**：把"这个端口上的入站通道交给谁"记进这条连接的表里（[`Inbound`]）。
//!    [`Handler`] 的回调按**端口**查这张表 —— 理由见 [`Inbound::deliver`]。
//! 3. **接线**：每条入站通道**先连本机目标**（[`ForwardTarget`] 在这里由**本机**解析，
//!    与 `-L` 的"由对端解析"正好相反），通了才接受那条通道。
//!
//! ## 为什么"先连、后接受"
//!
//! 本机服务不可达时，我们这一侧唯一能对外说的话是那次**通道拒绝**
//! （`ChannelOpenFailure::ConnectFailed`）。反过来"先接受、连不上再关"会让对端的客户端
//! 先看到连接建立、随后立刻被关 —— 用户看到的是"时通时不通"，而 OpenSSH 的
//! `client_request_forwarded_tcpip` 是前一种行为。
//!
//! ⚠️ 那个连接**必须在任务里做**：`Handler` 的回调是在连接的消息循环上被 `await` 的，
//! 在回调里等一个不可达地址会让整条连接无响应（连保活都停）—— 而那是
//! "隧道看起来还活着"的形态，比一条明确的失败难查得多。
//!
//! ## 谁持有那条连接
//!
//! 与 [`crate::LocalListener::serve`] 同一条纪律：[`RemoteForward::open`] **拿走**
//! [`SshConnection`] 的所有权，"这条转发还活着"与"那条连接还活着"因此是同一件事。
//! 收尾只有一条路径（任务收到停止信号），它按固定顺序做完四件事：
//! 收掉在途连接 → 撤掉登记（此后到达的通道一律被拒）→ `cancel_tcpip_forward`
//! → 礼貌断开连接。

use std::sync::{Arc, Mutex};

use russh::client::{ChannelOpenHandle, Msg};
use russh::{Channel, ChannelOpenFailure};
use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use tokio::runtime::Handle as RuntimeHandle;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::ending::{ForwardEnd, ForwardEnding};
use crate::error::SshError;
use crate::forward::SshConnection;
use crate::relay::{ForwardTarget, LIVENESS_POLL_INTERVAL};
use crate::target::host_and_port;

/// 服务端发起的一条 `forwarded-tcpip` 通道 —— 连同它的接受句柄交给转发任务。
///
/// 接受句柄（`reply`）**跟着它一起走**是刻意的：这样"拒绝"就只有一处出口，
/// 而"转发已经停了"那种情形不需要写代码 —— 发送失败会让它随消息一起被丢掉，
/// 而丢掉 `reply` 就是拒绝（上游的 `Drop` 实现发 `AdministrativelyProhibited`）。
pub(crate) struct Incoming {
    /// 那条通道（还没接受，也还没开始搬字节）。
    pub(crate) channel: Channel<Msg>,
    /// 服务端报的"这条连接是从哪条监听上来的"（`connected_address` / `connected_port`）。
    pub(crate) connected: (String, u32),
    /// 连到服务端那个端口的那一端的地址（RFC 4254 的原话，只进日志）。
    pub(crate) originator: (String, u32),
    /// 接受或拒绝那条通道的句柄。
    pub(crate) reply: ChannelOpenHandle,
}

/// 一条连接上的入站路由：服务端发起的通道该交给谁。
///
/// **一个格子而不是一张表**：[`RemoteForward::open`] 拿走那条连接的所有权，
/// 所以同一条连接上最多只可能存在一条远端转发（这也正是 D5 / D6 的"一条规则一条连接"）。
/// 唯一性由所有权保证，而不是靠调用方记得不要开第二条。
#[derive(Debug, Default)]
pub(crate) struct Inbound {
    /// 已经登记的那条转发。`None` = 这条连接上没有任何在听的远端转发。
    slot: Mutex<Option<Slot>>,
}

/// 格子里装的东西：实际端口，以及"交给谁"。
#[derive(Debug)]
struct Slot {
    port: u16,
    sender: mpsc::UnboundedSender<Incoming>,
}

/// 一条登记的凭据。**drop 即撤销** —— 收尾时不必记得手动撤回登记。
#[derive(Debug)]
struct Route {
    inbound: Arc<Inbound>,
    port: u16,
}

impl Drop for Route {
    fn drop(&mut self) {
        let mut slot = self
            .inbound
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 只清自己那一条（端口对得上才清）：清掉别人的登记是那种"看起来也对"的静默错误。
        if slot.as_ref().is_some_and(|slot| slot.port == self.port) {
            *slot = None;
        }
    }
}

impl Inbound {
    /// 登记一条转发，返回它的凭据。
    fn register(inbound: &Arc<Self>, port: u16, sender: mpsc::UnboundedSender<Incoming>) -> Route {
        let mut slot = inbound
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 只有 `RemoteForward::open` 会走到这里，而它拿走了连接的所有权 ——
        // 所以这一条在正常情况下不会被触发；写出来是为了让"挤掉前一条"永远不静默发生。
        debug_assert!(slot.is_none(), "同一条连接上登记了第二条远端转发");
        *slot = Some(Slot { port, sender });
        Route {
            inbound: Arc::clone(inbound),
            port,
        }
    }

    /// 收到服务端发起的通道 —— [`Handler`](crate::handshake::Handler) 的回调调它。
    ///
    /// ⚠️ **同步**（一个 `await` 都没有）：这个回调在连接的消息循环上被 `await`，
    /// 在这里等任何东西都会让整条连接停住（模块文档那条）。
    pub(crate) fn deliver(&self, incoming: Incoming) {
        let port = incoming.connected.1;
        tracing::debug!(
            address = %incoming.connected.0,
            port,
            originator = %incoming.originator.0,
            originator_port = incoming.originator.1,
            "remote forward incoming"
        );

        match self.sender_for(port) {
            Some(sender) => {
                // 发送失败 = 那条转发已经停了（接收端没了）→ 消息被退回并丢掉，
                // 也就是 `reply` 被丢掉 = 拒绝这条通道。
                if sender.send(incoming).is_err() {
                    tracing::debug!(port, "remote forward route gone");
                }
            }
            None => tracing::warn!(
                port,
                address = %incoming.connected.0,
                "remote forward incoming for an unregistered port"
            ),
        }
    }

    /// 这个端口上的通道该交给谁；`None` = 没登记过，一律拒绝（D10 的"规则不活跃时拒绝"）。
    ///
    /// 按**端口**认，不按地址字符串认：服务端回报的 `connected_address` 是"它认为在听的
    /// 地址"，与服务端自己的配置有关（请求 `localhost` 可能回报 `127.0.0.1`）。
    fn sender_for(&self, port: u32) -> Option<mpsc::UnboundedSender<Incoming>> {
        // `u32` 的端口号超出 `u16` 时不可能是我们登记过的那个（登记用的是 `u16`）。
        let port = u16::try_from(port).ok()?;
        let slot = self
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slot.as_ref()
            .filter(|slot| slot.port == port)
            .map(|slot| slot.sender.clone())
    }
}

/// 一条**起来的**远端转发。
///
/// 它活着就等于"服务端在那个端口上监听、连接在手上"。停止有两条等价的入口：
/// 调用 [`Self::shutdown`]，或者直接把它 drop（信号端一 drop，任务就走同一条收尾路径）。
#[derive(Debug)]
pub struct RemoteForward {
    /// 请求的绑定地址（原样，由**服务端**解释）。
    address: String,
    /// 服务端实际监听的端口（`port = 0` 时这就是它挑的那个 —— D10 要求使用返回值）。
    port: u16,
    shutdown: oneshot::Sender<()>,
}

impl RemoteForward {
    /// 请 `connection` 对应的服务端在 `address:port` 上监听（`port = 0` → 由它挑），
    /// 把每条入站连接接到 `target`。
    ///
    /// 拿走 `connection` 的所有权（见模块文档）。失败时那条连接已经没有用途
    /// （它是为这条转发建的），所以在这里礼貌断开再返回 —— 不留一条没人管的连接。
    ///
    /// 返回值是**两半**：转发本体与它的[结束通知](ForwardEnding)（理由同
    /// [`crate::LocalListener::serve`]）。
    pub async fn open(
        runtime: &RuntimeHandle,
        connection: SshConnection,
        address: &str,
        port: u16,
        target: ForwardTarget,
    ) -> Result<(Self, ForwardEnding), SshError> {
        // 请求里的地址按**服务端**那一侧解释（它的 `localhost` 是它自己）——
        // 要不要在非回环地址上开是**它**的策略（`GatewayPorts` 一类），我们只请求。
        let bound = match connection.remote_listen(address, port).await {
            Ok(bound) => bound,
            Err(err) => {
                connection.disconnect().await;
                return Err(err);
            }
        };

        let (sender, mut incoming) = mpsc::unbounded_channel();
        let route = Inbound::register(connection.inbound(), bound, sender);
        let (shutdown, mut stopped) = oneshot::channel::<()>();
        let (ended, ended_rx) = oneshot::channel::<ForwardEnd>();
        let worker = runtime.clone();
        let cancel_address = address.to_owned();

        tracing::debug!(address, port = bound, target = %target, "remote forward registered");

        runtime.spawn(async move {
            let connection = Arc::new(connection);
            // 在途的每条入站连接各一条任务：停止时要能**一起**收掉 ——
            // 只停监听会留下已经建立的通道，那条连接也就跟着活到最后一个客户端走为止。
            let mut live: Vec<JoinHandle<()>> = Vec::new();
            let mut liveness = tokio::time::interval(LIVENESS_POLL_INTERVAL);
            liveness.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let end = loop {
                tokio::select! {
                    // 先看停止信号：已经被要求停止时不该再接下一条连接。
                    _ = &mut stopped => break ForwardEnd::Stopped,
                    // 连接死了就结束自己。这一条在 `-R` 上尤其要紧：远端监听**属于那条连接**，
                    // 连接一没它就被服务端撤销了，而这条转发若还活着，界面会一直说"已连接"。
                    _ = liveness.tick() => {
                        if connection.is_closed() {
                            break ForwardEnd::ConnectionLost;
                        }
                    }
                    received = incoming.recv() => match received {
                        Some(incoming) => {
                            live.retain(|task| !task.is_finished());
                            live.push(worker.spawn(connect_local(incoming, target.clone())));
                        }
                        // 发送端没了 = 登记被撤掉（收尾时我们自己撤的，或者那条连接没了）
                        // → 这条转发再也收不到任何通道，结束。
                        None => break ForwardEnd::ConnectionLost,
                    },
                }
            };
            for task in &live {
                task.abort();
            }
            for task in live {
                // 等它们真的结束：abort 只是请求，而下面要拿回连接的所有权。
                let _ = task.await;
            }
            // 先撤登记：此后到达的通道一律被拒（drop `reply`），不会被接到一条正在收尾的转发上。
            drop(route);
            // 再请服务端把那个监听撤掉。顺序不能反过来：先撤监听的话，在它生效之前到达的
            // 通道仍会进到这条已经决定停止的转发里。
            // ⚠️ 两条结束原因要用**不同的级别**：连接已经死了时这条请求注定失败（没人可问），
            // 每次都记 warn 只会把真正要看的那一条（停止时的撤销失败）淹掉。
            match (end, connection.cancel_remote_listen(&cancel_address, bound).await) {
                (_, Ok(())) => {}
                (ForwardEnd::Stopped, Err(err)) => {
                    tracing::warn!(address = %cancel_address, port = bound, %err, "remote forward cancel failed");
                }
                (ForwardEnd::ConnectionLost, Err(err)) => {
                    tracing::debug!(address = %cancel_address, port = bound, %err, "remote forward cancel skipped");
                }
            }
            if let Ok(connection) = Arc::try_unwrap(connection) {
                connection.disconnect().await;
            }
            tracing::debug!(port = bound, reason = end.as_str(), "remote forward ended");
            // 结束信号**最后**发（与 `serve` 同一条纪律）。
            let _ = ended.send(end);
        });

        Ok((
            Self {
                address: address.to_owned(),
                port: bound,
                shutdown,
            },
            ForwardEnding::new(ended_rx),
        ))
    }

    /// 请求的绑定地址（原样）。
    pub fn address(&self) -> &str {
        &self.address
    }

    /// 服务端实际监听的端口。
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// 给人看的监听地址形态（错误消息、日志与 probe 用同一份拼法）。
    pub fn bound_display(&self) -> String {
        host_and_port(&self.address, self.port)
    }

    /// 停止：**发完信号即返回**（理由与 [`crate::LocalForward::shutdown`] 相同）。
    /// 撤销远端监听与断开连接都在 runtime 上做，要观察结果的地方看服务端的记录。
    pub fn shutdown(self) {
        let _ = self.shutdown.send(());
    }
}

/// 一条入站连接的接线：**先连本机目标，再决定接受还是拒绝**那条通道。
///
/// 本机目标在这里由**本机**解析 —— 与 `-L` 正好相反（那边由对端解析，绕开本机 DNS
/// 正是跳板的意义）。这里的地址说的是"我们这台机器上的服务"，
/// 交给对端解析反而是错的（对端的 `localhost` 是它自己）。
async fn connect_local(incoming: Incoming, target: ForwardTarget) {
    let Incoming {
        channel,
        originator,
        reply,
        ..
    } = incoming;
    let address = host_and_port(target.host(), target.port());
    let mut socket = match TcpStream::connect(address.as_str()).await {
        Ok(socket) => socket,
        Err(err) => {
            // **拒绝而不是"接受了再关"**（模块文档）：对端的客户端因此立刻知道本机服务不可达，
            // 而不是先拿到一条"连上了"的连接、再看着它被关。
            reply.reject(ChannelOpenFailure::ConnectFailed).await;
            tracing::warn!(
                host = target.host(),
                port = target.port(),
                originator = %originator.0,
                %err,
                "remote forward local connect failed"
            );
            return;
        }
    };
    reply.accept().await;

    let mut stream = channel.into_stream();
    match copy_bidirectional(&mut stream, &mut socket).await {
        // 第一个数是"从通道搬到本机服务"的字节，第二个数反过来。
        Ok((to_local, to_remote)) => {
            tracing::debug!(to_local, to_remote, "remote forward closed");
        }
        // 收尾那一下报错是常态（两边任一方撤了），所以是 debug 而不是 warn。
        Err(err) => tracing::debug!(%err, "remote forward relay ended"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    /// 登记过的那个端口能查到，别的端口一律查不到 —— 查不到就拒（D10）。
    #[test]
    fn only_the_registered_port_has_a_route() {
        let inbound = Arc::new(Inbound::default());
        let (sender, _rx) = mpsc::unbounded_channel();
        let route = Inbound::register(&inbound, 46_010, sender);

        assert!(inbound.sender_for(46_010).is_some(), "登记过的端口要能查到");
        assert!(
            inbound.sender_for(46_011).is_none(),
            "没登记过的端口不许有路由 —— 服务端报错端口时必须拒绝，不能接到别的转发上"
        );
        // `u32` 里超出 `u16` 的取值不可能是我们登记的那个（登记用的是 `u16`）。
        assert!(
            inbound.sender_for(u32::from(u16::MAX) + 1).is_none(),
            "超出可表示范围的端口号不许匹配上"
        );

        // 凭据一 drop，登记随之撤销 —— 此后到达的通道一律被拒（`reply` 被丢掉）。
        drop(route);
        assert!(
            inbound.sender_for(46_010).is_none(),
            "转发停掉之后，那个端口上的通道不该还有路由"
        );
    }

    /// 凭据只撤**自己**那一条：端口对不上时不动别人的登记。
    ///
    /// 本 crate 的正常路径上不会出现第二条登记（`open` 拿走了连接的所有权），
    /// 所以这里直接造一个端口对不上的凭据 —— 要验的是那条判断本身
    /// （清掉别人的登记是那种"看起来也对"的错误）。
    #[test]
    fn a_route_only_clears_its_own_slot() {
        let inbound = Arc::new(Inbound::default());
        let (sender, _rx) = mpsc::unbounded_channel();
        let route = Inbound::register(&inbound, 46_020, sender);

        drop(Route {
            inbound: Arc::clone(&inbound),
            port: 46_099,
        });
        assert!(
            inbound.sender_for(46_020).is_some(),
            "端口对不上的凭据不该清掉登记"
        );

        drop(route);
        assert!(inbound.sender_for(46_020).is_none());
    }
}
