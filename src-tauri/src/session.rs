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
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;

use akasha_core::{SessionId, SessionKind, SessionRegistry};
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
    transport: PtyTransport,
}

#[derive(Default)]
struct Inner {
    registry: SessionRegistry,
    live: HashMap<SessionHandle, Live>,
}

/// 全部终端会话。由 tauri 作为 `State` 持有（`Send + Sync`）。
#[derive(Default)]
pub struct Sessions {
    inner: Mutex<Inner>,
}

impl Sessions {
    fn lock(&self) -> Result<MutexGuard<'_, Inner>, IpcError> {
        self.inner.lock().map_err(|err| IpcError::Internal {
            message: err.to_string(),
        })
    }

    /// 登记一个会话，并把它的**批次流**交出来。
    ///
    /// 刻意不接 IPC：调用方决定 sink（命令接频道，测试接 `Vec`）。这样"会话管理"这段
    /// 逻辑可以脱离 app 测（`AGENTS.md` §3.1 的分层理由）。
    ///
    /// 返回的句柄是**过 IPC 的表示**（[`SessionHandle`]）：`SessionId` 不许在
    /// `akasha-core` 之外构造（那是它的设计意图），所以壳层只用它的 checked 投影，
    /// 真伪由注册表判定 —— 不存在的句柄一律 [`IpcError::NotFound`]。
    pub fn register(
        &self,
        mut transport: PtyTransport,
        policy: BatchPolicy,
    ) -> Result<(SessionHandle, Receiver<Batch>), IpcError> {
        // 先取走读端：载体允许读端只被取走一次（防两个读端互相偷字节）。
        let output = transport
            .output_stream()
            .ok_or_else(|| IpcError::Transport {
                message: "读端已被取走".into(),
            })?;

        let mut inner = self.lock()?;
        let id = inner
            .registry
            .open(SessionKind::Terminal)
            .map_err(|err| IpcError::Internal {
                message: err.to_string(),
            })?;
        let handle = Self::handle(id)?;
        inner.live.insert(handle, Live { id, transport });
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
        Ok(())
    }

    /// 当前活着的会话数（测试与将来的诊断用）。
    pub fn len(&self) -> usize {
        self.lock().map(|inner| inner.live.len()).unwrap_or(0)
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
}
