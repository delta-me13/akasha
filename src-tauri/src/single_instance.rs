//! 单实例（plan 0304）：第二个实例**唤起已有窗口**，而不是各跑一套。
//!
//! # 为什么必须处理
//!
//! 各跑一套的后果不是"多一个窗口"，而是**两条隧道指向同一个目标、两套托盘图标**，
//! 以及"退出一个还剩一个"（`scope.md` §5.5）。所以第二个实例要做的事只有一件：
//! 把已经在跑的那个窗口叫回来，然后自己退掉。
//!
//! # 为什么"唤起"要在这里显式做
//!
//! plan 0302 之后，**窗口不在屏幕上是一种常态**（点叉 = 收托盘，进程与终端原样留着）。
//! 一个隐藏窗口是 `set_focus()` 不出来的 —— 用户看到的是"点了图标，什么都没发生"。
//! 所以唤起 = **还原（若最小化）→ 显示 → 置前**，三步都做，见 [`activate`]。
//!
//! # 注册交给插件，判据只有一条
//!
//! 机制按平台分（Linux = D-Bus 会话总线上的一个名字、Windows = 命名 mutex、
//! macOS = `/tmp` 下的 unix socket），注册本身交给 `tauri-plugin-single-instance`。
//! 其中**只有 Linux 依赖外部服务**：容器、CI 的 xvfb 里没有会话总线，注册不上去，
//! app 就退化成"可以多开"。这条降级必须**看得见**（`AGENTS.md` §3.3：启动路径上的
//! 可选能力失败不得挡住启动，但降级之后的行为要一起定下来），所以：
//!
//! 1. 注册之前先问一次会话总线（[`available`]）；
//! 2. 问到的结果记进 probe（[`snapshot`]），E2E 靠它决定"真跑还是跳过"；
//! 3. 那条 `warn` 留到 `.setup()` 里再打（[`report`]）—— 日志插件是 builder 的一环，
//!    在它注册之前 `tracing` 没有 `log` 出口，那时打出去的记录会**静默消失**（坑 #47）。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tauri::{AppHandle, Manager, Wry};

/// 主窗口的 label：`tauri.conf.json` 的 `app.windows[0]` 没写 `label`，tauri 的默认值就是它。
const MAIN_LABEL: &str = "main";

/// 已经发生过几次"第二个实例来敲门"。
static ACTIVATIONS: AtomicU64 = AtomicU64::new(0);
/// 本次启动**注册了没有**。
static REGISTERED: AtomicBool = AtomicBool::new(false);

/// 单实例在这次启动里的结局。与 `watchdog::Startup` 同一个理由：**先决定、后记**
/// （日志插件是 builder 的一环，坑 #47）。
pub enum Startup {
    /// 已注册：第二个实例会被叫回已有窗口。
    Registered,
    /// 这台机器上注册不了 —— app 照常启动，只是**可以多开**。
    Unavailable,
}

/// 注册单实例机制；`None` = 这台机器上注册不了（[`available`]）。
///
/// 调用方把它注册在**最前面**（上游 README 也这么要求）：插件的 setup 在 `build()` 里
/// 按注册顺序跑，排在前面就意味着第二个实例在别的插件的 setup 之前就退掉。
/// ⚠️ **不是**为了"避免闪一个窗口"：全部插件的 setup 都跑在窗口创建（`RunEvent::Ready`）之前。
pub fn start() -> (Startup, Option<tauri::plugin::TauriPlugin<Wry>>) {
    if !available() {
        return (Startup::Unavailable, None);
    }
    REGISTERED.store(true, Ordering::Relaxed);
    let plugin = tauri_plugin_single_instance::init(on_second_instance);
    (Startup::Registered, Some(plugin))
}

/// 启动的结局记一笔 —— 调用点在 `.setup()`（理由见模块文档，坑 #47）。
pub fn report(startup: &Startup) {
    match startup {
        Startup::Registered => tracing::info!("single instance registered"),
        // 用户能直接看见的行为差异（同一个 app 可以开两份），所以是 warn 不是 debug。
        Startup::Unavailable => tracing::warn!("single instance unavailable"),
    }
}

