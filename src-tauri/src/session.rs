//! **Session 模型**与它在 app 侧的编组。
//!
//! 三条硬约束来自 `docs/scope.md` §5.1，这里逐条落成类型：
//!
//! 1. **资源挂在 `Session` 上**，不挂在"当前选中项"或全局单例上 ——
//!    所以 [`SessionRegistry`] 是一个**集合**，没有"当前 Session"这种概念。
//! 2. **事件按 [`SessionId`] 路由**，不是全局广播后由前端过滤 ——
//!    所以 `emit` 只投给该 `SessionId` 的订阅者，并返回实际投递数。
//! 3. **Session 类型之间互不依赖** —— [`SessionKind`] 是平级枚举，
//!    没有"先有终端才谈得上连接"这种隐含顺序。
//!
//! 命名守 `docs/scope.md` §1.2：**后端类型名不得编码 UI 呈现方式**
//! （不出 `Tab` / `Pane` / `Window` / `View`），由
//! `scripts/ast-grep/rules/no-ui-vocab-in-types.yml` 强制。
//!
//! **范围**：只有数据结构与路由语义。不含任何后端实现、不含持久化、
//! 不引入 async runtime。[`ipc`] 是它在 app 侧的那一半（命令、频道、注册表实体）。
//!
//! 零 Tauri 依赖的只有 [`model`] / [`registry`] / [`event`] 三个子模块（`AGENTS.md` §3.1）。

pub mod event;
pub mod ipc;
pub mod model;
pub mod registry;

pub use event::SessionEvent;
pub use ipc::*;
pub use model::{SessionError, SessionId, SessionKind};
pub use registry::SessionRegistry;
