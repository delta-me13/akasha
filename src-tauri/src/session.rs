//! 会话命令 —— **薄壳**：把 `akasha-pty` 的批次接到 IPC，把前端输入送回载体。
//!
//! 这里**没有任何业务逻辑**（`AGENTS.md` §0 架构原则 2）：会话的归属、回收、事件路由
//! 在 [`akasha_core`]；字节的进出与合批在 [`akasha_pty`]。本模块只做三件编组的事：
//!
//! 1. 给每个终端会话分配一个 `SessionId` 并持有它的载体（[`Sessions`]）；
//! 2. 把批次流转发给调用方给的 sink（命令里是 IPC 频道，测试里是一个 `Vec`）；
//! 3. 把 domain 错误收敛成一种可序列化的形状（[`IpcError`]）。
//!
//! # 字节怎么过 IPC（这一步的成败点）
//!
//! 输出走 **raw**：command 收下前端给的频道句柄，还原成
//! [`Channel<InvokeResponseBody>`](tauri::ipc::Channel)，每批发
//! [`InvokeResponseBody::Raw`]。**不能收 `Channel<Vec<u8>>`** —— `Vec<u8>` 命中的是
//! tauri 那条 `impl<T: Serialize> IpcResponse for T`，于是 64 KiB 的批次会变成
//! "6 万多个数字的 JSON 数组"，正是 `AGENTS.md` §3.2 禁止的那条路。
//! 参数类型只声明"频道句柄是个字符串"（[`RawChannel`]），因为 tauri 的频道**在线上就是**
//! `__CHANNEL__:<id>` 这个字符串，而 `InvokeResponseBody` 没有 `specta::Type`，
//! 生成器写不出准确的 TS 类型。
//!
//! 输入走普通 JSON：一次按键几个字节，没有吞吐问题；raw 请求体在 Android 上还不支持。

use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;

use akasha_core::{SessionId, SessionKind, SessionRegistry};
use akasha_pty::watchdog::SessionWatchdog;
use akasha_pty::{
    Batch, BatchPolicy, PtyTransport, TerminalSize, Transport, TransportError, spawn_batcher,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody, JavaScriptChannelId};
use tauri::{State, Webview};

/// 前端 raw 字节频道的句柄。
///
/// **线上就是一个字符串**（`__CHANNEL__:<id>`）：tauri 的频道是前端创建、只把 id 递给
/// 后端的东西。所以这里声明成"字符串"而不是 `Channel<Vec<u8>>` —— 后者在 JS 侧是
/// `number[]`（JSON 数组），是本文件开头说的那条慢路。
///
/// 校验在 Rust 侧：解析不出 `<id>` 就报 `IpcError::Channel`，不会静默丢掉输出。
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[specta(transparent)]
pub struct RawChannel(String);

/// 过 IPC 的**会话句柄**。
///
/// 为什么不直接用过线 `u64`：生成器拒绝把 `u64` 导出成 TS（BigInt 精度问题），
/// 而 shell 侧那个"危险地当 number 用"的全局开关，会让**将来每一个** `u64` 字段
/// 都悄悄失去保护。这里改成壳层自己的 `u32` 代理值，并**checked** 转换 ——
/// 截断是绝不允许的：那会把用户的按键送进**另一个会话**，而不是报错。
///
/// 与 `akasha_core::SessionId` 分开还有一层理由：core 刻意不许外部构造 id
/// （见其文档），而 IPC 边界天生要接收外部给的值 —— 于是"外部值"和"core 的 id"
/// 是两种东西，代理把它写在类型上。
pub type SessionHandle = u32;

/// IPC 边界的错误。
///
/// 域边界就在这里：`akasha-core` 与 `akasha-pty` **都不知道 IPC 存在**，所以它们的错误
/// 在这里收敛成一个可序列化的形状（`AGENTS.md` §3.4）。前端能据此区分的只有三件事：
/// 会话不存在 / 载体出错 / 频道句柄无效 —— 再细的分支要等真有 UI 依赖它时再加。
#[derive(Debug, thiserror::Error, Serialize, specta::Type)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum IpcError {
    /// 这个会话不存在：已经关闭，或从来没打开过。**两者的处置相同**，故不区分。
    #[error("会话 {handle} 不存在（已关闭或未打开）")]
    NotFound { handle: SessionHandle },

    /// 载体失败（IO / 能力不支持 / 已关闭）。消息原样来自 `TransportError`。
    #[error("载体失败：{message}")]
    Transport { message: String },

    /// 传给后端的东西不是合法的频道句柄。
    #[error("频道句柄无效：{message}")]
    Channel { message: String },

    /// 内部状态不可用（Mutex 中毒，或 core 的 id 装不进句柄）。
    /// **不是用户错误** —— 但也不能 `unwrap`。
    #[error("内部状态不可用：{message}")]
    Internal { message: String },
}

