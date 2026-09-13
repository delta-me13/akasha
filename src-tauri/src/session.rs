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
    Batch, BatchPolicy, ExitStatus, PtyTransport, TerminalSize, Transport, TransportError,
    spawn_batcher,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody, JavaScriptChannelId};
use tauri::{AppHandle, Emitter, State, Webview};
use tauri_specta::Event;

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

impl RawChannel {
    /// 把线上那个字符串还原成一条**能收 raw 字节**的频道。
    ///
    /// 校验在这里（解析不出就报 [`IpcError::Channel`]），本地终端与 SSH 两条路共用 ——
    /// 两条路各写一遍的话，"哪一种手柄是合法的"就会有两份说法。
    pub(crate) fn into_channel(
        self,
        webview: Webview,
    ) -> Result<Channel<InvokeResponseBody>, IpcError> {
        Ok(self
            .0
            .parse::<JavaScriptChannelId>()
            .map_err(|message| IpcError::Channel {
                message: message.to_string(),
            })?
            .channel_on(webview))
    }
}

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
    /// 会话集合变化时的通知（plan 0301）：托盘靠它刷新隧道列表。
    ///
    /// 为什么不是 tauri 事件：这条信号只在**进程内**用 —— 前端不需要知道"表变了"
    /// （它有自己的 `session_ended`），把内部表的变更广播给 webview 只会多一份要维护的契约。
    /// 反过来说，也**不要**在每个变更点手动调一次托盘：那会把"谁在订阅"散落到各处。
    changed: Arc<OnceLock<Arc<dyn Fn() + Send + Sync>>>,
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

/// 一个会话**自己**结束了：载体（PTY 里的 shell）退出 —— 用户敲了 `exit`、shell 崩了、
/// PTY 被关掉。总之**不是**"用户关掉了标签页"那条路。
///
/// 前端据此关掉对应的标签页：标签页与会话**同生命期**（`docs/scope.md` §5.6），两个方向都要
/// 成立 —— 关标签页 → 丢弃会话（plan 0305）；会话自己走 → 标签页跟着走（本步）。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionEnded {
    /// 哪个会话结束了。前端按它找要关掉的那个标签页。
    pub handle: SessionHandle,
    /// 结局的可读描述（`None` = 这个载体不报结局，或收尾时出了岔子 —— 见 `retire`）。
    pub status: Option<String>,
}

// 为什么**手写**这个 impl 而不是 `#[derive(tauri_specta::Event)]`：derive 来自
// `tauri-specta-macros`，只在 `derive` 特性下被引入 —— 为一个"一个常量 + 其余全默认方法"
// 的 trait 多拉一个 proc-macro 依赖（还要过 deny 的许可证门禁）不划算。trait 的其余方法
// 都有默认实现，所以这里写的就是全部。
impl tauri_specta::Event for SessionEnded {
    const NAME: &'static str = "session_ended";
}

