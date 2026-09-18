//! 配置：**判据**（配置 × 环境 → 行为）与它的**文件载体**。
//!
//! [`model`] 是判据表本身（[`CloseAction::decide`]），脱离 app 可单测；
//! [`ipc`] 是 app 侧的读写与数据目录（`AGENTS.md` §3.1）—— 读文件（以及将来要写的那一份）
//! 属于 app 侧（plan 0303）。

pub mod ipc;
pub mod model;

pub use ipc::*;
pub use model::{CloseAction, CloseBehavior, Config, Reconnect, Transfer};
