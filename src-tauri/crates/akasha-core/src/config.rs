//! 配置模型（plan 0303 / 0605）：**类型、默认值、判据表**；文件读写不在这里（那是 app 的事）。
//!
//! 为什么放 core：`AGENTS.md` §3.1 —— 纯逻辑下沉到不依赖 Tauri 的 crate，脱离 app 可单测。
//! 本模块真正的价值是 [`CloseAction::decide`] 那张表：它把
//! "配置想要什么" 与 "这台机器上实际能做到什么" 分开，于是两条容易写错的规则
//! （`AGENTS.md` §3.3）可以用普通单测钉住。隧道那条路上同类的东西是 [`Reconnect::delay`]：
//! 它把"还有没有下一次"与"下次等多久"合成一处判据（ADR-0003 D13）。
//!
//! ⚠️ 本 crate 的 `[dependencies]` 为空**是有意为之**（见 `Cargo.toml`），所以这里
//! **不引入任何解析器**：**值**的解析（`"tray"` / `"exit"`）在这里，**文件**的解析在 app 侧。

use std::time::Duration;

/// 关闭窗口时用户**想要**什么。
///
/// 默认 [`CloseBehavior::Tray`]：一个终端被误点叉不该把正在跑的作业全带走。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloseBehavior {
    /// 收进托盘：窗口**隐藏而不销毁**，会话与终端缓冲原样存活（plan 0302）。
    #[default]
    Tray,
    /// 直接退出：走 plan 0204 的收尾路径（`shutdown_all` → `RunEvent::Exit`），零残留。
    Exit,
}

impl CloseBehavior {
    /// 全部取值，供"配置项全集"的测试与将来的配置 UI 遍历。
    pub const ALL: [CloseBehavior; 2] = [CloseBehavior::Tray, CloseBehavior::Exit];

    /// 配置值 → 行为。**不认识的值返回 `None`**，由调用方落回默认值并记日志 ——
    /// 策略（默认值是什么、日志记在哪）不属于这里。
    ///
    /// 严格小写、不带别名：多一种写法就多一条要维护的契约，而配置文件是人手写的，
    /// 写错了要**看见**（调用方那条 warn），不是被悄悄猜中。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tray" => Some(Self::Tray),
            "exit" => Some(Self::Exit),
            _ => None,
        }
    }

    /// 稳定短名。**这是配置文件里的取值**，也是日志字段的取值 —— 两处必须是同一个字符串，
    /// 否则"日志里写 tray、文件里要写 Tray"这种分叉只会浪费下一个人的时间。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tray => "tray",
            Self::Exit => "exit",
        }
    }
}

/// 关窗请求**实际**会怎么做。
///
/// 与 [`CloseBehavior`] 分开是刻意的：用户的意愿与这台机器能做到的事**不是一回事**，
/// 把它们合成一个枚举，就等于默认"想要的 = 做得到的"，而那正是问题 #60 的形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// 拦下关闭、把窗口藏起来（会话与终端缓冲都还在）。
    Hide,
    /// 放行关闭：走正常退出路径。
    Exit,
}

impl CloseAction {
    /// 判据表 —— **本模块唯一的真逻辑**，两条规则都在 `AGENTS.md` §3.3：
    ///
    /// | 配置 | 托盘可用 | 动作 | 为什么 |
    /// |---|---|---|---|
    /// | `exit` | 任意 | 退出 | 用户明确要求；**这时不看托盘** —— "直接退出"就是直接退出 |
    /// | `tray` | 是 | 隐藏 | 常规路径：托盘能把窗口叫回来 |
    /// | `tray` | **否** | **退出** | 隐藏之后就**再也叫不回窗口**了 —— 用户看不见进程、也关不掉它（问题 #60） |
    pub const fn decide(behavior: CloseBehavior, tray_ready: bool) -> Self {
        match (behavior, tray_ready) {
            (CloseBehavior::Exit, _) => Self::Exit,
            (CloseBehavior::Tray, true) => Self::Hide,
            (CloseBehavior::Tray, false) => Self::Exit,
        }
    }

    /// 日志/诊断用的稳定短名（与 [`CloseBehavior::as_str`] 同样的理由）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hide => "hide",
            Self::Exit => "exit",
        }
    }
}