impl From<TransportError> for IpcError {
    fn from(err: TransportError) -> Self {
        Self::Transport {
            message: err.to_string(),
        }
    }
}

/// 一个**活着的**终端会话：注册表负责"它存在"，这里负责"它的字节怎么走"。
struct Live {
    /// 注册表里的名字。存在这里是为了 `close` 时能撤销登记 —— 不靠 `u64` 重建。
    id: SessionId,
    /// 载体。**装箱成 trait 对象**：一来 `shutdown_all` 要用假载体测调用序列
    /// （plan 0204 的验收），二来阶段 5/6 的 SSH / 隧道要装进同一个 map。
    transport: Box<dyn Transport>,
    /// 本地会话首进程 pid（看门狗的兜底凭据）。`None` = 这个载体没有本地进程。
    ///
    /// 存下来而不是每次现问 `transport`：`close` 与 `shutdown_all` 是在**收尾之后**
    /// 才去撤销登记的，那时载体已经收干净、pid 没了（`session_leader()` 会返回 `None`）。
    leader: Option<u32>,
}

#[derive(Default)]
struct Inner {
    registry: SessionRegistry,
    live: HashMap<SessionHandle, Live>,
}

/// 全部终端会话。由 tauri 作为 `State` 持有（`Send + Sync`）。
///
/// `inner` 是 `Arc` 的：**退出钩子与 panic hook 也要拿到同一份会话表**，而 tauri 的
/// `State` 只借给命令用（`AGENTS.md` §3.3：真正退出时必须能把子进程全收掉）。
/// `Clone` 出来的是同一个 `Arc`，不是两份表。
#[derive(Default, Clone)]
pub struct Sessions {
    inner: Arc<Mutex<Inner>>,
    /// 最后一道兜底：app **再也不跑代码**时（`tauri dev` 重载的 SIGKILL、`kill -9`），
    /// 由它拿会话首进程 pid 去收掉整个会话（plan 0205）。还没挂上 = 没起来
    /// （载体不支持、或启动失败）—— 那时退化回 plan 0204 的三条路径。
    ///
    /// 为什么是 `OnceLock` 而不是构造时定死：看门狗**最前面**起（会话一存在就必须
    /// 已经在兜底清单里），而它的日志要等到日志插件注册之后才有人看得到 —— 两件事
    /// 的时机不同，`OnceLock` 正好表达"只会被设置一次，读的人多"。
    watchdog: Arc<OnceLock<Arc<SessionWatchdog>>>,
}

/// 一次"全部回收"的结果。**给日志与测试用**，不是控制流。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ShutdownReport {
    /// 显式 kill + wait 收掉的会话数。
    pub shut_down: usize,
    /// 收不掉的会话（句柄 + 原因）。**退出路径上不抛异常** —— 抛出去只会把
    /// 证据和剩下的会话一起丢掉。
    pub failures: Vec<(SessionHandle, String)>,
}

impl ShutdownReport {
    /// 是否一个不剩、且没有失败。
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty()
    }
}