/// [`Sessions::retire`] 的产物。
#[derive(Debug)]
pub struct Retired {
    /// 会话在注册表里的名字。
    pub id: SessionId,
    /// 载体的结局（`None` = 载体不报结局，或收尸失败 —— 后者只记日志）。
    pub status: Option<ExitStatus>,
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
            tracing::warn!(reason = "already-attached", "watchdog attach ignored");
        }
    }

    /// 登记"会话集合变了"的通知。托盘是**唯一**的订阅者（plan 0301）。
    ///
    /// 只接受一个订阅者，第二个被忽略并记日志 —— 同 [`Self::attach_watchdog`] 的理由：
    /// 它说明有两处在悄悄改同一份 UI 状态，那必须被看见。
    pub fn on_change(&self, hook: impl Fn() + Send + Sync + 'static) {
        if self.changed.set(Arc::new(hook)).is_err() {
            tracing::warn!(reason = "already-registered", "change hook ignored");
        }
    }

    /// 通知订阅者"表变了"。
    ///
    /// ⚠️ **必须在放掉会话表的锁之后调用**：订阅者会回头读这张表（托盘要列隧道），
    /// 握着锁通知就是自己等自己。调用点因此都排在 `drop(inner)` 之后。
    fn notify_changed(&self) {
        if let Some(hook) = self.changed.get() {
            hook();
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
            tracing::warn!(leader, %err, "watchdog registration failed");
        }
    }

    /// 撤销登记。调用点必须排在这个会话**收尾之后**（理由见 `SessionWatchdog::forget`）。
    fn forget(&self, leader: Option<u32>) {
        let (Some(watchdog), Some(leader)) = (self.watchdog.get(), leader) else {
            return;
        };
        if let Err(err) = watchdog.forget(leader) {
            tracing::warn!(leader, %err, "watchdog deregistration failed");
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
        self.notify_changed();
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
        self.notify_changed();
        Ok(())
    }

    /// 一个会话**自己结束了**（载体的输出流断了：用户在终端里敲了 `exit`、shell 崩了、
    /// PTY 被关掉）：收尾 + 摘牌 + 撤销兜底登记，并交出它的结局。
    ///
    /// 与 [`Self::close`] 的关系：**做的事一样**（显式 kill + wait 收尸、摘牌、撤销登记），
    /// 区别只在**触发者**与**调用点** —— `close` 是"用户要求关"（IPC 命令，要报错给前端），
    /// `retire` 是"它自己走了"（后台线程观测到输出流结束，没有人在等结果）。
    ///
    /// 因此两条性质是必须的：
    ///
    /// * **幂等**：用户点 `×` 与 shell 自己退出可能几乎同时发生，两边都会走到这里。
    ///   已经摘过牌就返回 `Ok(None)`，不报错 —— 这不是失败，是"已经有人收过了"。
    /// * **不半途而废**：它已经结束了，任何一步失败都只记日志、继续把牌摘掉。
    ///   留着一条"查不到、但还活着"的登记，比收尸失败本身糟得多。
    pub fn retire(&self, handle: SessionHandle) -> Result<Option<Retired>, IpcError> {
        let mut inner = self.lock()?;
        let Some(mut live) = inner.live.remove(&handle) else {
            return Ok(None); // 已经被人收走了（用户关标签页 / 退出路径）—— 幂等
        };

        // 收尸：载体其实已经结束了，这一步是为 `wait`（不留僵尸）与"会话里还有别人"兜底
        // —— 例如用户 `exit` 了、但会话里还有一个忽略 SIGHUP 的后台作业。
        let status = match live.transport.shutdown() {
            Ok(status) => status,
            Err(err) => {
                tracing::warn!(handle, session = live.id.get(), %err, "session reap failed");
                None
            }
        };

        if let Err(err) = inner.registry.close(live.id) {
            tracing::warn!(handle, session = live.id.get(), %err, "session unregister failed");
        }
        // 撤销看门狗登记排在**收尾之后**（`SessionWatchdog::forget` 的理由），且在**锁外**
        // —— 写管道可能阻塞。
        drop(inner);
        self.forget(live.leader);
        self.notify_changed();

        Ok(Some(Retired {
            id: live.id,
            status,
        }))
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

    /// 当前登记着的**隧道类**会话，升序（plan 0301：托盘菜单要列它们）。
    ///
    /// 阶段 6 之前**必然是空的** —— 那时隧道才存在。这一步先把"列表从哪来"定下来：
    /// **只读注册表**，不另立一张表（两张表必然分叉，`registered` 的注释里已写过这条）。
    pub fn tunnels(&self) -> Vec<SessionId> {
        let inner = match self.lock() {
            Ok(inner) => inner,
            // 中毒时**不假装空列表**：菜单里少一项的后果比"日志里说清楚"小，但谎报
            // "一条都没有"会让排查的人往错的方向找（`docs/logging.md` 的字段值不撒谎）。
            Err(err) => {
                tracing::warn!(%err, "session table unavailable");
                return Vec::new();
            }
        };
        inner
            .registry
            .ids()
            .filter(|id| inner.registry.kind(*id) == Some(SessionKind::Tunnel))
            .collect()
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
                // 这条失败在下面没有对应的会话，所以**在这里**记：退出路径的汇总行只报数量。
                tracing::error!(%err, "session table unavailable");
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
                    log_ended("session reclaimed", handle, live.id, &status);
                }
                Err(err) => {
                    tracing::error!(handle, session = live.id.get(), %err, "session reclaim failed");
                    report.failures.push((handle, err.to_string()));
                }
            }
            // 回收失败**也要摘牌**：留着它只会在退出路径上被重复失败一遍。
            ids.push(live.id);
        }

        if let Ok(mut inner) = self.inner.lock() {
            for id in ids {
                if let Err(err) = inner.registry.close(id) {
                    tracing::warn!(session = id.get(), %err, "session unregister failed");
                }
            }
        }

        self.notify_changed();
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

