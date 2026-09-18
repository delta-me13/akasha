//! 同步 `Transport` 门面：**门是同步的，门后面是异步的**（ADR-0003 D3）。
//!
//! 会话层（`src-tauri/src/session.rs`）持的是 `Box<dyn Transport>`，所以 SSH 要做的
//! 就是**装进同一个 trait** —— 不改会话层、不改前端。这个文件就是那次"装进去"：
//!
//! | 方向 | 同步侧 | 队列 | 异步侧（连接自己的 task） |
//! |---|---|---|---|
//! | 写 | [`Transport::write`] | 有界 `mpsc`（[`COMMAND_QUEUE`]） | `Channel::data_bytes` |
//! | 尺寸 | [`Transport::resize`] | 同一条队列 | `window_change` |
//! | 读 | [`std::io::Read`]（合批器去读） | 有界 `mpsc`（[`OUTPUT_QUEUE`]） | `ChannelMsg::Data` → 入队 |
//!
//! 两条队列都是**有界**的，这不是省内存，而是**背压**：读方向满了就没人从连接里取字节，
//! 于是一路收紧到 TCP 窗口；写方向满了就返回 [`TransportError::Busy`]，
//! 而**绝不**用 `blocking_send` 阻塞调用线程（ADR D3：它会在 tokio 上下文里 panic，
//! 在其他上下文里会把 UI 拖住）。
//!
//! 结局（`exit-status` / `exit-signal`）由那条 task 记下来，[`Transport::exited`] 与
//! [`Transport::shutdown`] 读同一份 —— 后者是"幂等"这条契约的落点：收尾两次返回同一个结局。
//!
//! `session_leader()` **不覆写**（默认 `None`）：SSH 没有本地进程要收（D4）。
//! 进程被 SIGKILL 时 socket 由内核关掉，服务端随之拆掉它那边的监听 ——
//! 看门狗这条路对 SSH 无事可做。

use std::io::{self, Read};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use akasha_pty::{Capabilities, ExitStatus, TerminalSize, Transport, TransportError};
use russh::client::{self, Handle};
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect};
use tokio::runtime::Handle as RuntimeHandle;
use tokio::sync::mpsc;

use crate::ssh::error::SshError;
use crate::ssh::forward::{SshConnection, hops_chain};
use crate::ssh::handshake::{Handler, SshConnect, establish};

/// 写队列的容量（**积压上限，不是缓冲优化**：满了就是 [`TransportError::Busy`]）。
///
/// 64 条对终端输入是很大的余量：一次按键一条，而 task 只在 `data_bytes` 上等窗口。
const COMMAND_QUEUE: usize = 64;

/// 读队列的容量。同样是有界背压 —— 合批器跟不上时，连接那边就该慢下来。
const OUTPUT_QUEUE: usize = 64;

/// 收尾时等对端回 `Close` 的期限。
///
/// 有期限而不是无限等：收尾发生在"关标签页 / 退出应用"这条路上，对端不应答时
/// 用户看到的是"点了 × 什么都没发生"。
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

/// 同步侧送给 task 的东西。
enum Command {
    /// 用户的按键字节。
    Data(Vec<u8>),
    /// 窗口尺寸变化。
    Resize(TerminalSize),
}

/// task 与同步侧共享的那一份状态：**结局**。
#[derive(Default)]
struct Shared {
    status: Mutex<Option<ExitStatus>>,
}

impl Shared {
    fn set(&self, status: Option<ExitStatus>) {
        *self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = status;
    }

