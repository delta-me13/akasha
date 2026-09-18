//! akasha-pty —— **`Transport`：通用字节载体**。
//!
//! 这里落成的是**通用**抽象，不是 PTY 专属 trait：本地 PTY 只是它的第一个实现，
//! 之后 SSH shell 通道与串口要装进同一个 trait（`docs/scope.md` §2）。
//! 若做成 `PtySession` 那种形态，第二个后端到来时就得重构 —— 那正是本工作项存在的理由。
//!
//! 三条设计约束（都来自规范，不是偏好）：
//!
//! 1. **能力差异用 capability flag 表达**，不是"多几个方法都得实现一遍"
//!    （`docs/scope.md` §2）。所以 [`Transport`] 里 `resize` / `exited` 都有**默认实现**：
//!    不具备该能力的载体不必写它们，写了才有意义。
//! 2. **绝不当 `String` 传字节**（`AGENTS.md` §3.2）：trait 的方法签名只出现 `&[u8]`
//!    与 [`std::io::Read`]，不出现 `String` / `&str`。
//! 3. **shutdown 必须显式 kill + wait 收尸**，`drop` 不能代替（`AGENTS.md` §3.3）。
//!    所以本 crate **没有** `Drop` 实现 —— 那会让"忘记收尸"变成静默成功。
//!    对本地 PTY 还多一层：只 kill 那个 shell 是不够的，会话里**忽略 SIGHUP** 的进程
//!    （`nohup` / `trap "" HUP` / 守护化的）会活下来 —— [`PtyTransport::shutdown`]
//!    会先把**整个会话**收掉再收尸，理由与平台差异见 `teardown` 模块（plan 0204）。
//!
//! 还有**一条路径谁都没机会跑代码**：`tauri dev` 的重编译重启是 SIGKILL（plan 0205）。
//! 那一条靠 [`watchdog`]：另起一个进程读一条管道，app 一死就由它把登记过的会话全部收掉。
//!
//! 本 crate **不含**：IPC（plan 0202）、前端（阶段 2）、`Session` 模型（已在 `akasha-core`）。
//! 它只管"字节怎么进出载体"，外加把输出合批成 [`Batch`]（[`spawn_batcher`]）。
//!
//! ```no_run
//! use crate::pty::{PtyTransport, TerminalSize, Transport};
//!
//! let mut transport = PtyTransport::spawn_default(TerminalSize::DEFAULT)?;
//! let _output = transport.output_stream();
//! transport.write(b"echo hi\n")?;
//! transport.shutdown()?; // 显式收尸；不调用它就会留下子进程
//! # Ok::<(), crate::pty::TransportError>(())
//! ```

mod batcher;
mod local;
mod shell;
mod teardown;
pub mod testing;
mod transport;
pub mod watchdog;

pub use batcher::{Batch, BatchPolicy, OutputBatcher, Trigger, spawn_batcher};
pub use local::PtyTransport;
pub use shell::ShellLaunch;
pub use transport::{Capabilities, ExitStatus, TerminalSize, Transport, TransportError};