impl Sessions {
    /// 挂上看门狗（plan 0205）。
    ///
    /// `Sessions` 默认**没有**看门狗：单元测试和"看门狗起不来"的降级路径都走那一支，
    /// 那时行为就是 plan 0204 的三条路径（能跑代码的两条干净、SIGKILL 那条有残留）。
    ///
    /// 已经挂过就忽略这一次 —— 两个看门狗会各自收同一批会话，"收两次"至少是重复劳动，
    /// 更要紧的是它说明**有第三个地方在悄悄接另一条管道**，那必须被看见。
    pub fn attach_watchdog(&self, watchdog: SessionWatchdog) {
        if self.watchdog.set(Arc::new(watchdog)).is_err() {
            tracing::warn!("看门狗：已经挂过一个了，这一次忽略");
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, IpcError> {
        self.inner.lock().map_err(|err| IpcError::Internal {
            message: err.to_string(),
        })
    }

    /// 把"这个会话由看门狗兜底"写进协议。
    ///
    /// **失败只记日志**：看门狗是最后一道兜底，它坏了不该让用户开不了终端。
    /// 反过来说，这条路径上的失败会**静默地**把安全性退回 0204 的水平 —— 所以
    /// 日志级别是 `warn`，不是 `debug`。
    fn watch(&self, leader: Option<u32>) {
        let (Some(watchdog), Some(leader)) = (self.watchdog.get(), leader) else {
            return;
        };
        if let Err(err) = watchdog.watch(leader) {
            tracing::warn!(leader, %err, "看门狗：登记会话失败（这条会话少一道兜底）");
        }
    }

    /// 撤销登记。调用点必须排在这个会话**收尾之后**（理由见 `SessionWatchdog::forget`）。
    fn forget(&self, leader: Option<u32>) {
        let (Some(watchdog), Some(leader)) = (self.watchdog.get(), leader) else {
            return;
        };
        if let Err(err) = watchdog.forget(leader) {
            tracing::warn!(leader, %err, "看门狗：撤销登记失败（端到端退出时会再收一次）");
        }
    }

    /// 看门狗进程的 pid（诊断用：证明它真的起来了）。
    pub fn watchdog_pid(&self) -> Option<u32> {
        self.watchdog.get().and_then(|watchdog| watchdog.pid())
    }

    /// 登记一个会话，并把它的**批次流**交出来。
    ///
    /// 刻意不接 IPC：调用方决定 sink（命令接频道，测试接 `Vec`）。这样"会话管理"这段
    /// 逻辑可以脱离 app 测（`AGENTS.md` §3.1 的分层理由）。
    ///
    /// 返回的句柄是**过 IPC 的表示**（[`SessionHandle`]）：`SessionId` 不许在
    /// `akasha-core` 之外构造（那是它的设计意图），所以壳层只用它的 checked 投影，
    /// 真伪由注册表判定 —— 不存在的句柄一律 [`IpcError::NotFound`]。
    pub fn register<T: Transport + 'static>(
        &self,
        mut transport: T,
        policy: BatchPolicy,
    ) -> Result<(SessionHandle, Receiver<Batch>), IpcError> {
        // 先取走读端：载体允许读端只被取走一次（防两个读端互相偷字节）。
        let output = transport
            .output_stream()
            .ok_or_else(|| IpcError::Transport {
                message: "读端已被取走".into(),
            })?;

        // 登记去**前面**、锁**外面**：
        //   * 前面 —— 会话一旦存在就必须已经在兜底清单里。反过来的话，"登记完、
        //     还没告诉看门狗"这一瞬 app 死了，那个 shell 就没人管了；
        //   * 外面 —— 写管道是可能阻塞的（看门狗被暂停住、管道写满），
        //     握着会话表的锁阻塞会冻住全部会话命令。
        // 多登记一次是安全的：看门狗收的是**会话**，重复收同一个 pid 不会误伤别人。
        let leader = transport.session_leader();
        self.watch(leader);

        let mut inner = self.lock()?;
        let id = inner
            .registry
            .open(SessionKind::Terminal)
            .map_err(|err| IpcError::Internal {
                message: err.to_string(),
            })?;
        let handle = Self::handle(id)?;
        inner.live.insert(
            handle,
            Live {
                id,
                transport: Box::new(transport),
                leader,
            },
        );
        // 锁只保护"谁存在"，读循环在锁外跑 —— 否则一次阻塞的读就冻住整个 app。
        drop(inner);
        Ok((handle, spawn_batcher(output, policy)))
    }

    /// 把用户输入送进载体。
    pub fn write(&self, handle: SessionHandle, bytes: &[u8]) -> Result<(), IpcError> {
        let mut inner = self.guard_for(handle)?;
        inner
            .live
            .get_mut(&handle)
            .ok_or(IpcError::NotFound { handle })?
            .transport
            .write(bytes)?;
        Ok(())
    }

    /// 调整窗口尺寸（"尽力"语义 —— 载体可以拒绝，见 `akasha_pty` 的能力位）。
    pub fn resize(&self, handle: SessionHandle, size: TerminalSize) -> Result<(), IpcError> {
        let mut inner = self.guard_for(handle)?;
        inner
            .live
            .get_mut(&handle)
            .ok_or(IpcError::NotFound { handle })?
            .transport
            .resize(size)?;
        Ok(())
    }

    /// 关闭会话：**显式 kill + wait 收尸**（`AGENTS.md` §3.3），再从注册表摘牌。
    ///
    /// 顺序是有意的：先收尸再摘牌。反过来的话，`shutdown` 失败就会留下一个
    /// "查不到、但还活着"的子进程 —— 那正是这一节要防的残留。
    ///
    /// 已经关过的 id 再关一次报 [`IpcError::NotFound`]（幂等性由调用方决定，
    /// 这里不假装成功）。
    pub fn close(&self, handle: SessionHandle) -> Result<(), IpcError> {
        let mut inner = self.lock()?;
        let mut live = inner
            .live
            .remove(&handle)
            .ok_or(IpcError::NotFound { handle })?;
        live.transport.shutdown()?;
        inner
            .registry
            .close(live.id)
            .map_err(|err| IpcError::Internal {
                message: err.to_string(),
            })?;
        // 撤销登记排在**收尾之后**（`SessionWatchdog::forget` 的理由），且放在**锁外**
        // —— 写管道可能阻塞，握着会话表的锁阻塞会冻住全部会话命令。
        // 收尾失败时上面已经 `?` 返回了：那时**不撤销**，让看门狗下次再试一遍。
        drop(inner);
        self.forget(live.leader);
        Ok(())
    }

    /// 当前活着的会话数（测试与将来的诊断用）。
    pub fn len(&self) -> usize {
        self.lock().map(|inner| inner.live.len()).unwrap_or(0)
    }

    /// 注册表里登记着的会话数（诊断用）。它和 [`Self::len`] **必须一致** ——
    /// 两张表分叉就说明有会话"查得到、却没人管"（或反过来）。
    pub fn registered(&self) -> usize {
        self.lock().map(|inner| inner.registry.len()).unwrap_or(0)
    }

    /// **真正退出时**把全部会话收掉：逐个显式 `shutdown()`（kill + wait 收尸），再摘牌。
    ///
    /// 为什么不能靠 `Drop`（`AGENTS.md` §3.3）：进程退出时析构**不保证执行**，
    /// 而"谁负责收尸"一旦交给析构，就再也分不清"收干净了"与"忘了收"。
    ///
    /// 顺序与理由：
    ///
    /// 1. **先把清单整份搬出来，再逐个收**：收一个会话会 `wait`（阻塞），
    ///    握着锁 wait 会让并发的命令一起卡住（甚至死锁）。
    /// 2. 逐个 `shutdown()` —— 载体自己保证"kill + wait 收尸"，PTY 还会额外把
    ///    **整个会话**收掉（见 `akasha_pty` 的 `teardown`）。
    /// 3. 收尾再摘牌：`registry` 与 `live` 两张表一起清空，不留"查得到但已经死了"的登记。
    ///
    /// 幂等：第二次调用时清单已经空了，什么都不做（退出路径可能被触发多次）。
    pub fn shutdown_all(&self) -> ShutdownReport {
        let mut report = ShutdownReport::default();

        let drained: Vec<(SessionHandle, Live)> = match self.inner.lock() {
            Ok(mut inner) => inner.live.drain().collect(),
            Err(err) => {
                // 中毒：拿不到清单。**不 panic** —— 退出路径上 panic 会把证据一起丢掉。
                report.failures.push((0, format!("会话表不可用：{err}")));
                return report;
            }
        };

        let mut ids = Vec::with_capacity(drained.len());
        for (handle, mut live) in drained {
            match live.transport.shutdown() {
                Ok(status) => {
                    report.shut_down += 1;
                    // 收干净了才撤销登记：失败的那些留着，让看门狗在 EOF 时再试一次。
                    self.forget(live.leader);
                    tracing::info!(
                        handle,
                        session = live.id.get(),
                        ?status,
                        "退出：会话已显式回收"
                    );
                }
                Err(err) => {
                    tracing::error!(handle, session = live.id.get(), %err, "退出：会话回收失败");
                    report.failures.push((handle, err.to_string()));
                }
            }
            // 回收失败**也要摘牌**：留着它只会在退出路径上被重复失败一遍。
            ids.push(live.id);
        }

        if let Ok(mut inner) = self.inner.lock() {
            for id in ids {
                if let Err(err) = inner.registry.close(id) {
                    tracing::warn!(session = id.get(), %err, "退出：撤销登记失败");
                }
            }
        }

        report
    }

    /// 是否一个会话都没有。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// core 的 id → 过 IPC 的句柄。装不下就报 [`IpcError::Internal`]，
    /// **绝不截断**（截断会把按键送进另一个会话）。
    fn handle(id: SessionId) -> Result<SessionHandle, IpcError> {
        SessionHandle::try_from(id.get()).map_err(|_| IpcError::Internal {
            message: format!("会话 id {} 超出句柄范围", id.get()),
        })
    }

    fn guard_for(&self, handle: SessionHandle) -> Result<MutexGuard<'_, Inner>, IpcError> {
        // 借用检查器不允许"守卫 + 它的字段"一起返回，所以这里返回守卫，
        // 调用方自己取 `live.get_mut(&handle)`。查找失败就是 NotFound。
        let inner = self.lock()?;
        if inner.live.contains_key(&handle) {
            Ok(inner)
        } else {
            Err(IpcError::NotFound { handle })
        }
    }
}

