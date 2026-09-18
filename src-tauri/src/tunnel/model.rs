//! **隧道状态机**（ADR-0003 **D12** / **D13**）。
//!
//! `docs/scope.md` §2.2 把端口转发的状态定成五态，并要求**状态变化发事件**、
//! **失败必须可见**、**提供手动重试**。这里的全部内容就是那三条里的前两条与第三条的
//! **语义**部分 —— 一个纯逻辑的状态机，零 Tauri、零 async、零网络。
//!
//! 为什么状态机单独成一份而不是写在隧道实体里：ADR-0003 D12 的原话是"状态机本身
//! **不依赖 russh**：它是纯逻辑"。放在这里之后，"哪些转移合法"可以用穷尽的正反例固定下来
//! （见文件末尾的用例），而**真实路径上产生不了的那条边**（`重连中`，要等 plan 0605 的
//! 重连循环）也照样被覆盖 —— 那正是把它与连接代码分开的价值。
//!
//! ## 与"连接"的分工
//!
//! 本模块**只回答"状态能不能变成另一个状态"**。谁去连接、连不上算哪一类失败、
//! 退避多久，都在别处：策略参数在配置模型里（D13），连接与实体在 app 侧（plan 0601）。

use std::fmt;

/// 一条隧道的状态（`docs/scope.md` §2.2 原文的五态）。
///
/// ⚠️ 状态名会进入 `src/ipc/bindings.ts` 与前端（ADR-0003 §10 第 4 条把这条列为
/// 不可逆点）—— 改名等于改一份可能已经有人依赖的契约。IPC 用的名字是 [`Self::as_str`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TunnelState {
    /// 正在连接（含首次连接与手动重试之后的连接）。
    Connecting,
    /// 已连接：这条隧道的连接活着。
    Connected,
    /// 重连中：正在等待**第 `attempt` 次**重试。`attempt` 从 1 起 ——
    /// "第 0 次重试"是自相矛盾的，那种情况用 [`TunnelState::Connecting`] 表示。
    Reconnecting {
        /// 第几次重试。**必须 ≥ 1**（由 [`TunnelState::advance`] 强制）。
        attempt: u32,
    },
    /// 失败：重连次数耗尽，或者这一类失败**本来就不重试**（D13 的判据表）。
    Failed,
    /// 已停止：用户（或退出路径）明确停止了它，或它已正常收尾。
    Stopped,
}

impl TunnelState {
    /// 全部状态（`重连中` 取 `attempt = 1` 作代表）。用于测试与界面遍历。
    pub const ALL: [TunnelState; 5] = [
        TunnelState::Connecting,
        TunnelState::Connected,
        TunnelState::Reconnecting { attempt: 1 },
        TunnelState::Failed,
        TunnelState::Stopped,
    ];