/// 掉线重连的参数（ADR-0003 **D13**）：**有限次** + 指数退避，然后放弃。
///
/// 为什么不是"无限重连"：`docs/scope.md` §2.2 已否决 —— 后台无谓的重试会持续扰动
/// 防火墙，也让对端日志堆满同一条失败。D13 另给了一条更具体的理由：以错误的口令
/// 连续尝试三次正是账号锁定的经典成因，所以"不重试"的类别必须与"重试"的分开
/// （判据在 `crate::TunnelState` 的使用方 `akasha` 侧，因为那一档要读错误）。
///
/// 取值放在这里而不是散在调用处：D13 的原话是"默认值写在配置模型里"。
/// 它同时也是"重连预算是多少"这一条判据的**唯一**来源 —— 序列算在 [`Self::delay`]，
/// 调用方只按次数问，不自己乘。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reconnect {
    /// 最多重试几次。`0` = 不自动重连（掉线直接进 `失败`）。
    pub max_attempts: u32,
    /// **第 1 次**重试之前等多久。
    pub initial: Duration,
    /// 每往后一次，等待乘几。
    pub factor: u32,
}

impl Default for Reconnect {
    /// D13 定死的三个数：**3 次**，`1s → 2s → 4s`。
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial: Duration::from_secs(1),
            factor: 2,
        }
    }
}

impl Reconnect {
    /// **第 `attempt` 次**重试之前要等多久（`attempt` 从 1 起，与
    /// [`crate::TunnelState::Reconnecting`] 带的那个数是同一个数）。
    ///
    /// `None` = 这次重试**没有预算**（`attempt = 0`，或者次数已经用完）——
    /// "还有没有下一次"因此只有一个判据，不必让调用方自己比大小。
    ///
    /// 乘法用 `saturating_mul`：配置里的 `factor` 是用户给的数，一个荒唐的取值
    /// 不该让重连循环 panic（`AGENTS.md` §0 的禁止 #4 管的是长驻任务）。
    pub fn delay(&self, attempt: u32) -> Option<Duration> {
        if attempt == 0 || attempt > self.max_attempts {
            return None;
        }
        let mut delay = self.initial;
        for _ in 1..attempt {
            delay = delay.saturating_mul(self.factor);
        }
        Some(delay)
    }

    /// 用完全部次数一共要等多久（1s + 2s + 4s = 7s）。
    ///
    /// 它是**判据**的一部分（"耗尽次数"这件事在时间上意味着多久），
    /// 所以由这里算出来给测试与文档用，而不是让每处各加一遍。
    pub fn budget(&self) -> Duration {
        (1..=self.max_attempts)
            .filter_map(|attempt| self.delay(attempt))
            .fold(Duration::ZERO, |total, delay| total.saturating_add(delay))
    }
}

/// 传输引擎的参数（ADR-0006 **D6**）：**同时有几个文件在搬**。
///
/// 为什么上限要有一个具体的默认值而不是"不限"：多开一条传输就多占一份 32 KiB 缓冲与一次
/// 在途请求，而一条 SSH 连接上的在途请求不是无限的（上游按请求 id 复用同一条通道）。
/// 上限的作用是把等待折叠起来（小文件的墙钟时间几乎全是一次次往返的叠加，`scope.md` §4.1），
/// 而不是把对端排满。
///
/// 取值口径与 [`Reconnect::default`] 一致：**默认值写在配置模型里**，因为它是一条判据的
/// 唯一来源（"上限是多少"不该散在调用处）。文件形态（`config.json`）还没有 —— 与 plan 0605
/// 对 `Reconnect` 的处理相同：先有默认值，文件与配置界面一起做。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    /// 同时有几个文件在搬。`0` 会被引擎兜成 `1`（"零个"不是一种能工作的配置）。
    pub in_flight: u32,
}

impl Default for Transfer {
    /// `8`：**占用与吞吐的折中**，不是饱和点。
    ///
    /// 上限同时是对端打开句柄数与本机缓冲数的上界（每条传输一份 32 KiB 缓冲），所以它必须
    /// 是一个数而不是"不限"。plan 0704 的分档实测（12 个 1 KiB 文件 + 一条每个方向延后
    /// 10 ms 的链路）是：串行 1.17 s、上限 8 的 0.38 s（3.1×）、上限 16 的 0.21 s（5.5×）——
    /// 再往上仍然更快，而每一条多出来的传输都要多占一个句柄与一份缓冲。8 落在通行的折中
    /// 位置上；那组数字与其中还没解释清楚的一段记在
    /// `docs/plans/archive/0704-pipelining.md` 的实施记录里。
    fn default() -> Self {
        Self { in_flight: 8 }
    }
}

