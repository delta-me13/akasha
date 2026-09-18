// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
pub mod bindings;
pub mod bw;
pub mod config;
pub mod lifecycle;
pub mod pools;
pub mod prompt;
pub mod serial;
pub mod session;
pub mod single_instance;
pub mod ssh;
pub mod tray;
pub mod tunnel;
pub mod vault;
pub mod watchdog;

use crate::config::CloseBehavior;
use session::{Sessions, ShutdownReport};
use tauri::Manager;

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

    // 单实例（plan 0304）：第二个实例唤起已有窗口，而不是各跑一套。
    //
    // 注册在**最前面**（上游 README 也是这么要求的：插件的 setup 在 `build()` 里按注册
    // 顺序跑）—— 第二个实例走到自己这一步就该退，不该先去建日志、连 portal。
    // ⚠️ "第二个实例不会先闪一个窗口"**与顺序无关**：全部插件的 setup 都跑在窗口创建
    //（`RunEvent::Ready`）之前。注册不上去时这里回 `None`（Linux 上没有会话总线，
    // 见该模块），app 照常启动，只是**可以多开**。
    let (instance, instance_plugin) = single_instance::start();

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

    // SSH 的三样长住状态（plan 0504）：专用 runtime（ADR-0003 D2）、内存凭据缓存（D8）、
    // 提问往返。**起 runtime 失败不挡启动**（`AGENTS.md` §3.3）—— 与看门狗同一个处置：
    // 起得比日志插件早，所以**不能在这里记日志**，把原因留到 `.setup()` 里再说。
    //
    // 提问表要单独 `manage` 一份（同一个 `Arc`，不是两份表）：三条回答命令只认它，
    // 而它们不该为了拿一份表去穿过 `Ssh`。
    let bitwarden = bw::Bitwarden::default();
    let ssh = ssh::Ssh::start();
    let ssh_error = ssh.startup_error().map(str::to_owned);
    let prompts = ssh.prompts().clone();

    let mut app_builder = tauri::Builder::default();
    if let Some(plugin) = instance_plugin {
        app_builder = app_builder.plugin(plugin);
    }

    let app = app_builder
        .plugin(logger())
        .plugin(tauri_plugin_opener::init())
        .manage(sessions.clone())
        // 库的解锁状态（plan 0407）。**启动时是锁着的** —— 口令只从 `vault_unlock` 进来，
        // 没有自动解锁，也没有从配置文件 / 环境变量读取的路径（ADR-0002 D5）。
        .manage(vault::Vault::default())
        .manage(ssh)
        .manage(prompts)
        // Bitwarden 的两个轴（plan 0902）：默认取 `host`（用户当次指令）。
        // ⚠️ 这里**不解析也不下载** —— 启动路径上不做网络与磁盘的额外动作
        //（`AGENTS.md` §3.3：可选能力失败不得挡住启动）。
        // ⚠️ 先放默认值，`.setup()` 里再用配置文件里那两个轴与数据目录覆盖它 ——
        // 启动期读不到数据目录（同 `config` 那一条：它在 `.setup()` 里才定下来）。
        .manage(bitwarden.clone())
        .invoke_handler(builder.invoke_handler())
        // 比 `victauri_plugin::init()` 只多注册三个 probe：**关窗语义**（plan 0302/0303）、
        // **单实例**（plan 0304）与**会话表**（plan 0504）。它们给 E2E 一个"这台机器该验哪条、
        // 还是该显式跳过"的判据（`AGENTS.md` §7：观察后端状态用 probe 读，不靠 grep 日志反推）。
        // `build()` 只在 port / 容量这类配置非法时失败，而这里全是默认值 —— 与
        // `victauri_plugin::init()` 内部的 `expect` 是同一条保证（默认配置永远合法）。
        .plugin(
            victauri_plugin::VictauriBuilder::new()
                .probe("lifecycle", lifecycle::snapshot)
                .probe("single_instance", single_instance::snapshot)
                .probe("sessions", {
                    let sessions = sessions.clone();
                    move || session::snapshot(&sessions)
                })
                // 隧道（plan 0601）：`sessions` 那个探针回答"有没有人管得着"，
                // 这一份回答"**哪一条**现在是什么状态" —— "失败必须可见"（`scope.md` §2.2）
                // 的机器可读那一半。
                .probe("tunnels", {
                    let sessions = sessions.clone();
                    move || tunnel::snapshot(&sessions)
                })
                // SFTP（plan 0701）：判据"两侧各自列目录成功"的读数口 ——
                // 两侧各自的状态与当前目录都在这里（一个 SFTP 会话 = 一条记录）。
                .probe("sftp", {
                    let sessions = sessions.clone();
                    move || ssh::ipc::sftp::snapshot(&sessions)
                })
                // 托盘（plan 0605）：隧道那几行文字是"失败必须可见"（`scope.md` §5.2）
                // 的落点，而这一份报的就是菜单上写的那些字。
                .probe("tray", {
                    let sessions = sessions.clone();
                    move || tray::snapshot(&sessions)
                })
                // Bitwarden（plan 0902）：两个轴解析出来是什么、CLI 自报的版本与变体、
                // **我们手里有没有 session key**（不是 key 本身）。
                // ⚠️ 它**不起进程**：读的是上一次动作留下的读数。
                .probe("bitwarden", {
                    let bitwarden = bitwarden.clone();
                    move || bw::probe(&bitwarden)
                })
                // **关闭之后还剩什么**（plan 0606）：判据"关闭转发 Session 后连接数与重连任务数
                // 都归零"（D5）的机器可读那一半。
                //
                // 为什么不能拿 `tunnels` 那份当证据：它数的是注册表里的实体，而关闭命令自己
                // 就会把实体摘掉 —— "表里没了"只是那条命令的效果。这两个数说的是**资源本身**
                // 还在不在（连接对象归转发任务持有、看护任务归 runtime 持有，都与实体表无关）。
                .probe("residue", || {
                    serde_json::json!({
                        "sshConnections": crate::ssh::live_connections(),
                        "watchTasks": tunnel::watch_tasks(),
                    })
                })
                .build()
                .expect("default Victauri configuration is always valid"),
        )
        // 到这一步日志插件已经就绪 —— 看门狗与单实例的成败终于有人看得到
        //（两者的 `Startup` 都是同一个理由）。
        .setup(move |app| {
            // 便携模式下**数据目录写不进去** → 拒绝启动（`docs/portable.md` §4 第 3 条）。
            //
            // 为什么是拒绝而不是替他退回 OS 目录：那个目录本身就是"我要便携"的标记，
            // 替他决定就等于把数据写到他看不见的地方 —— 他拔了 U 盘才发现。
            // ⚠️ **只在这条路上拒绝**：退回 OS 目录的那条路上"写不了"仍然只是降级
            //（配置读不到就取默认值，`config::load`），那从来不是用户的明确要求。
            //
            // 位置选在 `.setup()` 的开头：此时各插件的 setup 已经跑过（日志出口就绪，
            // 这条错误才**说得出来**），而窗口与托盘还没建（拒绝是"什么都没发生"地退出）。
            // 退出码见 `config::EXIT_NOT_WRITABLE`，E2E 按它判定。
            if let Some(dir) = config::portable_data_dir()
                && let Err(err) = config::require_writable(&dir)
            {
                tracing::error!(
                    %err,
                    path = %dir.display(),
                    "portable data dir not writable"
                );
                std::process::exit(config::EXIT_NOT_WRITABLE);
            }
            watchdog::report(&startup);
            single_instance::report(&instance);
            // SSH 那三样起不来时只降级：app 照常能用，只是开不了 SSH 会话
            //（命令会回一句"runtime 起不来"）。这里才说得出来 —— 日志出口刚刚就绪。
            if let Some(reason) = &ssh_error {
                tracing::error!(reason = reason.as_str(), "ssh runtime unavailable");
            }
            // ⚠️ 事件必须在 setup 里挂上：`tauri-specta` 的 `Builder::invoke_handler`
            // 只覆盖命令，事件缺了这一步会在**发**的时候 panic（`EventRegistry not found`）。
            builder.mount_events(app);
            // 提问往返接上真正的前端（**必须在 `mount_events` 之后**：事件没注册时
            // `emit` 会 panic）。装在这里而不是启动时，是因为它依赖上面那一句。
            app.state::<prompt::Prompts>()
                .to_frontend(app.handle().clone());
            // 关窗语义的两个输入在这里定下来。**顺序有讲究**：先读配置、再建托盘、
            // 最后登记 —— 判据要同时看这两样，而"托盘建成没有"只有 `tray::setup` 的
            // 返回值知道，事后没人能再问出来。
            let config = config::load(app.handle());
            // Bitwarden 的两个轴（plan 0902）：配置文件里写了就用它，没写就是默认的 `host`。
            // ⚠️ 只读、不下载、不解析可执行文件 —— 那几步都在用户动作里做（启动路径上不碰网络）。
            if let Some(dir) = config::data_dir_of(app.handle()) {
                let settings = bw::settings_from_config(&dir);
                app.state::<bw::Bitwarden>().configure(dir, settings);
            }
            // 生效的配置登记成状态：命令侧（隧道重连的预算）要读它。
            // ⚠️ 登记的是**生效的那一份**（含默认值），不是文件里的原文。
            app.manage(config);
            let tray_ready = tray::setup(app.handle());
            let lifecycle = lifecycle::record(config.close_behavior, tray_ready);
            // 配置要"收托盘"、但这台机器上**建不起托盘** → 实际动作降级为"直接退出"
            //（§3.3 那条规则要求把降级之后的行为一起定下来）。这是用户能直接看见的
            // 行为差异，所以记 warn，不是 debug。
            if lifecycle.close_behavior() == CloseBehavior::Tray && !lifecycle.tray_ready() {
                tracing::warn!(
                    close_action = lifecycle.close_action().as_str(),
                    "close behavior degraded"
                );
            }
            Ok(())
        })
        .on_window_event(lifecycle::on_window_event)
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(move |_handle, event| {
        // ⚠️ 只挂 `Exit`（**不可回头**的那一刻），**不挂** `ExitRequested`：
        //
        //   * 在 `ExitRequested` 里收会话是错的：那一刻**还可能被 `prevent_exit` 拦回来**
        //     （托盘模式下就是如此），于是"窗口收进托盘、终端却全被杀掉" —— 与托盘语义相反。
        //   * 也**不**在那里 `prevent_exit`：`AppHandle::exit()`（托盘菜单的"退出"走它）
        //     **同样会触发 `ExitRequested`** —— 拦它等于把唯一的退出入口也拦掉。
        //     而关窗那条路已经在 `CloseRequested` 里拦下（`lifecycle` 的 `prevent_close`），
        //     窗口根本不会被销毁，轮不到这里兜。见 plan 0302 的实施记录。
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
