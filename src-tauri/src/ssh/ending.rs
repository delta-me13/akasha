//! **一次转发是怎么结束的**（plan 0605）：[`ForwardEnd`] 与它的通知端 [`ForwardEnding`]。
//!
//! 重连循环要知道两件事：那次转发**没了**（否则它只能永远停在"已连接"），
//! 以及**是不是我们让它没的**。后者把它与"用户点了停止""重试换了一条"这类正常的收尾
//! 分开 —— 分不开的话，一条已经停掉的隧道会被它自己的重连循环重新拉起来。
//!
//! 通知端与转发本体**分开持有**：本体归隧道实体（停止与 probe 要它），通知端归重连循环
//! （它只关心"结束了"）。两者都只被 drop 一次，所以没有谁需要记得配对；
//! 谁拿到哪一半，在创建它的那一行就看得见（`serve` / `open` 的返回值）。

use tokio::sync::oneshot;

/// 这次转发为什么结束了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardEnd {
    /// 被要求停止（调用 `shutdown`，或者转发本体被 drop）。
    Stopped,
    /// 那条 SSH 连接没了：对端断开、保活耗尽、网络中断。
    ConnectionLost,
}

impl ForwardEnd {
    /// 日志字段用的稳定短名（`docs/logging.md`：一种事件一个 grep 模式）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::ConnectionLost => "connection_lost",
        }
    }
}

/// 一次转发结束时的通知端。
#[derive(Debug)]
pub struct ForwardEnding(oneshot::Receiver<ForwardEnd>);

impl ForwardEnding {
    /// 由转发任务那一侧创建（只在本 crate 内 —— 它是收尾路径的一部分）。
    pub(crate) fn new(receiver: oneshot::Receiver<ForwardEnd>) -> Self {
        Self(receiver)
    }

    /// 等它结束，返回原因。
    ///
    /// 发送端**没给值**就没了（只有转发任务被 abort 才会这样；本 crate 的每条收尾路径
    /// 都会先发一次）时按 [`ForwardEnd::Stopped`] 处理：在一条没人解释的结束上自作主张地
    /// 再连一遍，比少重连一次更坏 —— 后者只是用户点一下重试，前者会把一条已经停掉的隧道
    /// 重新拉起来。
    pub async fn ended(self) -> ForwardEnd {
        self.0.await.unwrap_or(ForwardEnd::Stopped)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn the_two_reasons_have_distinct_short_names() {
        assert_eq!(ForwardEnd::Stopped.as_str(), "stopped");
        assert_eq!(ForwardEnd::ConnectionLost.as_str(), "connection_lost");
        assert_ne!(
            ForwardEnd::Stopped.as_str(),
            ForwardEnd::ConnectionLost.as_str()
        );
    }

    #[tokio::test]
    async fn the_reason_crosses_the_channel() {
        let (sender, receiver) = oneshot::channel();
        sender.send(ForwardEnd::ConnectionLost).unwrap();
        assert_eq!(
            ForwardEnding::new(receiver).ended().await,
            ForwardEnd::ConnectionLost
        );
    }

    /// 发送端直接没了（任务被 abort）：按"停止"处理 —— 见 [`ForwardEnding::ended`]。
    #[tokio::test]
    async fn a_dropped_sender_reads_as_stopped() {
        let (sender, receiver) = oneshot::channel::<ForwardEnd>();
        drop(sender);
        assert_eq!(
            ForwardEnding::new(receiver).ended().await,
            ForwardEnd::Stopped
        );
    }
}
