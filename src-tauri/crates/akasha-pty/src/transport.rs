//! `Transport` 契约与它的词表：尺寸、能力、结局、错误。

use std::fmt;
use std::io::Read;

/// 终端窗口尺寸。
///
/// 只有 `cols` / `rows`：像素尺寸在 PTY 上**几乎总是被内核忽略**，
/// 留两个永远写 0 的字段只会让调用方以为它们有用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    /// 列数。
    pub cols: u16,
    /// 行数。
    pub rows: u16,
}

impl TerminalSize {
    /// 常用默认值（80×24）。**不用 `derive(Default)`**：那会得到 0×0，
    /// 而 0×0 是一个所有终端都会算错的尺寸。
    pub const DEFAULT: Self = Self { cols: 80, rows: 24 };

    /// 构造尺寸。
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 载体**声明**自己能做什么。
///
/// 能力位存在的意义：让"支持什么"是**数据**而不是"你有没有实现那个方法"。
/// 于是 serial / SSH 不必为了实现 trait 而写一堆 `unimplemented!()`。
///
/// 这里只放 [`Transport`] **当前真能表达**的能力。刻意**没有** `signals`：
/// trait 里没有任何方法能发信号，一个没有对应方法的能力位是空头支票 ——
/// 等有了 `signal()` 再加它。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// 能否 `resize`。serial 没有窗口尺寸；SSH 也没有本地窗口语义。
    pub resize: bool,
    /// 能否给出退出结局。serial 与 SSH 没有本地进程语义。
    pub exit_status: bool,
}

impl Capabilities {
    /// 本地 PTY：三样都有。
    pub const PTY: Self = Self {
        resize: true,
        exit_status: true,
    };

    /// 什么都不声明（最小实现的样子）。
    pub const NONE: Self = Self {
        resize: false,
        exit_status: false,
    };
}

/// 载体的结局。
///
/// 与 `portable-pty` 的 `ExitStatus` 分开定义：那是**实现**的类型，
/// 不该出现在跨后端（将来还有 SSH / serial）的公共契约里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitStatus {
    /// 退出码。非零不代表"错了" —— 那取决于被运行的程序。
    Code(u32),
    /// 被信号终止（POSIX）。Windows 上不会出现这一支。
    Signal(String),
}

impl ExitStatus {
    /// 是否正常结束（退出码 0，且不是被信号杀掉的）。
    pub fn success(&self) -> bool {
        matches!(self, ExitStatus::Code(0))
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExitStatus::Code(code) => write!(f, "退出码 {code}"),
            ExitStatus::Signal(name) => write!(f, "被信号 {name} 终止"),
        }
    }
}

/// 载体操作的失败原因。
///
/// 分域定义（`AGENTS.md` §3.4）：调用方要能区分"这个载体不支持"（能力问题，不该重试）
/// 与"IO 出错"（环境问题，可能可重试）—— 两者的处置完全不同。
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// 载体不具备该能力。**不是异常，是契约的一部分**：调用方应先看
    /// [`Transport::capabilities`]。
    #[error("载体不支持该能力：{0}")]
    Unsupported(&'static str),
    /// 载体已关闭（或写端已取走）。继续写没有意义。
    #[error("载体已关闭")]
    Closed,
    /// 底层 IO 失败。
    #[error("IO 失败：{0}")]
    Io(#[from] std::io::Error),
}

/// **字节载体**：把字节写进去、把字节读出来，外加尺寸与收尾。
///
/// 为什么是这五个方法（`docs/scope.md` §2）：它们恰好覆盖三个后端共同的部分，
/// 而差异（窗口尺寸 / 退出码 / 信号 / 本地进程语义）全部由 [`Capabilities`] 表达。
///
/// 实现者注意：**不要**用 `Drop` 代替 [`Transport::shutdown`]（`AGENTS.md` §3.3）。
pub trait Transport: Send {
    /// 声明能力（不是猜测）。默认实现给出"最小载体"的答案。
    fn capabilities(&self) -> Capabilities {
        Capabilities::NONE
    }

    /// 把字节送进载体。
    ///
    /// **只收 `&[u8]`**：终端输入不是 UTF-8 字符串，一次按键可能是多字节序列的一部分，
    /// 也可能只是半个序列（`AGENTS.md` §3.2）。
    fn write(&mut self, bytes: &[u8]) -> Result<(), TransportError>;

    /// 取走输出流。
    ///
    /// **只能成功取一次**，之后返回 `None`。这不是省事，而是防错：
    /// 同一个载体的两个读端会**互相偷字节**，于是输出随机丢一段 ——
    /// 那种 bug 不报错、不可复现，只在读端数量上出错。API 直接让它写不出来。
    ///
    /// 取走后怎么读（线程、合批、背压）是调用方的事 —— 本 crate 不**偷偷**起线程，
    /// 合批要用就显式调 [`crate::spawn_batcher`]。
    fn output_stream(&mut self) -> Option<Box<dyn Read + Send>>;

    /// 调整窗口尺寸。**默认返回 [`TransportError::Unsupported`]** —— 具备该能力的载体覆写它
    /// （`capabilities().resize` 为 `true` 时**必须**覆写，否则就是自相矛盾）。
    fn resize(&mut self, size: TerminalSize) -> Result<(), TransportError> {
        let _ = size;
        Err(TransportError::Unsupported("resize"))
    }

    /// **非阻塞**查询是否已结束。`Ok(None)` = 还在跑，或该载体不报告结局。
    ///
    /// 默认实现返回 `Ok(None)`：serial 与 SSH 没有本地进程语义，不必为此写一个空方法。
    fn exited(&mut self) -> Result<Option<ExitStatus>, TransportError> {
        Ok(None)
    }

    /// 本载体背后**本地进程**的会话首进程 pid（没有就是 `None`）。
    ///
    /// 它存在的理由只有一条：`tauri dev` 的重编译重启是 SIGKILL，进程里没有任何代码
    /// 会执行 —— 那种时候只能靠 [`crate::watchdog`] 拿这个 pid 去收掉**整个会话**
    /// （`AGENTS.md` §3.3 的"真正退出"那一格，见 plan 0205）。
    ///
    /// 默认 `None`：内存载体与将来的纯网络后端没有本地进程可收。
    fn session_leader(&self) -> Option<u32> {
        None
    }

    /// 收尾：**显式结束并收尸**，返回结局（不具备 `exit_status` 能力的载体返回 `Ok(None)`）。
    ///
    /// `Ok(None)` **不是在说"成功"**，而是在说"这个载体没有结局可报"。
    /// 实现必须保证幂等：第二次调用返回同一个结局，而不是报错。
    ///
    /// ⚠️ 这是回收子进程的**唯一**正当入口。不调用它，子进程就会留下来 ——
    /// 这是刻意的（`AGENTS.md` §3.3：`drop` 不能代替 kill + wait）。
    fn shutdown(&mut self) -> Result<Option<ExitStatus>, TransportError>;
}