    /// 日志与 IPC 用的稳定短名。**不是** `Debug` 输出，也不含 `attempt`
    /// （`attempt` 是字段，按 `docs/logging.md` 的规则单独成字段）。
    pub const fn as_str(self) -> &'static str {
        match self {
            TunnelState::Connecting => "connecting",
            TunnelState::Connected => "connected",
            TunnelState::Reconnecting { .. } => "reconnecting",
            TunnelState::Failed => "failed",
            TunnelState::Stopped => "stopped",
        }
    }

    /// 重连尝试次数（只有 [`TunnelState::Reconnecting`] 有）。
    pub const fn attempt(self) -> Option<u32> {
        match self {
            TunnelState::Reconnecting { attempt } => Some(attempt),
            _ => None,
        }
    }

    /// 这个状态下这条隧道**已经不用再等了**：后端不再有活动，也没有重连任务。
    ///
    /// "失败必须可见"（`docs/scope.md` §2.2）靠的就是这两个状态在界面与托盘里出现 ——
    /// 所以它们是**终态**，不会自己再变回 [`TunnelState::Connecting`]（只有手动重试会）。
    pub const fn is_terminal(self) -> bool {
        matches!(self, TunnelState::Failed | TunnelState::Stopped)
    }

    /// `self → next` 是不是一条合法边。
    ///
    /// 与 [`Self::advance`] 的区别：这个只回答"能不能"，不改任何东西 —— 界面与托盘
    /// 只是想显示，不该为了判断而构造一次转移。
    pub const fn can_advance_to(self, next: TunnelState) -> bool {
        use TunnelState::{Connected, Connecting, Failed, Reconnecting, Stopped};
        match (self, next) {
            // 连接中：成了、败了、被停了，或者进入重连（第 1 次）。
            (Connecting, Connected | Failed | Stopped) => true,
            (Connecting, Reconnecting { attempt }) => attempt >= 1,
            // 已连接：断了要重连（D13 的传输层断开那一档）、或者败了 / 被停了。
            (Connected, Failed | Stopped) => true,
            (Connected, Reconnecting { attempt }) => attempt >= 1,
            // 重连中：开始下一次尝试、放弃、或被停。
            // ⚠️ `重连中 → 重连中` **非法**：那是在同一状态里改数字，
            // 而"第 n 次等待"与"第 n+1 次等待"之间必然隔着一次尝试（`连接中`）。
            (Reconnecting { .. }, Connecting | Failed | Stopped) => true,
            // 终态只能被手动重试拉回 `连接中`（D12），或者被停（幂等地变成"已停止"）。
            (Failed, Connecting | Stopped) => true,
            (Stopped, Connecting) => true,
            _ => false,
        }
    }

    /// 走一步。非法边是 [`TunnelTransitionError`]，**不是** panic：
    /// 调用方是长驻任务与命令边界（`AGENTS.md` §0）。
    pub const fn advance(self, next: TunnelState) -> Result<TunnelState, TunnelTransitionError> {
        if let TunnelState::Reconnecting { attempt } = next
            && attempt == 0
        {
            return Err(TunnelTransitionError::AttemptOutOfRange { attempt });
        }
        if self.can_advance_to(next) {
            Ok(next)
        } else {
            Err(TunnelTransitionError::Illegal {
                from: self,
                to: next,
            })
        }
    }

    /// 手动重试（D12：`失败 / 已停止 → 连接中`）。其余状态报非法。
    ///
    /// 它单独一个方法而不是让调用方自己写 `advance(Connecting)`：D12 把这三次转移
    /// 定成产品动作（"自动放弃的前提是用户能一键再试"），而 `advance` 那条路要同时覆盖
    /// `连接中 → …` 等**内部**转移。分开之后，界面与命令只需问一句"能不能重试"。
    pub const fn retry(self) -> Result<TunnelState, TunnelTransitionError> {
        match self {
            TunnelState::Failed | TunnelState::Stopped => Ok(TunnelState::Connecting),
            from => Err(TunnelTransitionError::Illegal {
                from,
                to: TunnelState::Connecting,
            }),
        }
    }
}

impl fmt::Display for TunnelState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一次非法转移。
///
/// 带着**两端**：只报"目标不合法"在排查时没有用（"从哪儿来的"正是要看的那个信息）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelTransitionError {
    /// 这条边不在白名单里。
    Illegal {
        /// 出发状态。
        from: TunnelState,
        /// 被拒绝的目标状态。
        to: TunnelState,
    },
    /// `重连中` 的次数非法（必须是 ≥ 1 的数）。
    AttemptOutOfRange {
        /// 被拒绝的次数。
        attempt: u32,
    },
}

impl fmt::Display for TunnelTransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TunnelTransitionError::Illegal { from, to } => {
                write!(f, "非法的隧道状态转移：{from} → {to}")
            }
            TunnelTransitionError::AttemptOutOfRange { attempt } => {
                write!(f, "重连次数必须 ≥ 1（收到 {attempt}）")
            }
        }
    }
}

