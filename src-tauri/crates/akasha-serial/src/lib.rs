//! akasha-serial —— **`Transport` 的串口实现**（`docs/scope.md` §2 的第三个后端）。
//!
//! 它只做三件事：按参数打开一个串口、把字节送进去与取出来、显式收尾。
//! 会话模型（`akasha-core`）、IPC 与界面都不在这里 —— 那些是 app 的事，
//! 与 `akasha-pty` / `akasha-ssh` 同一条边界。
//!
//! 三条来自规范的形状：
//!
//! 1. **能力差异用 capability flag 表达**（`scope.md` §2）：串口**没有**窗口尺寸、
//!    **没有**退出结局、本地**没有**进程。所以能力位是 `Capabilities::NONE`，
//!    `resize` 走 trait 的默认实现（答 `Unsupported`），`session_leader()` 是 `None`。
//! 2. **字节永远是 `&[u8]`**（`AGENTS.md` §3.2）：本 crate 的公共 API 里没有以
//!    `String` 承载数据的参数，也不假设输入是 UTF-8。
//! 3. **收尾必须显式**（`AGENTS.md` §3.3）：本 crate **没有** `Drop` 实现。串口没有子进程
//!    要回收，但读端会一直持有一个句柄 —— 不调 `shutdown()` 就没人告诉它结束。
//!
//! ⚠️ **手动指定路径是第一类输入**：[`SerialSettings::new`] 收的就是一个设备路径
//! （`/dev/ttyUSB0` / `COM3`）。端口枚举、以及枚举不可用时的降级，是 plan 0802 的事 ——
//! 打开这条路不依赖它。
//!
//! ```no_run
//! use akasha_pty::Transport;
//! use akasha_serial::{SerialSettings, SerialTransport};
//!
//! let mut transport = SerialTransport::open(&SerialSettings::new("/dev/ttyUSB0", 115200))?;
//! let _output = transport.output_stream();
//! transport.write(b"?\\r\\n")?;
//! transport.shutdown()?; // 显式收尾；不调用它，读端就一直在等
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod settings;
mod transport;

pub use settings::{DataBits, Flow, Parity, SerialSettings, StopBits};
pub use transport::{SerialError, SerialTransport};