/// 会话表的**只读快照**（Victauri probe `sessions`，plan 0504）。
///
/// 为什么需要它：SSH 会话**没有本地进程**（ADR-0003 D4：`session_leader()` 永远是 `None`），
/// 所以"关标签页之后零残留"这句话在进程表上**看不到** —— 能看到的只有注册表。
/// 靠 grep 日志反推"会话没了"是不行的（`AGENTS.md` §7），于是把内部状态摆出来读。
///
/// 返回的两个数**必须相等**才有意义：`live` 是"谁在跑"，`registered` 是"注册表里登记着谁"
/// —— 两张表分叉就说明有会话"查得到、却没人管"（或反过来）。
pub fn snapshot(sessions: &Sessions) -> serde_json::Value {
    serde_json::json!({
        "live": sessions.len(),
        "registered": sessions.registered(),
    })
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
///
/// 会话**自己结束**时（用户在终端里敲了 `exit`、shell 崩了、PTY 被关掉）由一条收尾线程
/// 负责：收掉它 + 发 [`SessionEnded`] 让前端关掉那个标签页。
#[tauri::command]
#[specta::specta]
pub fn open_session(
    app: AppHandle,
    webview: Webview,
    channel: RawChannel,
    state: State<'_, Sessions>,
) -> Result<SessionHandle, IpcError> {
    let channel = channel.into_channel(webview)?;
    let transport = PtyTransport::spawn_default(TerminalSize::DEFAULT)?;
    open_terminal(&app, &state, transport, channel)
}

/// 注册一个载体、把它的批次流接上频道，并起一条收尾线程。**两条载体共用这一段。**
///
/// 抽出来的理由：本地 PTY 与 SSH 会话在"注册之后"的每一件事都相同（句柄、频道、
/// `session_ended`、收尾），差的只是载体从哪来。抄第二份的代价不是几行代码，
/// 而是**收尾与会话结束的语义会有两种写法**（那正是"敲了 `exit` 标签页不关"这类 bug 的家）。
pub(crate) fn open_terminal(
    app: &AppHandle,
    sessions: &Sessions,
    transport: impl Transport + 'static,
    channel: Channel<InvokeResponseBody>,
) -> Result<SessionHandle, IpcError> {
    let (handle, batches) = sessions.register(transport, BatchPolicy::DEFAULT)?;

    let pump = forward(batches, move |batch| {
        // 发不出去只可能是前端已经走了（频道已 drop）—— 那条流没有意义了，
        // 但**不要**在这里关会话：关不关由用户/退出路径决定（plan 0204）。
        let _ = channel.send(InvokeResponseBody::Raw(batch.bytes));
    });

    // 输出流结束 = 这个会话自己结束了。为什么要**等 `forward` 收工**再去收尾：
    // 它结束就意味着最后一批已经交给频道了 —— 收尾（以及随后的 `SessionEnded` 事件）
    // 排在那之后，前端才不会"先收到会话结束、后收到最后一段输出"。
    //
    // 为什么不在原地收尾：`forward` 的线程要在流结束的那一瞬退出，而收尾里有一次
    // `wait`（收尸）—— 混在一起会让"最后一批送达"被收尾时长拖住。
    let sessions = Sessions::clone(sessions);
    let app = app.clone();
    std::thread::Builder::new()
        .name("akasha-session-retire".into())
        .spawn(move || {
            let _ = pump.join();
            retire_and_report(&sessions, &app, handle);
        })
        .map_err(|err| IpcError::Internal {
            message: format!("收尾线程起不来：{err}"),
        })?;

    Ok(handle)
}

/// 会话结束后要做的两件事：**后端收尾** + **告诉前端**。
///
/// 抽成函数只是为了让收尾线程的本体保持三行；真正的语义在 [`Sessions::retire`] 里
/// （幂等、不半途而废），那部分有单测。
fn retire_and_report(sessions: &Sessions, app: &AppHandle, handle: SessionHandle) {
    let retired = match sessions.retire(handle) {
        Ok(Some(retired)) => retired,
        // 已经被人收走了（用户点 × / 退出路径）—— 静默：这不是异常，是两条路撞在一起。
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(handle, %err, "session retire failed");
            return;
        }
    };

    log_ended("session retired", handle, retired.id, &retired.status);

    let status = retired.status.map(|status| status.to_string());
    let event = SessionEnded { handle, status };
    if let Err(err) = app.emit(SessionEnded::NAME, event) {
        // 前端可能已经走了（窗口销毁 / webview 没了）。后端该收的已经收完了，
        // 发不出去不影响"零残留"这条判据。
        tracing::warn!(event = SessionEnded::NAME, %err, "event emit failed");
    }
}

