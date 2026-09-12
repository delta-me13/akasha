// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
pub mod bindings;
pub mod session;
pub mod tray;
pub mod watchdog;

use session::{Sessions, ShutdownReport};

/// 模板留下的探针命令：用来验证 IPC 通道本身是通的（`docs/STATUS.md` 的 IPC 端到端检查）。
///
/// `#[specta::specta]` 是必须的 —— 少了它，`collect_commands!` 会直接编译失败
/// （比"前端调不到"这种运行期错误好得多）。
#[tauri::command]
#[specta::specta]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// 启动 app。
///
/// ⚠️ **这一节是"真正退出"的落点**（plan 0204）。三条路径各有各的限制：
///
/// | 路径 | 谁在跑回收 |
/// |---|---|
/// | 关窗口 / 正常退出 | `RunEvent::Exit`（下面） |
/// | panic | panic hook（[`install_panic_reclaim`]） |
/// | `tauri dev` 重编译重启 / `kill -9` / `kill -TERM` | **app 里没人能跑**（SIGKILL 不可捕获）：改由[看门狗](crate::watchdog)在**另一个进程**里收 —— 它读一条管道，app 一死就收到 EOF。见 plan 0205 |
///
/// 日志走 `tauri-plugin-log`；`tracing` 的事件靠 `tracing/log-always` 特性转发成 `log`
/// 记录 —— 没有这一步，那些回收记录会**静默消失**（app 里没有 tracing subscriber）。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 命令清单只有 `bindings::builder()` 一处：**同一份**既喂 `invoke_handler`
    //（运行期分发），也喂 `just gen-types`（生成 TS）。分成两份必然漂移。
    let builder = bindings::builder();

    // 会话表要**共享**给退出钩子与 panic hook —— tauri 的 `State` 只借给命令用。
    // `Sessions` 内部是 `Arc`，clone 出来的是同一份表。
    //
    // 看门狗在这里起：它是第四道回收，专管"进程里没有任何代码能跑"的那条路径
    // （`tauri dev` 重载 = SIGKILL、`kill -9`、`kill -TERM`）—— plan 0205。
    // 起不来就退化回 plan 0204 的三条路径，**不挡启动**（理由见 `watchdog` 模块）。
    //
    // ⚠️ 起得比日志插件早，所以**不能在这里记日志**（那时 `tracing` 没有 `log` 出口，
    // 记录会静默消失）；`Startup` 把结果留到 `.setup()` 里再报 —— 见 `watchdog::Startup`。
    let sessions = Sessions::default();
    let startup = watchdog::start_early(&sessions);
    install_panic_reclaim(sessions.clone());

    let app = tauri::Builder::default()
        .plugin(logger())
        .plugin(tauri_plugin_opener::init())
        .manage(sessions.clone())
        .invoke_handler(builder.invoke_handler())
        .plugin(victauri_plugin::init())
        // 到这一步日志插件已经就绪 —— 看门狗的成败终于有人看得到（`Startup` 的理由）。
        .setup(move |app| {
            watchdog::report(&startup);
            // ⚠️ 事件必须在 setup 里挂上：`tauri-specta` 的 `Builder::invoke_handler`
            // 只覆盖命令，事件缺了这一步会在**发**的时候 panic（`EventRegistry not found`）。
            builder.mount_events(app);
            // 托盘在窗口与会话表都就绪之后建。返回"可用吗" —— plan 0302 拿它决定
            // "关窗口 = 隐藏还是真关掉"（没有托盘就没有能叫回窗口的地方）。本步还没有消费者。
            let _tray_ready = tray::setup(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(move |_handle, event| {
        // ⚠️ 只挂 `Exit`（**不可回头**的那一刻），**不挂** `ExitRequested`：
        // 阶段 3 的"点叉收托盘"正是在 `ExitRequested` 里 `prevent_exit` 的
        //（plan 0301/0302）。在那里收会话 = "窗口收进托盘、终端却全被杀掉"，
        // 与托盘语义正好相反 —— 所以回收点必须晚于"退出已成定局"。
        if let tauri::RunEvent::Exit = event {
            // 这一条比看门狗**更早、更精确**（进程还活着，能逐个 kill + wait 收尸），
            // 所以两条路径不是二选一：能跑代码的时候跑这里，跑不了的时候才轮到看门狗。
            log_reclaim(&sessions.shutdown_all(), "exit");
        }
    });
}

/// 收尾结果的一条汇总：**数字进字段，每个会话的细节在 `shutdown_all` 里各自记**
/// （这里不 `?` 打 `failures` —— 那是把 `Vec<(u32, String)>` 的 `Debug` 倒进日志）。
///
/// 形态规则见 `docs/logging.md`；`trigger` 取值是稳定的字面量
/// （`exit` / `panic` / `tray`）。
pub(crate) fn log_reclaim(report: &ShutdownReport, trigger: &'static str) {
    if report.is_clean() {
        tracing::info!(reclaimed = report.shut_down, trigger, "sessions reclaimed");
    } else {
        tracing::error!(
            reclaimed = report.shut_down,
            failed = report.failures.len(),
            trigger,
            "sessions not reclaimed"
        );
    }
}

/// panic 路径的回收（`AGENTS.md` §3.3 表格里"panic → 回收"那一格）。
/// 不做这件事的后果很具体：panic 之后**死掉的只有 app**，PTY 里的 shell 与它的作业
/// 靠内核 hangup 带走 —— 而**明确忽略 SIGHUP 的进程**（`nohup` / 守护化的）会留下来，
/// 用户看不见也关不掉（实测见 plan 0204）。
///
/// 顺序是有意的：
///
/// 1. 先把崩溃现场交给**上一个** hook —— 那是证据，不能被我们吃掉；
/// 2. 尽力回收（此时进程状态已经不可信，所以只求"能收多少收多少"，失败只记日志）；
/// 3. `abort()`。这是刻意的：panic 过的终端不该继续假装能工作；release profile
///    本来就是 `panic = "abort"`，这一步只是让 debug 与 release 行为一致。
fn install_panic_reclaim(sessions: Sessions) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        log_reclaim(&sessions.shutdown_all(), "panic");
        std::process::abort();
    }));
}

/// 日志落点：**只走 stdout**，暂不写日志目录；级别**显式定在 Info**。
///
/// 为什么不带 `TargetKind::LogDir`（那是 `tauri-plugin-log` 的默认目标之一）：
/// 它在**创建日志目录失败时会让插件初始化失败**，而插件初始化失败 = **app 起不来**。
/// 这不是理论风险：`$HOME` 只读的环境里实测直接 abort ——
/// `PluginInitialization("log", "只读文件系统 (os error 30)")`。
/// 一个终端不该因为"日志文件写不了"就打不开。文件日志与轮转（`AGENTS.md` §3.4）
/// 等能做到"写不了就降级"时再加。
///
/// 为什么级别要显式写：`tracing/log-always` 会把**依赖树里**的 tracing 事件一起
/// 转成 `log` 记录（rmcp 每个 MCP 请求好几条，还有 `tracing::span::active` 这种
/// 平时看不见的内部事件）。默认 Trace 下实测把 app 日志刷成几十万行、
/// 并且明显拖慢 app —— 一个终端不该被自己的日志拖垮。
fn logger() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_log::{Target, TargetKind, log::LevelFilter};

    tauri_plugin_log::Builder::new()
        .targets([Target::new(TargetKind::Stdout)])
        .level(LevelFilter::Info)
        .build()
}