    fn get(&self) -> Option<ExitStatus> {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// 一条 SSH 连接 + 它的 shell 通道，装成会话层认识的 [`Transport`]。
pub struct SshTransport {
    /// 写命令的口。`None` = 已经收尾（或正在收尾）。
    commands: Option<mpsc::Sender<Command>>,
    /// 输出流的读端。**只能取走一次**（[`Transport::output_stream`] 的契约）。
    output: Option<SshOutput>,
    /// 与 task 共享的结局。
    shared: Arc<Shared>,
    /// task 收工的信号（同步侧的 `mpsc`，所以有 `recv_timeout` 可用）。
    ended: Receiver<()>,
    /// task 的把手。只在超时的时候用它 `abort`。
    task: Option<tokio::task::JoinHandle<()>>,
    /// 收尾之后的结局。**`Some` 就代表"已经收过尾"** —— 幂等性靠它，不靠 task 的状态。
    finished: Option<Option<ExitStatus>>,
}

impl SshTransport {
    /// 连上、认证、开一个带 pty 的 shell 通道，然后把它包成同步门面。
    ///
    /// `runtime` 由**调用方**提供（ADR D2）：库不自建 runtime。
    ///
    /// ⚠️ 这个函数**会阻塞**调用线程（握手 + 认证 + 开通道都在这条路上），最长
    /// [`SshConfig::connect_timeout`](crate::ssh::SshConfig::connect_timeout) 那个量级。
    /// 而且它**不能在 tokio 上下文里调用** —— 那种情况下返回
    /// [`SshError::BlockingInsideRuntime`]，不是 panic（调用方是 app 的命令层，
    /// 那里 panic 会连带丢掉整个 app）。
    pub fn connect(runtime: &RuntimeHandle, options: SshConnect) -> Result<Self, SshError> {
        Self::connect_via(runtime, Vec::new(), options)
    }

    /// 经一条**跳板链**连到 `options` 描述的目标（plan 0505 / D9）。
    ///
    /// `hops` 从**最外层**到最内层：第一个是 app 直接连的那一台，最后一个是紧挨着目标的
    /// 那一台；空链就是直连（[`Self::connect`]）。
    ///
    /// 每一跳各是一份 [`SshConnect`] —— 也就是**每一跳各问各的凭据、各校各的主机密钥**
    /// （跳板机与目标机是两台机器，信任记录与密钥池当然各是各的）。
    ///
    /// ⚠️ 与 [`Self::connect`] 同一套约束：**不能在 tokio 上下文里调用**，而且**会阻塞**
    /// 调用线程 —— 最坏情况是"每一跳各一次 `connect_timeout`"，别在 UI 线程上直接调它。
    pub fn connect_via(
        runtime: &RuntimeHandle,
        hops: Vec<SshConnect>,
        mut options: SshConnect,
    ) -> Result<Self, SshError> {
        if RuntimeHandle::try_current().is_ok() {
            return Err(SshError::BlockingInsideRuntime);
        }

        let established = runtime.block_on(async {
            // 跳板链的搭法**只有一份实现**（`akasha-ssh::forward::hops_chain`）：
            // 隧道那条路（`SshConnection::connect_via`）用的是同一个函数，差的只是终点。
            let carriers = hops_chain(hops).await?;
            // 最后一条流给目标：它是"离目标最近的那一跳"上的一条通道，直连时则是 None。
            let under = match carriers.last() {
                None => None,
                Some(previous) => Some(
                    previous
                        .direct_tcpip(options.target.host(), options.target.port())
                        .await?,
                ),
            };
            establish(&mut options, under, carriers).await
        })?;

        let (command_tx, command_rx) = mpsc::channel(COMMAND_QUEUE);
        let (output_tx, output_rx) = mpsc::channel(OUTPUT_QUEUE);
        let shared = Arc::new(Shared::default());
        let (ended_tx, ended_rx) = std::sync::mpsc::channel();

        let task = runtime.spawn(pump(
            established.read,
            established.write,
            command_rx,
            output_tx,
            Arc::clone(&shared),
            // 句柄交给 task：**task 结束就是连接结束**（收尾的唯一出口在那里）。
            established.session,
            // 跳板链一起进去：它们必须活到这条连接结束（`direct_tcpip` 的那条通道长在
            // 其中最后一条上），而"谁在什么时候丢掉它们"因此只有一个答案。
            established.carriers,
            ended_tx,
        ));

        Ok(Self {
            commands: Some(command_tx),
            output: Some(SshOutput::new(output_rx)),
            shared,
            ended: ended_rx,
            task: Some(task),
            finished: None,
        })
    }

    /// 把一条命令放进队列。两条命令路径（字节 / 尺寸）只差一个变体，所以合成一处。
    fn send(&self, command: Command) -> Result<(), TransportError> {
        let Some(commands) = &self.commands else {
            return Err(TransportError::Closed);
        };
        match commands.try_send(command) {
            Ok(()) => Ok(()),
            // 队列满：**显式说出来**（ADR D3）。调用方可以稍后重试 ——
            // 这条是 [`TransportError`] 里唯一可能自愈的变体。
            Err(mpsc::error::TrySendError::Full(_)) => Err(TransportError::Busy("ssh-write")),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(TransportError::Closed),
        }
    }
}

impl Transport for SshTransport {
    /// D4：SSH 终端**有**尺寸（`window_change`）也**有**结局（远端 shell 报 `exit-status`），
    /// 但在本地**没有进程**（`session_leader()` 保持默认的 `None`）。
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            resize: true,
            exit_status: true,
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.send(Command::Data(bytes.to_vec()))
    }

    fn output_stream(&mut self) -> Option<Box<dyn Read + Send>> {
        self.output
            .take()
            .map(|output| Box::new(output) as Box<dyn Read + Send>)
    }

    fn resize(&mut self, size: TerminalSize) -> Result<(), TransportError> {
        self.send(Command::Resize(size))
    }

    fn exited(&mut self) -> Result<Option<ExitStatus>, TransportError> {
        if let Some(status) = &self.finished {
            return Ok(status.clone());
        }
        Ok(self.shared.get())
    }

    /// 收尾：通知 task 走关闭流程、等它收工、交出结局。**幂等**。
    ///
    /// 顺序与理由：
    /// 1. **丢掉写口**（`commands.take()`）—— task 看到队列关闭就去做
    ///    `close` + 有期限地读干剩下的消息。不用 `blocking_send` 送一个"关闭"命令：
    ///    队列满时它会挂住调用线程。
    /// 2. **有期限地等** task 回话。等到了就交出它记下的结局。
    /// 3. 等不到就 `abort`：**连接必须死**，哪怕对端不配合 —— 否则关标签页之后
    ///    进程里还留着一条活连接，而"零残留"正是这条路径的判据。
    fn shutdown(&mut self) -> Result<Option<ExitStatus>, TransportError> {
        if let Some(status) = &self.finished {
            return Ok(status.clone());
        }

        self.commands.take();

        let outcome = self.ended.recv_timeout(SHUTDOWN_DEADLINE);
        let status = self.shared.get();
        self.finished = Some(status.clone());
        self.task.take();

        match outcome {
            Ok(()) => Ok(status),
            Err(RecvTimeoutError::Timeout) => {
                // 超时不再持有 task 的把手（上面已经 `take`）—— 这里要的是"把它掐掉"，
                // 所以先把把手拿回来再 abort。
                Err(TransportError::Io(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("ssh 收尾超时（{SHUTDOWN_DEADLINE:?}）"),
                )))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(TransportError::Io(io::Error::other("ssh 收尾任务没有回话")))
            }
        }
    }
}