/// 第二个实例来敲门时**在已有实例里**跑。
///
/// payload（argv + cwd）在 v1 **不消费** —— 本 app 没有"用命令行打开一个文件 / 一条
/// 连接"的入口（`scope.md` 的能力清单里没有），能做的只有一件事：把窗口叫回来。
/// 将来真有了 deep-link 之类，分发点就在这里。
fn on_second_instance(app: &AppHandle<Wry>, _args: Vec<String>, _cwd: String) {
    let activations = note_activation();
    tracing::info!(activations, "second instance activated");
    activate(app);
}

/// 把已有窗口叫回来：**还原（若最小化）→ 显示 → 置前**。
///
/// 三步**都做、不先问窗口的状态**：最小化的窗口 `is_visible()` 仍然为真，按可见性
/// 分支就会漏掉"还原"；反过来，已经可见、已经置前时这三个调用本身就是空操作。
/// 少一次状态判断，就少一处"状态读错了"的可能。
///
/// 每一步失败只记一条日志：叫不回来的是用户面前那个窗口，而**正在跑的这个实例**
/// 不该因为这一步失败再出别的问题。
fn activate(app: &AppHandle<Wry>) {
    let Some(window) = app.get_webview_window(MAIN_LABEL) else {
        // 正常路径上不会发生：窗口是**隐藏**不是销毁（plan 0302），它一直在。
        tracing::warn!(label = MAIN_LABEL, "window activation target missing");
        return;
    };
    report_step("unminimize", window.unminimize());
    report_step("show", window.show());
    report_step("focus", window.set_focus());
}

/// 唤起的一步失败了只记日志（见 [`activate`]）。
fn report_step(step: &'static str, result: tauri::Result<()>) {
    if let Err(err) = result {
        tracing::warn!(%err, step, "window activation step failed");
    }
}

/// 记一次"第二个实例来敲门"，返回这是第几次。
fn note_activation() -> u64 {
    ACTIVATIONS.fetch_add(1, Ordering::Relaxed) + 1
}

/// `app_state { probe: "single_instance" }` 的返回（在 `lib.rs` 注册）。
///
/// 两个字段各自有用途：`registered` 决定 E2E **真跑还是跳过**，`activations` 是
/// "第二个实例的话**真的带到了这个进程**"的证据。
pub fn snapshot() -> serde_json::Value {
    snapshot_of(
        REGISTERED.load(Ordering::Relaxed),
        ACTIVATIONS.load(Ordering::Relaxed),
    )
}

fn snapshot_of(registered: bool, activations: u64) -> serde_json::Value {
    serde_json::json!({
        "registered": registered,
        "activations": activations,
    })
}

/// 这台机器上能不能注册单实例机制。
///
/// Linux 上判据取"会话总线连得上吗" —— 与插件随后要建立的那条连接是**同一个**
/// （`$DBUS_SESSION_BUS_ADDRESS`，或退回到 `$XDG_RUNTIME_DIR/bus`）。没有它，
/// 插件的注册会失败并**静默**退化成"可以多开"，而我们要把这件事说出来。
#[cfg(target_os = "linux")]
fn available() -> bool {
    zbus::blocking::Connection::session().is_ok()
}

/// Windows（命名 mutex）与 macOS（`/tmp` 下的 unix socket）都不依赖外部服务。
#[cfg(not(target_os = "linux"))]
fn available() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reports_registration_and_activations() {
        let idle = snapshot_of(false, 0);
        assert_eq!(idle["registered"], false);
        assert_eq!(idle["activations"], 0);

        let served = snapshot_of(true, 3);
        assert_eq!(served["registered"], true);
        assert_eq!(served["activations"], 3);
    }

    #[test]
    fn activations_are_counted_from_one() {
        let before = ACTIVATIONS.load(Ordering::Relaxed);
        assert_eq!(note_activation(), before + 1);
        assert_eq!(note_activation(), before + 2);
    }
}
