//! 隧道：**五态状态机**（[`model`]）与它在 app 侧的编组（[`ipc`]）。
//!
//! [`model`] 只回答"哪些转移合法"，不看平台、不碰 Tauri；[`ipc`] 持有实体表与重试任务。
//! 实体表与 [`crate::session::Sessions`] 是**同一张注册表**（ADR-0003 D6），不另立一份。

pub mod ipc;
pub mod model;

pub use ipc::*;
pub use model::{TunnelState, TunnelTransitionError};