/// 转发循环：把批次一个个交给 `sink`，源结束（EOF / 读错误 / 载体关闭）就收工。
///
/// 单独提出来只为一个理由：**它必须能脱离 Tauri 测**。命令里的 sink 是 IPC 频道，
/// 测试里的 sink 是一个 `Vec` —— 同一段代码，两种接法。
pub fn forward(
    batches: Receiver<Batch>,
    mut sink: impl FnMut(Batch) + Send + 'static,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for batch in batches {
            sink(batch);
        }
    })
}

/// 打开一个终端会话，输出经 `channel` 以 **raw 字节**送出。
///
/// 返回的 id 是前端后续 `write_session` / `resize_session` / `close_session` 要用的句柄。
#[tauri::command]
#[specta::specta]
pub fn open_session(
    webview: Webview,
    channel: RawChannel,
    state: State<'_, Sessions>,
) -> Result<SessionHandle, IpcError> {
    let channel: Channel<InvokeResponseBody> = channel
        .0
        .parse::<JavaScriptChannelId>()
        .map_err(|message| IpcError::Channel {
            message: message.to_string(),
        })?
        .channel_on(webview);

    let transport = PtyTransport::spawn_default(TerminalSize::DEFAULT)?;
    let (handle, batches) = state.register(transport, BatchPolicy::DEFAULT)?;

    forward(batches, move |batch| {
        // 发不出去只可能是前端已经走了（频道已 drop）—— 那条流没有意义了，
        // 但**不要**在这里关会话：关不关由用户/退出路径决定（plan 0204）。
        let _ = channel.send(InvokeResponseBody::Raw(batch.bytes));
    });

    Ok(handle)
}

