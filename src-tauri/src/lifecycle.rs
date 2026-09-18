//! 关窗语义（plan 0302 + 0303）：**点了窗口上的叉，到底发生什么**。
//!
//! 两件事在这里合流：
//!
//! * **0302「隐藏而非销毁」** —— 窗口 `hide()`，进程、会话、终端缓冲全留着；
//! * **0303「关闭行为可配置」** —— `close_behavior` 决定收托盘还是直接退出。
//!
//! ⚠️ 这条语义**不能只当"窗口的事"来处理**：`Sessions` 的生命期挂在**进程**上，
//! 销毁窗口会连带丢掉所有会话（终端缓冲在前端，一起没），与"连接 = Session 生命期"
//! （`scope.md` §2.2）正好相反。所以窗口关闭**不是回收时机**（`AGENTS.md` §3.3 的表）：
//! 本模块既不发命令、也不收会话，只是决定"关还是不关"。
//!
//! 判据本身（配置 × 托盘可用性 → 动作）在 `config` 模块，脱离 app 可单测；
//! 这里只做三件事：**记住启动那一刻的事实**、把判据接到窗口事件上、给 Victauri
//! 提供一个能读它的 probe（`AGENTS.md` §7：观察后端状态用 probe，不靠 grep 日志反推）。

use std::sync::OnceLock;

use crate::config::{CloseAction, CloseBehavior};
use tauri::{Window, Wry};

/// 启动那一刻定下来的两个事实。**两个都在 `.setup()` 里写一次**，之后只读。
///
/// 为什么不是"每次关窗时现问"：托盘**是不是真的建成了**只有启动那一刻知道
/// （`tray::setup` 的返回值），事后没有第二个人能问出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifecycle {
    close_behavior: CloseBehavior,
    tray_ready: bool,
}

/// 全进程一份。用 `static` 而不是 Tauri 的 `State`：Victauri 的 probe 要在
/// `.plugin()` 阶段就拿到它，而那时 app 还没有 —— 它是**启动期事实**，不是请求级状态。
static LIFECYCLE: OnceLock<Lifecycle> = OnceLock::new();

/// 登记启动期事实。重复调用不覆盖：这两个值描述的是启动那一刻，后到的调用者不可能更清楚。
pub fn record(close_behavior: CloseBehavior, tray_ready: bool) -> &'static Lifecycle {
    LIFECYCLE.get_or_init(|| Lifecycle {
        close_behavior,
        tray_ready,
    })
}

/// 托盘到底建成了没有。**还没登记就是"没有"** —— 那个值描述的是启动那一刻的事实，
/// 而"没跑过 `record`"只可能是启动还没走到那一步。
///
/// 谁在问：`tray` probe 要据它说清"这台机器上有没有托盘"（几行没有去处的文字
/// 不该被当成"状态不可见"，见 `crate::tray::snapshot`）。
pub fn tray_ready() -> bool {
    LIFECYCLE
        .get()
        .is_some_and(|lifecycle| lifecycle.tray_ready())
}

/// 关窗判据（表见 [`CloseAction::decide`] 与 `AGENTS.md` §3.3）。
///
/// 还没登记就按**退出**处理：那是"没有托盘"时的降级行为，也是唯一不会把用户关在
/// "窗口没了、进程还在、也没地方叫回来"里的取值（问题 #60）。
fn close_action() -> CloseAction {
    LIFECYCLE
        .get()
        .map_or(CloseAction::Exit, |lifecycle| lifecycle.close_action())
}

/// `app_state { probe: "lifecycle" }` 的返回（在 `lib.rs` 注册）。
///
/// ⚠️ 还没登记时**绝不编一个动作出来**（`docs/logging.md`：值不撒谎）——
/// E2E 正是靠这个字段决定"该验隐藏还是该跳过"。
pub fn snapshot() -> serde_json::Value {
    match LIFECYCLE.get() {
        Some(lifecycle) => lifecycle.snapshot(),
        None => serde_json::json!({ "initialized": false }),
    }
}

impl Lifecycle {
    /// 用户想要的行为（配置里的那个值）。
    pub const fn close_behavior(self) -> CloseBehavior {
        self.close_behavior
    }

    /// 托盘到底建成了没有（`tray::setup` 的返回值）。
    pub const fn tray_ready(self) -> bool {
        self.tray_ready
    }

    /// 实际会怎么做。
    pub const fn close_action(self) -> CloseAction {
        CloseAction::decide(self.close_behavior, self.tray_ready)
    }

    /// probe 的快照：两个输入 + 由它们推出的动作，一起去 ——
    /// 只报动作的话，"为什么是它"就得靠人脑再推一遍。
    fn snapshot(self) -> serde_json::Value {
        serde_json::json!({
            "close_behavior": self.close_behavior.as_str(),
            "tray_ready": self.tray_ready,
            "close_action": self.close_action().as_str(),
        })
    }
}

/// `.on_window_event` 的落点：**只认关窗请求**，别的事件原样放行。
///
/// `Hide` 分支的顺序是有讲究的：**先隐藏、成功了才拦下关闭**。反过来（先
/// `prevent_close` 再隐藏）在隐藏失败时会留下一个"点了叉没反应"的窗口 ——
/// 那正是用户没法理解、也没法绕过的状态。
pub fn on_window_event(window: &Window<Wry>, event: &tauri::WindowEvent) {
    let tauri::WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };

    match close_action() {
        // 放行：窗口照常被销毁 → 最后一个窗口消失 → 进程退出 → `RunEvent::Exit` 收尾（0204）。
        CloseAction::Exit => {}
        CloseAction::Hide => match window.hide() {
            Ok(()) => api.prevent_close(),
            Err(err) => {
                tracing::warn!(%err, window = window.label(), "window hide failed");
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reports_the_inputs_and_the_action() {
        let lifecycle = Lifecycle {
            close_behavior: CloseBehavior::Tray,
            tray_ready: true,
        };
        assert_eq!(lifecycle.close_behavior(), CloseBehavior::Tray);
        assert!(lifecycle.tray_ready());
        assert_eq!(lifecycle.close_action(), CloseAction::Hide);

        let snapshot = lifecycle.snapshot();
        assert_eq!(snapshot["close_behavior"], "tray");
        assert_eq!(snapshot["tray_ready"], true);
        assert_eq!(snapshot["close_action"], "hide");
    }

    /// 没有托盘时 E2E 要跳过"隐藏"那条判据，而它靠的就是这个字段。
    #[test]
    fn snapshot_degrades_without_a_tray() {
        let lifecycle = Lifecycle {
            close_behavior: CloseBehavior::Tray,
            tray_ready: false,
        };
        assert_eq!(lifecycle.snapshot()["close_action"], "exit");
    }

    /// 静态只写一次 —— 这条保证"probe 读到的"与"窗口事件用的"是同一份事实。
    #[test]
    fn recording_twice_keeps_the_first_value() {
        let first = record(CloseBehavior::Exit, true);
        let second = record(CloseBehavior::Tray, false);
        assert_eq!(first, second);
        assert_eq!(second.close_behavior(), CloseBehavior::Exit);
        assert!(second.tray_ready());
    }
}
