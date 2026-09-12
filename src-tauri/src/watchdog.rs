//! 看门狗在 **app 这一侧**的接法（机制本身在 `akasha_pty::watchdog`，plan 0205）。
//!
//! 这里只有两件事，都必须在 Tauri 起来**之前**决定：
//!
//! 1. [`run_if_watchdog`]：这次启动是不是"以看门狗身份跑"。看门狗**就是本可执行
//!    文件**再跑一次（见 `SessionWatchdog::spawn` 为什么不用单独的 bin），所以这个
//!    判断必须待在 `main` 的第一行 —— 跑到 Tauri 里再去认领，就已经起了一个窗口。
//! 2. [`start`]：正常启动时把看门狗起起来，交给 [`akasha_lib::session::Sessions`]。
//!
//! 两者的失败都**不致命**：app 该照常能用，只是少了"被 SIGKILL 时的最后一道兜底"
//! （退化回 plan 0204 的水平）。一个终端不该因为兜底进程起不来就打不开。

use akasha_pty::watchdog::{self, SessionWatchdog};

use crate::session::Sessions;

/// 以看门狗身份跑：读协议，到 EOF 收掉还登记着的会话，然后退出。
///
/// 返回 `true` 表示"这次启动已经被看门狗模式认领了"，调用方**必须**就此返回 ——
/// 看门狗没有窗口、没有 IPC、也不该碰 Tauri。
pub fn run_if_watchdog() -> bool {
    if !watchdog::is_invocation(std::env::args_os()) {
        return false;
    }
    // 与终端脱钩：`^C`、关终端这些"打给前台进程组"的信号从此带不走它 ——
    // 而它的职责恰恰是"app 死了它还在"。失败也不致命（只是多了一条被 `^C` 带走的路径）。
    let _ = watchdog::detach();
    // 报告没有读者：走到这里说明 app 已经不在了，而它按设计不输出任何东西。
    let _ = watchdog::run(std::io::stdin().lock());
    true
}

/// 看门狗的启动结果：**先起（并立刻挂上），后记**。
///
/// 为什么要分成两步（这一步是实测踩出来的）：日志插件（`tauri-plugin-log`）是 tauri
/// builder 的一环，在它注册之前 `tracing` 的事件**没有 `log` 出口** —— 那时打出去的
/// 记录会**静默消失**（实测：app 日志里只有 victauri 的 INFO，看门狗那几行一个都没有）。
/// 而看门狗必须**最前面**起：会话一存在就必须已经在兜底清单里，不能等 builder 走完。
///
/// 于是：启动留着不动，把"记一笔"推到 [`report`]（在 `.setup()` 里调，那时日志已就绪）。
pub enum Startup {
    /// 已经起来、并挂到 `Sessions` 上了；这里只留"记一笔"要用的 pid。
    Up { pid: Option<u32> },
    /// 起不来。原因**必须留到能看见的时候**再说 —— 当场丢掉就等于静默降级。
    Down { reason: String },
}

/// 起一个看门狗并挂到 `sessions` 上。**不写日志**（理由见 [`Startup`]）。
pub fn start_early(sessions: &Sessions) -> Startup {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => {
            return Startup::Down {
                reason: format!("current_exe failed: {err}"),
            };
        }
    };
    match SessionWatchdog::spawn(&exe) {
        Ok(watchdog) => {
            let pid = watchdog.pid();
            sessions.attach_watchdog(watchdog);
            Startup::Up { pid }
        }
        Err(err) => Startup::Down {
            reason: format!("spawn failed: {err}; exe = {}", exe.display()),
        },
    }
}

/// 日志就绪之后，把看门狗这件事记一笔（见 [`Startup`]）。
///
/// 正常路径上它只证明"兜底在位"；**失败路径上它是唯一的信号** —— 没有它，
/// "看门狗没起来"这件事在 app 里完全看不出来（只有 plan 0205 的用例会红）。
pub fn report(startup: &Startup) {
    match startup {
        Startup::Up { pid } => match pid {
            Some(pid) => tracing::info!(pid, "watchdog started"),
            // 拿不到 pid 不等于没起来（平台差异），所以照样报"已启动"，只是不带字段。
            None => tracing::info!("watchdog started"),
        },
        Startup::Down { reason } => tracing::warn!(reason = %reason, "watchdog failed to start"),
    }
}