/// 会话收尾成功的一条记录：**消息是常量，结局进字段**。
///
/// 结局分两支（正常退出码 / 被信号终止），字段也跟着分两支 —— [`ExitStatus`] 的
/// `Display` 是给用户看的中文、`Debug` 会带上 `Some(ExitStatus::Code(..))` 包装，
/// 两个都不适合当日志字段。形态规则见 `docs/logging.md`。
fn log_ended(
    event: &'static str,
    handle: SessionHandle,
    session: SessionId,
    status: &Option<ExitStatus>,
) {
    match status {
        Some(ExitStatus::Code(code)) => {
            tracing::info!(
                handle,
                session = session.get(),
                exit_code = *code,
                "{event}"
            );
        }
        Some(ExitStatus::Signal(signal)) => {
            tracing::info!(handle, session = session.get(), signal = %signal, "{event}");
        }
        None => tracing::info!(handle, session = session.get(), "{event}"),
    }
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
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    // ── plan 0306：会话**自己**结束（敲 exit）时的收尾 ──────────────────────

    #[test]
    fn retiring_a_self_ended_session_reaps_it_and_unregisters_it() {
        // "自己结束"这条路上没人等结果，所以它必须**自己**把三件事做完：
        // 收尸（不留僵尸）、摘牌（两张表一起）、撤销兜底登记（否则端到端退出时
        // 会对着一个复用掉的 pid 再发一次 SIGKILL）。
        let log = ControlLog(Arc::new(Mutex::new(Vec::new())));
        let trace = Arc::new(Mutex::new(Vec::new()));
        let (sessions, written) = watchdog_with(&log);

        let (handle, _batches) = sessions
            .register(
                Recording::new(&trace, "a", false).leader(2001),
                BatchPolicy::DEFAULT,
            )
            .expect("登记失败");
        assert_eq!(control_lines(&written), vec!["+2001"]);

        let retired = sessions
            .retire(handle)
            .expect("retire 不该失败")
            .expect("会话还在，应当被收掉");

        assert_eq!(retired.id.get(), 1);
        assert_eq!(retired.status, Some(ExitStatus::Code(0)));
        assert_eq!(shutdown_count(&trace), 1, "自己结束也要显式收尸");
        assert!(
            sessions.is_empty() && sessions.registered() == 0,
            "两张表必须一起摘干净"
        );
        assert_eq!(
            control_lines(&written),
            vec!["+2001", "-2001"],
            "收干净之后要撤销兜底登记"
        );
    }

    #[test]
    fn retiring_twice_is_a_no_op_not_an_error() {
        // 用户点 × 与 shell 自己退出可能几乎同时发生 —— 两条路都会走到 retire。
        let log = ControlLog(Arc::new(Mutex::new(Vec::new())));
        let trace = Arc::new(Mutex::new(Vec::new()));
        let (sessions, _written) = watchdog_with(&log);
        let (handle, _batches) = sessions
            .register(
                Recording::new(&trace, "a", false).leader(2002),
                BatchPolicy::DEFAULT,
            )
            .expect("登记失败");

        sessions
            .retire(handle)
            .expect("第一次 retire 失败")
            .expect("应当收掉");
        assert!(
            sessions
                .retire(handle)
                .expect("第二次 retire 不该报错")
                .is_none(),
            "已经收过的会话再收一次是空操作"
        );
        assert_eq!(shutdown_count(&trace), 1, "不得重复收尸");

        // 从没登记过的句柄同理：不是错误。
        assert!(sessions.retire(4242).expect("未知句柄不该报错").is_none());
    }

    #[test]
    fn a_failed_reap_still_unregisters_the_ended_session() {
        // **不半途而废**：会话已经结束了，收尸失败只该记日志 —— 留着一条
        // "查不到、其实已经死了"的登记比收尸失败本身糟得多。
        let log = ControlLog(Arc::new(Mutex::new(Vec::new())));
        let trace = Arc::new(Mutex::new(Vec::new()));
        let (sessions, written) = watchdog_with(&log);
        let (handle, _batches) = sessions
            .register(
                Recording::new(&trace, "bad", true).leader(2003),
                BatchPolicy::DEFAULT,
            )
            .expect("登记失败");

        let retired = sessions
            .retire(handle)
            .expect("收尸失败不该让 retire 报错")
            .expect("仍然要摘牌");

        assert_eq!(retired.status, None, "收尸失败时没有结局可报");
        assert!(sessions.is_empty() && sessions.registered() == 0);
        assert_eq!(
            control_lines(&written),
            vec!["+2003", "-2003"],
            "失败也要撤销登记：它已经不存在了"
        );
    }

    // ── plan 0301：托盘靠这条通知刷新菜单（隧道列表）─────────────────────────

    /// 登记一个假载体，返回句柄。
    fn register_recording(sessions: &Sessions, name: &'static str) -> SessionHandle {
        let log = Arc::new(Mutex::new(Vec::new()));
        sessions
            .register(Recording::new(&log, name, false), BatchPolicy::DEFAULT)
            .expect("登记失败")
            .0
    }

    #[test]
    fn sessions_changed_hook_fires_on_open_close_and_retire() {
        // ⚠️ 钩子里**回头读会话表** —— 托盘就是这么干的（列出隧道）。通知若排在会话表
        // 的锁里面，这里会自己等自己：于是"忘了 `drop(inner)` 再通知"这种错当场暴露。
        let sessions = Sessions::default();
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&fired);
        let table = sessions.clone();
        sessions.on_change(move || {
            assert!(
                table.tunnels().is_empty(),
                "会话表读到一半的状态必须是可读的"
            );
            counter.fetch_add(1, Ordering::SeqCst);
        });

        let handle = register_recording(&sessions, "a");
        assert_eq!(fired.load(Ordering::SeqCst), 1, "开一个会话没通知");

        sessions.close(handle).expect("关闭失败");
        assert_eq!(fired.load(Ordering::SeqCst), 2, "关一个会话没通知");

        // retire 是另一条路（用户敲 exit）—— 它同样会改变菜单该显示什么。
        let handle = register_recording(&sessions, "b");
        sessions.retire(handle).expect("retire 失败");
        assert_eq!(fired.load(Ordering::SeqCst), 4, "自己结束的会话也要通知");

        sessions.shutdown_all();
        assert_eq!(fired.load(Ordering::SeqCst), 5, "退出收尾也要通知");
    }

    #[test]
    fn a_second_change_hook_is_ignored() {
        // 两个订阅者会去改同一份 UI 状态（托盘菜单），那必须被看见而不是静默叠加。
        let sessions = Sessions::default();
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));
        let (a, b) = (Arc::clone(&first), Arc::clone(&second));
        sessions.on_change(move || {
            a.fetch_add(1, Ordering::SeqCst);
        });
        sessions.on_change(move || {
            b.fetch_add(1, Ordering::SeqCst);
        });

        let _ = register_recording(&sessions, "a");

        assert_eq!(first.load(Ordering::SeqCst), 1);
        assert_eq!(second.load(Ordering::SeqCst), 0, "第二个订阅者必须被忽略");
    }

    #[test]
    fn terminal_sessions_are_not_listed_as_tunnels() {
        // ⚠️ 这条只守一半：**能开出隧道类会话的入口要到阶段 6 才有**（0601），
        // 所以"隧道出现在列表里"那一半现在无法构造。它守的是"别把终端混进隧道列表"。
        let sessions = Sessions::default();
        let _ = register_recording(&sessions, "a");

        assert!(sessions.tunnels().is_empty());
        assert_eq!(sessions.registered(), 1, "终端会话本身仍然登记着");
    }
}
