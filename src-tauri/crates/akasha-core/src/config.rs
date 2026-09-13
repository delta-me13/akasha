//! 配置模型（plan 0303）：**类型、默认值、判据表**；文件读写不在这里（那是 app 的事）。
//!
//! 为什么放 core：`AGENTS.md` §3.1 —— 纯逻辑下沉到不依赖 Tauri 的 crate，脱离 app 可单测。
//! 本模块真正的价值是 [`CloseAction::decide`] 那张表：它把
//! "配置想要什么" 与 "这台机器上实际能做到什么" 分开，于是两条容易写错的规则
//! （`AGENTS.md` §3.3）可以用普通单测钉住。
//!
//! ⚠️ 本 crate 的 `[dependencies]` 为空**是有意为之**（见 `Cargo.toml`），所以这里
//! **不引入任何解析器**：**值**的解析（`"tray"` / `"exit"`）在这里，**文件**的解析在 app 侧。

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

/// 生效的配置。
///
/// 字段目前只有一个，但仍然是结构体而不是裸的 [`CloseBehavior`]：配置项会变多，
/// 而"调用方拿的是整份配置"这件事不该跟着变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Config {
    /// 关窗行为（见 [`CloseBehavior`]）。
    pub close_behavior: CloseBehavior,
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
}
