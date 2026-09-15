//! akasha-core —— **Session 模型**：谁是资源的归属单位、谁负责回收。
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
//! `.ast-grep/rules/no-ui-vocab-in-types.yml` 强制。
//!
//! **零 Tauri 依赖**（`AGENTS.md` §3.1），由
//! `.ast-grep/rules/no-tauri-in-core-crates.yml` 强制。
//!
//! **范围**：只有数据结构与路由语义。不含任何后端实现（PTY 在 plan 0105）、
//! 不含持久化（阶段 4）、不引入 async runtime（本步只立模型）。
//!
//! 配置模型（[`Config`]）也在这里：它同样只是**判据**（配置 × 环境 → 行为），
//! 不是实现 —— 读文件（以及将来要写的那一份）都属于 app 侧（plan 0303）。

mod config;
mod event;
mod registry;
mod session;
mod tunnel;

pub use config::{CloseAction, CloseBehavior, Config, Reconnect, Transfer};
pub use event::SessionEvent;
pub use registry::SessionRegistry;
pub use session::{SessionError, SessionId, SessionKind};
pub use tunnel::{TunnelState, TunnelTransitionError};