impl std::error::Error for TunnelTransitionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use TunnelState::{Connected, Connecting, Failed, Reconnecting, Stopped};

    /// 应当合法的边。**逐条写出来**，不引用实现里的那个 `match` ——
    /// 把白名单抄进用例等于没测（`AGENTS.md` §6 记过同一类问题：判据太弱时门禁照样转绿）。
    const LEGAL: &[(TunnelState, TunnelState)] = &[
        (Connecting, Connected),
        (Connecting, Failed),
        (Connecting, Stopped),
        (Connecting, Reconnecting { attempt: 1 }),
        (Connecting, Reconnecting { attempt: 3 }),
        (Connected, Failed),
        (Connected, Stopped),
        (Connected, Reconnecting { attempt: 1 }),
        (Reconnecting { attempt: 1 }, Connecting),
        (Reconnecting { attempt: 2 }, Failed),
        (Reconnecting { attempt: 1 }, Stopped),
        (Failed, Connecting),
        (Failed, Stopped),
        (Stopped, Connecting),
    ];

    /// 应当非法的边。同态那几条由 [`TunnelState::ALL`] 生成，其余手写。
    const ILLEGAL: &[(TunnelState, TunnelState)] = &[
        // "已连接"不能悄悄变回"连接中"：那会把一次已建立的连接从状态里抹掉。
        (Connected, Connecting),
        // 终态不能跳过"连接中"直接回到"已连接"。
        (Failed, Connected),
        (Stopped, Connected),
        // "已停止"不是"失败"的前一态。
        (Stopped, Failed),
        // 重连成功也必须经过"连接中"。
        (Reconnecting { attempt: 1 }, Connected),
        // "重连中(n)"与"重连中(m)"之间必然隔着一次尝试。
        (Reconnecting { attempt: 1 }, Reconnecting { attempt: 2 }),
        (Reconnecting { attempt: 2 }, Reconnecting { attempt: 2 }),
    ];

    #[test]
    fn every_state_has_a_distinct_short_name() {
        let names: Vec<&str> = TunnelState::ALL
            .iter()
            .map(|state| state.as_str())
            .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "短名必须互不相同：{names:?}");
        assert_eq!(names.len(), 5, "五态一个不少");
    }

    #[test]
    fn display_matches_the_short_name() {
        for state in TunnelState::ALL {
            assert_eq!(state.to_string(), state.as_str());
        }
    }

    #[test]
    fn the_legal_edges_are_exactly_what_the_plan_lists() {
        for (from, to) in LEGAL {
            assert!(from.can_advance_to(*to), "这条边应当合法：{from} → {to}");
            assert_eq!(from.advance(*to), Ok(*to), "{from} → {to} 应当走通");
        }
    }

    #[test]
    fn the_illegal_edges_are_rejected() {
        for (from, to) in ILLEGAL {
            assert!(!from.can_advance_to(*to), "这条边应当被拒：{from} → {to}");
            assert!(
                matches!(
                    from.advance(*to),
                    Err(TunnelTransitionError::Illegal { .. })
                ),
                "{from} → {to} 必须报 Illegal，而不是别的错误或 Ok"
            );
        }
    }

    #[test]
    fn no_state_may_move_to_itself() {
        for state in TunnelState::ALL {
            assert!(
                !state.can_advance_to(state),
                "同态转移必须被拒（否则「状态没变」与「变了一次」在事件里无法区分）：{state}"
            );
        }
    }

    #[test]
    fn reconnecting_attempt_must_be_at_least_one() {
        for from in TunnelState::ALL {
            assert_eq!(
                from.advance(Reconnecting { attempt: 0 }),
                Err(TunnelTransitionError::AttemptOutOfRange { attempt: 0 }),
                "第 0 次重试是自相矛盾的（那种情况是「连接中」）：从 {from} 转移时也必须被拒"
            );
        }
    }

    #[test]
    fn retry_is_only_available_from_the_two_terminal_states() {
        assert_eq!(Failed.retry(), Ok(Connecting));
        assert_eq!(Stopped.retry(), Ok(Connecting));
        let busy = [Connecting, Connected, Reconnecting { attempt: 2 }];
        for from in busy {
            assert!(
                matches!(from.retry(), Err(TunnelTransitionError::Illegal { .. })),
                "{from} 不该能手动重试（它本来就在忙）"
            );
        }
    }

    #[test]
    fn terminal_states_are_failed_and_stopped() {
        assert!(Failed.is_terminal());
        assert!(Stopped.is_terminal());
        assert!(!Connecting.is_terminal());
        assert!(!Connected.is_terminal());
        let waiting = Reconnecting { attempt: 1 };
        assert!(!waiting.is_terminal(), "重连中不是终态：它还会再动");
    }

    #[test]
    fn attempt_is_carried_by_reconnecting_only() {
        let waiting = Reconnecting { attempt: 2 };
        assert_eq!(waiting.attempt(), Some(2));
        for state in [Connecting, Connected, Failed, Stopped] {
            assert_eq!(state.attempt(), None, "{state} 不该带尝试次数");
        }
    }

    /// "五态可观测"的**模型**证据：从 `连接中` 出发，五态都走得到。
    ///
    /// 真实路径上 `重连中` 要等 plan 0605 的重连循环（那时才有人去驱动那条边），
    /// 所以它的可达性只能在这里证明。
    #[test]
    fn every_state_is_reachable_from_connecting() {
        let mut reached = vec![Connecting];
        loop {
            let before = reached.len();
            for from in reached.clone() {
                for to in TunnelState::ALL {
                    if from.can_advance_to(to) && !reached.contains(&to) {
                        reached.push(to);
                    }
                }
            }
            if reached.len() == before {
                break;
            }
        }
        for state in TunnelState::ALL {
            // `ALL` 里的 `重连中` 是 `attempt = 1`；`advance` 允许的也正是 ≥ 1 的取值。
            assert!(
                reached.contains(&state),
                "{state} 从「连接中」到不了 —— 五态里有走不到的格子"
            );
        }
    }

    /// 非法转移**带着两端**（只报"目标不合法"在排查时没有用）。
    #[test]
    fn an_illegal_transition_names_both_ends() {
        let err = Connected.advance(Connecting).expect_err("应当被拒");
        assert_eq!(
            err,
            TunnelTransitionError::Illegal {
                from: Connected,
                to: Connecting
            }
        );
        assert!(
            err.to_string().contains("connected"),
            "错误消息要能读：{err}"
        );
        assert!(
            err.to_string().contains("connecting"),
            "错误消息要能读：{err}"
        );
    }
}