/// 生效的配置。
///
/// 每个字段都不是裸值：配置项会变多，而"调用方拿的是整份配置"这件事不该跟着变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Config {
    /// 关窗行为（见 [`CloseBehavior`]）。
    pub close_behavior: CloseBehavior,
    /// 掉线重连（见 [`Reconnect`]）。
    pub reconnect: Reconnect,
    /// 传输引擎的并发上限（见 [`Transfer`]）。
    pub transfer: Transfer,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试里用 `expect` 是断言手段（同 `registry.rs` 的说明）。
    #[test]
    fn close_behavior_parses_its_own_names() {
        for behavior in CloseBehavior::ALL {
            assert_eq!(
                CloseBehavior::parse(behavior.as_str()),
                Some(behavior),
                "as_str 与 parse 必须是同一套取值 —— 否则配置文件与日志会分叉"
            );
        }
    }

    #[test]
    fn close_behavior_rejects_anything_else() {
        for value in ["", "Tray", "TRAY", "exitt", "exit ", "true", "1"] {
            assert_eq!(
                CloseBehavior::parse(value),
                None,
                "{value:?} 不该被猜成一个取值"
            );
        }
    }

    #[test]
    fn default_is_tray() {
        assert_eq!(Config::default().close_behavior, CloseBehavior::Tray);
        assert_eq!(CloseBehavior::default(), CloseBehavior::Tray);
    }

    #[test]
    fn exit_behavior_ignores_the_tray() {
        for tray_ready in [true, false] {
            assert_eq!(
                CloseAction::decide(CloseBehavior::Exit, tray_ready),
                CloseAction::Exit
            );
        }
    }

    #[test]
    fn tray_behavior_hides_when_the_tray_exists() {
        assert_eq!(
            CloseAction::decide(CloseBehavior::Tray, true),
            CloseAction::Hide
        );
    }

    /// 问题 #60：只读 runtime dir / 容器 / 没有托盘宿主的机器上**建不起托盘**，
    /// 这时若还"关窗 = 隐藏"，窗口就再也叫不回来了 —— 用户看见的是一个收不回的进程。
    #[test]
    fn tray_behavior_degrades_to_exit_without_a_tray() {
        assert_eq!(
            CloseAction::decide(CloseBehavior::Tray, false),
            CloseAction::Exit
        );
    }

    #[test]
    fn action_names_are_ascii_and_distinct() {
        assert_eq!(CloseAction::Hide.as_str(), "hide");
        assert_eq!(CloseAction::Exit.as_str(), "exit");
    }

    /// D13 的三个数：**3 次**，退避 `1s → 2s → 4s`。
    #[test]
    fn the_default_retry_budget_is_three_tries_with_doubling_waits() {
        let policy = Reconnect::default();
        assert_eq!(policy.max_attempts, 3);
        assert_eq!(policy.initial, Duration::from_secs(1));
        assert_eq!(policy.factor, 2);
        assert_eq!(Config::default().reconnect, policy);

        // 序列写全，不引用实现里的循环 —— 把算式抄进用例等于没测。
        let waits: Vec<Option<Duration>> = (1..=4).map(|n| policy.delay(n)).collect();
        assert_eq!(
            waits,
            vec![
                Some(Duration::from_secs(1)),
                Some(Duration::from_secs(2)),
                Some(Duration::from_secs(4)),
                None,
            ],
            "第 4 次没有预算 —— 那正是「次数耗尽」的判据"
        );
        assert_eq!(policy.budget(), Duration::from_secs(7));
    }

    /// `0` 次是合法的配置（= 不自动重连），而"第 0 次重试"不是合法的一步
    /// （与 `TunnelState::Reconnecting` 的"次数必须 ≥ 1"同一条口径）。
    #[test]
    fn the_zeroth_attempt_never_has_a_budget() {
        let policy = Reconnect::default();
        assert_eq!(policy.delay(0), None);
        let off = Reconnect {
            max_attempts: 0,
            ..Reconnect::default()
        };
        assert_eq!(off.delay(1), None, "关掉重连时第一次也不该等");
        assert_eq!(off.budget(), Duration::ZERO);
    }

    /// 次数与倍率都是配置里的数（用户能给任意值），所以极端取值不能 panic ——
    /// 乘法要饱和。
    #[test]
    fn an_absurd_factor_saturates_instead_of_overflowing() {
        let policy = Reconnect {
            max_attempts: 64,
            initial: Duration::from_secs(u64::MAX / 2),
            factor: 3,
        };
        assert!(policy.delay(64).is_some(), "次数内一律给得出等待时长");
        assert!(policy.budget() <= Duration::MAX, "总预算也要饱和");
    }
}
