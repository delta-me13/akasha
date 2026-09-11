use crate::session::SessionId;

/// 发给某个 `Session` 的事件。
///
/// **每个事件自带它的 `SessionId`**，这就是"按 `SessionId` 路由"的物理基础：
/// 投递方不需要额外的"这是发给谁的"参数，路由因此不可能与事件内容不一致
/// （`docs/scope.md` §5.1 第 2 条 —— 全局广播后由前端过滤，多 Session 并行时会串）。
///
/// 目前只有生命周期事件：数据事件要等 `Transport` 落地（plan 0105 / 0201）。
/// 这里**不预先编造** `Output` / `Exited` 之类的形状 —— 那是下一个工作项的事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// 该 `Session` 已被关闭。这是订阅者能收到的**最后一条**事件，之后通道断开。
    Closed { id: SessionId },
}

impl SessionEvent {
    /// 这个事件属于哪个 `Session` —— 路由只看它。
    pub const fn session(&self) -> SessionId {
        match self {
            SessionEvent::Closed { id } => *id,
        }
    }
}