/// 把用户输入（按键字节）送进会话。
#[tauri::command]
#[specta::specta]
pub fn write_session(
    handle: SessionHandle,
    data: Vec<u8>,
    state: State<'_, Sessions>,
) -> Result<(), IpcError> {
    state.write(handle, &data)
}

/// 调整会话的窗口尺寸（列 / 行）。
#[tauri::command]
#[specta::specta]
pub fn resize_session(
    handle: SessionHandle,
    cols: u16,
    rows: u16,
    state: State<'_, Sessions>,
) -> Result<(), IpcError> {
    state.resize(handle, TerminalSize::new(cols, rows))
}

/// 关闭会话：显式 kill + wait 收尸，之后这个 id 不再有效。
#[tauri::command]
#[specta::specta]
pub fn close_session(handle: SessionHandle, state: State<'_, Sessions>) -> Result<(), IpcError> {
    state.close(handle)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use akasha_pty::ExitStatus;
    use akasha_pty::ShellLaunch;
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::{Duration, Instant};

    /// 起一个跑探测命令的 `/bin/sh`：不碰用户登录 shell，结果才可复现。
    fn sh() -> PtyTransport {
        PtyTransport::spawn(&ShellLaunch::new("/bin/sh"), TerminalSize::DEFAULT)
            .expect("spawn /bin/sh 失败")
    }

    /// 在**截止时间**内把批次攒到出现 `needle` 为止。
    ///
    /// 不用固定 sleep（`AGENTS.md` §0 第 5 条）：shell 启动耗时随机器变，
    /// 猜一个 sleep 要么慢要么 flaky。
    fn read_until(batches: &Receiver<Batch>, needle: &[u8], timeout: Duration) -> Vec<u8> {
        let deadline = Instant::now() + timeout;
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            match batches.recv_timeout(Duration::from_millis(100)) {
                Ok(batch) => {
                    seen.extend_from_slice(&batch.bytes);
                    if seen.windows(needle.len()).any(|w| w == needle) {
                        return seen;
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        seen
    }

    #[test]
    fn a_real_shell_round_trips_through_the_session() {
        let sessions = Sessions::default();
        let (id, batches) = sessions
            .register(sh(), BatchPolicy::DEFAULT)
            .expect("登记会话失败");
        assert_eq!(sessions.len(), 1);

        sessions
            .write(id, b"echo akasha-batch-probe\n")
            .expect("写入失败");

        let seen = read_until(&batches, b"akasha-batch-probe", Duration::from_secs(10));
        assert!(
            seen.windows(15).any(|w| w == b"akasha-batch-pr"),
            "命令回显必须原样出来（拿到 {} 字节）",
            seen.len()
        );

        sessions.close(id).expect("关闭失败");
        assert!(sessions.is_empty(), "关闭后不得留下登记");
    }

    #[test]
    fn unknown_ids_are_not_found_not_panics() {
        let sessions = Sessions::default();

        assert!(matches!(
            sessions.write(42, b"x"),
            Err(IpcError::NotFound { handle: 42 })
        ));
        assert!(matches!(
            sessions.resize(42, TerminalSize::DEFAULT),
            Err(IpcError::NotFound { handle: 42 })
        ));
        assert!(matches!(
            sessions.close(42),
            Err(IpcError::NotFound { handle: 42 })
        ));
    }

    #[test]
    fn closing_twice_reports_not_found() {
        let sessions = Sessions::default();
        let (id, _batches) = sessions
            .register(sh(), BatchPolicy::DEFAULT)
            .expect("登记会话失败");

        sessions.close(id).expect("第一次关闭应当成功");
        assert!(
            matches!(sessions.close(id), Err(IpcError::NotFound { .. })),
            "重复关闭必须报 NotFound，而不是假装成功"
        );
    }

    #[test]
    fn resize_reaches_the_transport() {
        let sessions = Sessions::default();
        let (id, _batches) = sessions
            .register(sh(), BatchPolicy::DEFAULT)
            .expect("登记会话失败");

        sessions
            .resize(id, TerminalSize::new(120, 40))
            .expect("PTY 支持 resize，应当成功");
    }

    #[test]
    fn every_batch_reaches_the_sink_in_order() {
        // 纯逻辑那半边：用假载体跑一遍 forward，确认一个字节都不丢、顺序不变。
        // （没有 Tauri、没有真实进程 —— 这正是把转发单独提出来测的价值。）
        use akasha_pty::Capabilities;
        use akasha_pty::testing::FakeTransport;

        let mut transport = FakeTransport::new(Capabilities::PTY, TerminalSize::DEFAULT);
        let output = transport.output_stream().expect("取输出流失败");
        let batches = spawn_batcher(output, BatchPolicy::DEFAULT);

        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        forward(batches, move |batch| {
            let _ = tx.send(batch.bytes);
        });

        transport.feed(b"first ");
        transport.feed(b"second");
        transport.shutdown().expect("shutdown 失败"); // 关写端 = EOF → 残批交付

        let mut got = Vec::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(5)) {
            got.extend_from_slice(&chunk);
        }
        assert_eq!(got, b"first second");
    }

    // ── plan 0204：退出路径的显式回收 ────────────────────────────────────────

    /// 记流水账的假载体。存在的理由：`shutdown_all` 要验的是**调用序列**
    /// （每个会话恰好一次 shutdown、之后句柄失效），而这在有真进程时看不清楚。
    struct Recording {
        log: Arc<Mutex<Vec<String>>>,
        name: &'static str,
        fail: bool,
        closed: bool,
        /// 本地会话首进程 pid（看门狗兜底凭据）。`None` = 这个假载体没有本地进程。
        leader: Option<u32>,
    }

    impl Recording {
        fn new(log: &Arc<Mutex<Vec<String>>>, name: &'static str, fail: bool) -> Self {
            Self {
                log: Arc::clone(log),
                name,
                fail,
                closed: false,
                leader: None,
            }
        }

        /// 让这个假载体看起来像"有一个本地会话"。
        fn leader(mut self, leader: u32) -> Self {
            self.leader = Some(leader);
            self
        }
    }

    impl Transport for Recording {
        fn write(&mut self, _bytes: &[u8]) -> Result<(), TransportError> {
            if self.closed {
                return Err(TransportError::Closed);
            }
            Ok(())
        }

        fn session_leader(&self) -> Option<u32> {
            self.leader
        }

        fn output_stream(&mut self) -> Option<Box<dyn std::io::Read + Send>> {
            // 立刻 EOF 的读端：合批线程马上收工，测试不必再管它。
            Some(Box::new(std::io::empty()))
        }

        fn shutdown(&mut self) -> Result<Option<ExitStatus>, TransportError> {
            self.log
                .lock()
                .expect("流水账锁中毒")
                .push(format!("{}:shutdown", self.name));
            self.closed = true;
            if self.fail {
                return Err(TransportError::Unsupported("shutdown"));
            }
            Ok(Some(ExitStatus::Code(0)))
        }
    }

    fn shutdown_count(log: &Arc<Mutex<Vec<String>>>) -> usize {
        log.lock()
            .expect("流水账锁中毒")
            .iter()
            .filter(|line| line.ends_with(":shutdown"))
            .count()
    }

    #[test]
    fn shutdown_all_collects_every_session_exactly_once() {
        let sessions = Sessions::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (first, _a) = sessions
            .register(Recording::new(&log, "a", false), BatchPolicy::DEFAULT)
            .expect("登记 a 失败");
        let (second, _b) = sessions
            .register(Recording::new(&log, "b", false), BatchPolicy::DEFAULT)
            .expect("登记 b 失败");
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions.registered(), 2, "两张表必须同步");

        let report = sessions.shutdown_all();

        assert_eq!(report.shut_down, 2, "两个会话都该被显式收掉");
        assert!(report.is_clean(), "不该有失败：{:?}", report.failures);
        assert_eq!(shutdown_count(&log), 2, "每个会话**恰好**收一次");
        assert!(
            sessions.is_empty() && sessions.registered() == 0,
            "两张表一起清空"
        );
        assert!(
            matches!(sessions.close(first), Err(IpcError::NotFound { .. })),
            "收完之后旧句柄必须失效"
        );
        assert!(matches!(
            sessions.write(second, b"x"),
            Err(IpcError::NotFound { .. })
        ));

        // 幂等：退出路径可能被触发不止一次（关窗口 + 进程退出），第二次必须是空操作。
        let again = sessions.shutdown_all();
        assert_eq!(again.shut_down, 0);
        assert_eq!(shutdown_count(&log), 2, "重复调用不得再收一遍");
    }

    #[test]
    fn a_failing_shutdown_is_reported_and_still_unregistered() {
        let sessions = Sessions::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (bad, _bad_rx) = sessions
            .register(Recording::new(&log, "bad", true), BatchPolicy::DEFAULT)
            .expect("登记失败");
        let (_good, _good_rx) = sessions
            .register(Recording::new(&log, "good", false), BatchPolicy::DEFAULT)
            .expect("登记失败");

        let report = sessions.shutdown_all();

        assert_eq!(report.shut_down, 1);
        assert_eq!(report.failures.len(), 1, "失败必须被**报出来**，不是吞掉");
        assert_eq!(report.failures[0].0, bad);
        assert!(!report.is_clean());
        // 收不掉也摘牌：留着它只会在退出路径上被重复失败一遍。
        assert!(sessions.is_empty() && sessions.registered() == 0);
    }

    // ── plan 0205：看门狗（app 被 SIGKILL 时的最后一道兜底）────────────────────

    /// 把看门狗的控制端接到一段内存上，好断言协议到底写了什么。
    ///
    /// 为什么要断言**字节**而不是"调用过"：协议是**跨进程**的约定，两端各自编译、
    /// 各自演进 —— 只有把写出去的行与 `akasha_pty::watchdog::parse` 对上，
    /// 才能保证"app 说登记了"与"看门狗听懂了"是同一件事。
    #[derive(Clone)]
    struct ControlLog(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for ControlLog {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("控制端锁中毒").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// 每次写都失败的看门狗（进程没了、管道断了的样子）。
    struct DeadControl;

    impl std::io::Write for DeadControl {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("看门狗不在了"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn watchdog_with(log: &ControlLog) -> (Sessions, Arc<Mutex<Vec<u8>>>) {
        let sessions = Sessions::default();
        sessions.attach_watchdog(SessionWatchdog::with_writer(Box::new(log.clone())));
        (sessions, Arc::clone(&log.0))
    }

    fn control_lines(written: &Arc<Mutex<Vec<u8>>>) -> Vec<String> {
        let bytes = written.lock().expect("控制端锁中毒").clone();
        String::from_utf8(bytes)
            .expect("协议必须是 ASCII")
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn the_watchdog_is_told_exactly_which_sessions_to_collect() {
        let log = ControlLog(Arc::new(Mutex::new(Vec::new())));
        // 两个流水账：`trace` 记载体被怎么用了（plan 0204 的假载体），
        // `written` 记协议写出去什么（本 plan）。混在一起会分不清是哪一边的事。
        let trace = Arc::new(Mutex::new(Vec::new()));
        let (sessions, written) = watchdog_with(&log);

        let (first, _a) = sessions
            .register(
                Recording::new(&trace, "a", false).leader(1001),
                BatchPolicy::DEFAULT,
            )
            .expect("登记 a 失败");
        let (_second, _b) = sessions
            .register(
                Recording::new(&trace, "b", false).leader(1002),
                BatchPolicy::DEFAULT,
            )
            .expect("登记 b 失败");
        assert_eq!(
            control_lines(&written),
            vec!["+1001", "+1002"],
            "会话一存在就必须已经在兜底清单里"
        );

        sessions.close(first).expect("关闭 a 失败");
        assert_eq!(
            control_lines(&written),
            vec!["+1001", "+1002", "-1001"],
            "收干净的会话要撤销登记（否则端到端退出时会对着复用掉的 pid 再发一次 SIGKILL）"
        );

        sessions.shutdown_all();
        assert_eq!(
            control_lines(&written),
            vec!["+1001", "+1002", "-1001", "-1002"]
        );
    }

    #[test]
    fn a_session_without_a_local_process_is_not_registered() {
        // 内存载体 / 将来的纯网络后端没有本地进程可收：不能凭空登记一个 pid。
        let log = ControlLog(Arc::new(Mutex::new(Vec::new())));
        let trace = Arc::new(Mutex::new(Vec::new()));
        let (sessions, written) = watchdog_with(&log);

        sessions
            .register(Recording::new(&trace, "a", false), BatchPolicy::DEFAULT)
            .expect("登记失败");
        sessions.shutdown_all();

        assert!(
            control_lines(&written).is_empty(),
            "没有本地进程的载体不该产生任何协议行"
        );
    }

    #[test]
    fn a_dead_watchdog_does_not_take_the_sessions_with_it() {
        // 看门狗是**最后一道**兜底，不是主路径：它坏了，终端必须照常能用
        // （退化回 plan 0204 的水平），只是少了一层保险。
        let sessions = Sessions::default();
        sessions.attach_watchdog(SessionWatchdog::with_writer(Box::new(DeadControl)));

        let trace = Arc::new(Mutex::new(Vec::new()));
        let (handle, _batches) = sessions
            .register(
                Recording::new(&trace, "a", false).leader(7),
                BatchPolicy::DEFAULT,
            )
            .expect("看门狗写不进去，也不该让会话开不了");
        sessions.close(handle).expect("关闭失败");
        assert!(sessions.is_empty());
    }

    #[test]
    fn a_failed_collection_keeps_the_session_registered_for_the_watchdog() {
        // 收不掉的会话**不撤销**：留着它，让看门狗在 app 真的死掉时再试一次。
        // 反过来的话，唯一一次机会被一个瞬时错误花掉了，进程永远留在用户机器上。
        let log = ControlLog(Arc::new(Mutex::new(Vec::new())));
        let trace = Arc::new(Mutex::new(Vec::new()));
        let (sessions, written) = watchdog_with(&log);

        sessions
            .register(
                Recording::new(&trace, "bad", true).leader(1003),
                BatchPolicy::DEFAULT,
            )
            .expect("登记失败");
        let report = sessions.shutdown_all();

        assert_eq!(report.failures.len(), 1);
        assert_eq!(
            control_lines(&written),
            vec!["+1003"],
            "失败的会话不得被撤销登记"
        );
    }
}