/// 连接的读端：把有界队列包成 `std::io::Read`，交给现成的合批器。
struct SshOutput {
    /// task 送来的字节块。
    rx: mpsc::Receiver<Vec<u8>>,
    /// 上一次没读完的尾巴（`read` 的 `buf` 可能比一块小）。
    pending: Vec<u8>,
    offset: usize,
}

impl SshOutput {
    fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            pending: Vec::new(),
            offset: 0,
        }
    }
}

impl Read for SshOutput {
    /// 阻塞到有数据、或者连接结束（返回 `Ok(0)` = EOF）。
    ///
    /// ⚠️ `blocking_recv` **不能在 tokio 上下文里调用**。调用方是合批线程
    /// （`akasha_pty::spawn_batcher` 起的是 `std::thread`），所以这里成立 ——
    /// 而这也是"读方向必须有一条队列"的原因：把异步的连接状态机留给 task。
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.offset >= self.pending.len() {
            match self.rx.blocking_recv() {
                Some(chunk) => {
                    self.pending = chunk;
                    self.offset = 0;
                }
                // 队列空了且发送端没了 = 连接结束。读 `0` 是 EOF 的标准说法，
                // 合批器据此收工（`akasha-pty` 的读循环就是这么结束的）。
                None => return Ok(0),
            }
        }
        let remaining = &self.pending[self.offset..];
        let taken = remaining.len().min(buf.len());
        buf[..taken].copy_from_slice(&remaining[..taken]);
        self.offset += taken;
        Ok(taken)
    }
}

