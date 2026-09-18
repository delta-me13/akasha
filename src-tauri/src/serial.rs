//! akasha-serial —— **`Transport` 的串口实现**（`docs/scope.md` §2 的第三个后端）。
//!
//! 它做四件事：枚举本机的串口、按参数打开一个串口、把字节送进去与取出来、显式收尾。
//! 会话模型（`akasha-core`）、IPC 与界面都不在这里 —— 那些是 app 的事，
//! 与 `akasha-pty` / `akasha-ssh` 同一条边界。
//!
//! 四条来自规范的形状：
//!
//! 1. **能力差异用 capability flag 表达**（`scope.md` §2）：串口**没有**窗口尺寸、
//!    **没有**退出结局、本地**没有**进程。所以能力位是 `Capabilities::NONE`，
//!    `resize` 走 trait 的默认实现（答 `Unsupported`），`session_leader()` 是 `None`。
//! 2. **字节永远是 `&[u8]`**（`AGENTS.md` §3.2）：本 crate 的公共 API 里没有以
//!    `String` 承载数据的参数，也不假设输入是 UTF-8。
//! 3. **收尾必须显式**（`AGENTS.md` §3.3）：本 crate **没有** `Drop` 实现。串口没有子进程
//!    要回收，但读端会一直持有一个句柄 —— 不调 `shutdown()` 就没人告诉它结束。
//! 4. **枚举只是一个列表**：`ports()` 列出本机串口，空表是正常结果；
//!    ⚠️ 列出来的端口**不保证能打开**（见 `enumerate` 的模块文档）。
//!
//! ⚠️ **手动指定路径是第一类输入**：[`SerialSettings::new`] 收的就是一个设备路径
//! （`/dev/ttyUSB0` / `COM3`）。打开这条路**不经过枚举**，所以枚举不可用
//! （发行版缺 libudev）时它照常可用。
//!
//! ```no_run
//! use crate::pty::Transport;
//! use akasha_lib::serial::{SerialSettings, SerialTransport};
//!
//! let mut transport = SerialTransport::open(&SerialSettings::new("/dev/ttyUSB0", 115200))?;
//! let _output = transport.output_stream();
//! transport.write(b"?\r\n")?;
//! transport.shutdown()?; // 显式收尾；不调用它，读端就一直在等
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod enumerate;
pub mod ipc;
mod error;
mod settings;
mod transport;

pub use enumerate::{PortInfo, PortKind, ports};
pub use error::SerialError;
pub use settings::{DataBits, Flow, Parity, SerialSettings, StopBits};
pub use transport::SerialTransport;
pub use ipc::*;