/// 连接自己的 task：**唯一的**一方同时碰读半、写半与句柄。
#[allow(clippy::too_many_arguments)]
async fn pump(
    mut read: ChannelReadHalf,
    write: ChannelWriteHalf<client::Msg>,
    mut commands: mpsc::Receiver<Command>,
    output: mpsc::Sender<Vec<u8>>,
    shared: Arc<Shared>,
    session: Handle<Handler>,
    mut carriers: Vec<SshConnection>,
    ended: std::sync::mpsc::Sender<()>,
) {
    let mut status = None;
    let mut closing = false;

    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Data(bytes)) => {
                    if write.data_bytes(bytes).await.is_err() {
                        break;
                    }
                }
                Some(Command::Resize(size)) => {
                    if write
                        .window_change(u32::from(size.cols), u32::from(size.rows), 0, 0)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                // 写口关了 = 调用方要求收尾。**先别走**：还要把对端的结局读回来。
                None => {
                    closing = true;
                    break;
                }
            },
            // `ChannelReadHalf::wait` 就是一次 `recv`，取消是安全的
            // （另一支先完成时不会丢消息）。
            message = read.wait() => match handle(&mut status, message, &output).await {
                Flow::Continue => {}
                Flow::Done => break,
            },
        }
    }

    if closing {
        // **先 EOF、再读、最后 CLOSE**（RFC 4254 的半关顺序）：EOF 是"我不再发了"，
        // 但对端还可能把结下来的数据与结局发出来 —— 直接 `close()` 会把它们一起丢掉，
        // 表现就是"关标签页之后拿不到退出码"。
        let _ = write.eof().await;
        let drain = drain_channel(&mut read, &output, &mut status);
        // 有期限：对端不回 `Close` 时不能把收尾变成永久等待。
        if tokio::time::timeout(SHUTDOWN_DEADLINE, drain)
            .await
            .is_err()
        {
            tracing::warn!("ssh channel close timed out");
        }
        let _ = write.close().await;
    }

    // 显式断开：`Handle` 一 drop 也会结束连接，但那时是"悄悄走"，
    // 服务端只会看到 TCP 断了；这一句是礼貌且有用的（服务端能记下原因）。
    let _ = session.disconnect(Disconnect::ByApplication, "", "").await;

    // 跳板链：**从最内层往外**断（`pop` 拿到的正是最内层）。反过来会把承载我们的那条
    // 通道先踩掉 —— 后面每一跳的 disconnect 就都发在一条已经死掉的连接上，
    // 服务端看到的是"被 TCP 掐了"，而不是我们说了再见。
    while let Some(carrier) = carriers.pop() {
        carrier.disconnect().await;
    }

    shared.set(status);
    // 回话失败只可能是同步侧已经不在了（调用了 `shutdown` 之后又 drop 了传输）——
    // 那一边本来也不等，所以不记日志。
    let _ = ended.send(());
}

/// 收尾阶段的读循环：把剩下的消息读干（要么读完、要么对端关掉）。
async fn drain_channel(
    read: &mut ChannelReadHalf,
    output: &mpsc::Sender<Vec<u8>>,
    status: &mut Option<ExitStatus>,
) {
    while let Some(message) = read.wait().await {
        if let Flow::Done = handle(status, Some(message), output).await {
            break;
        }
    }
}

/// 一条通道消息的处置结果。
enum Flow {
    /// 继续读。
    Continue,
    /// 通道结束（对端关闭 / 读到 `None` / 没人再读我们的输出了）。
    Done,
}

/// 处理一条通道消息：数据入队、结局记下、关闭则收工。
///
/// 抽成函数不是为了复用（两处调用），而是为了让 `select!` 的两支都短到看得清分支。
async fn handle(
    status: &mut Option<ExitStatus>,
    message: Option<ChannelMsg>,
    output: &mpsc::Sender<Vec<u8>>,
) -> Flow {
    match message {
        Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
            // ⚠️ **这里会 await**：队列满时就此停住 —— 读方向的背压正是这么来的
            // （没人取字节 → 我们不再从连接里读 → 一路收紧到 TCP 窗口）。
            if output.send(data.to_vec()).await.is_err() {
                return Flow::Done;
            }
            Flow::Continue
        }
        Some(ChannelMsg::ExitStatus { exit_status }) => {
            *status = Some(ExitStatus::Code(exit_status));
            Flow::Continue
        }
        Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
            // `Sig` 的 `name()` 是私有的，`Debug` 给出的正是 `KILL` / `TERM` 这种形态
            // —— 与 PTY 那条路把它当字符串带走的做法一致（`ExitStatus::Signal`）。
            *status = Some(ExitStatus::Signal(format!("{signal_name:?}")));
            Flow::Continue
        }
        Some(ChannelMsg::Close) | None => Flow::Done,
        Some(_) => Flow::Continue,
    }
}
