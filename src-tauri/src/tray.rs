//! 系统托盘（plan 0301）：图标 + 菜单 —— 显示/隐藏窗口、**隧道列表**、退出。
//!
//! 菜单是 `docs/scope.md` §5.2 指定的**"失败可见"落点**：隧道重连耗尽次数之后，
//! 必须有个地方让用户看见"它已经死了" —— 否则就是最危险的失败模式：用户以为隧道还在
//! 转发，实际早就断了。隧道实体在阶段 6，本步先把菜单、**稳定的菜单 id**，以及
//! "会话表一变就重推菜单"这条管线立起来；列表内容此刻必然是空的。
//!
//! ⚠️ **建不起托盘不挡启动**（理由同 `lib.rs` 的 `logger`）：Linux 上图标要落到
//! `$XDG_RUNTIME_DIR/tray-icon`（`tray-icon` 的 `temp_icon_path`），只读 runtime dir /
//! 容器里会失败。失败时只记一条日志，调用方据此决定关窗语义（plan 0302）。
//!
//! 为什么**不动** `capabilities/*.json`：整个托盘都在 Rust 侧建，前端一次都不碰
//! （`AGENTS.md` §4.3 最小权限）。给 `core:tray:*` 只会把"改托盘"的能力白送给 webview。

use akasha_core::TunnelState;
use tauri::menu::{IsMenuItem, Menu, MenuBuilder, MenuEvent, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::Sessions;
use crate::session::SessionHandle;

/// 托盘 id。刷新菜单时按它取回句柄（`AppHandle::tray_by_id`）。
const TRAY_ID: &str = "akasha";

/// 主窗口的 label。`tauri.conf.json` 里那个窗口没写 label，tauri 给的默认值就是 `main`。
pub const MAIN_WINDOW: &str = "main";

/// 菜单 id：**稳定字面量**，事件按它分发。
///
/// 为什么不按文案分发：文案会改、要翻译、会带数字 —— 一改事件就**静默**不再匹配
/// （`AGENTS.md` §11 记过同一类教训：照着一份过期的东西用命令，类型检查看不出来）。
const ID_TOGGLE_WINDOW: &str = "window.toggle";
const ID_TUNNELS: &str = "tunnels";
const ID_TUNNELS_EMPTY: &str = "tunnels.empty";
const ID_QUIT: &str = "app.quit";

/// 隧道项的 id 命名法：`tunnel.<会话 id>`。
///
/// 点击处理等到界面那一侧的工作 —— 没有可聚焦的视图时那条分支永远进不去，
/// 写出来只是死代码。名字先定下来，是为了**接上点击时不必改协议**
/// （改 id 等于改一份别人可能已经依赖的契约）。
fn tunnel_item_id(handle: SessionHandle) -> String {
    format!("tunnel.{handle}")
}

/// 建托盘。返回值 = "托盘可用吗"，调用方据此决定关窗语义（plan 0302）。
///
/// 失败只有一种后果：**没有托盘**。启动、终端、退出都不受影响 —— 一个终端不该因为
/// 系统缺少托盘宿主（很多 Wayland 合成器默认就没有）而打不开。
pub fn setup(app: &AppHandle<Wry>) -> bool {
    let Some(icon) = app.default_window_icon().cloned() else {
        // 图标来自 `tauri.conf.json` 的 `bundle.icon`，构建脚本已经解码进二进制。
        // 走到这里说明打包配置被改坏了 —— 那也不是启动失败的理由（同上）。
        tracing::warn!(reason = "no-default-icon", "tray unavailable");
        return false;
    };

    let menu = match build_menu(app) {
        Ok(menu) => menu,
        Err(err) => {
            tracing::warn!(%err, "tray menu build failed");
            return false;
        }
    };

    let built = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("akasha")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app);

    match built {
        Ok(_) => {
            // 管线接上：会话表一变就重推菜单（隧道列表跟着走）。
            // 排在**建成之后**：建不起来时没人需要刷新。
            let handle = app.clone();
            app.state::<Sessions>().on_change(move || refresh(&handle));
            true
        }
        Err(err) => {
            // 最常见的一种：写不了 `$XDG_RUNTIME_DIR/tray-icon`。
            tracing::warn!(%err, "tray unavailable");
            false
        }
    }
}

/// 会话集合变了 → 重画菜单。
///
/// 菜单项是**快照**：muda 不会自己去读我们的会话表，所以每次变化都得重推一遍。
fn refresh(app: &AppHandle<Wry>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return; // 没有托盘（建不起来），没什么可刷新的
    };
    match build_menu(app) {
        Ok(menu) => {
            if let Err(err) = tray.set_menu(Some(menu)) {
                // 尽力而为：菜单是**显示**，它失败了不影响任何会话的生命周期。
                tracing::debug!(%err, "tray menu refresh failed");
            }
        }
        Err(err) => tracing::warn!(%err, "tray menu build failed"),
    }
}

/// 整份菜单。**每次刷新整份重建**：菜单项与 `SessionId` 的对应关系不能有残留，
/// 而增量改一张 muda 菜单要维护"哪一项对应哪个会话"的第二份账。
fn build_menu(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    let toggle = MenuItemBuilder::with_id(ID_TOGGLE_WINDOW, "显示/隐藏窗口").build(app)?;
    let tunnels = tunnel_submenu(app)?;
    let quit = MenuItemBuilder::with_id(ID_QUIT, "退出 akasha").build(app)?;

    MenuBuilder::new(app)
        .items(&[&toggle, &tunnels, &quit])
        .build()
}

/// 隧道子菜单。**"失败可见"的落点**（`docs/scope.md` §5.2 / §2.2）：重连耗尽之后必须有
/// 个地方让用户看到"它已经死了"，否则用户以为隧道还在转发。
///
/// 数据源是 `Sessions::tunnel_entries()` —— 与 `tunnels` probe **同一份**，不另立一张账。
fn tunnel_submenu(app: &AppHandle<Wry>) -> tauri::Result<tauri::menu::Submenu<Wry>> {
    let list = app.state::<Sessions>().tunnel_entries();

    if list.is_empty() {
        let empty = MenuItemBuilder::with_id(ID_TUNNELS_EMPTY, "（暂无隧道）")
            .enabled(false)
            .build(app)?;
        return SubmenuBuilder::with_id(app, ID_TUNNELS, "隧道")
            .item(&empty)
            .build();
    }

    // 按 `SessionId` 逐条列，文案 = **名称 · 状态**（plan 0601）。状态来自状态机本身，
    // 不是另写一套（`docs/logging.md`：值不撒谎）。
    //
    // 仍然一律**禁用**：点击聚焦要等界面那一侧的工作 —— 一个点了没反应的菜单项
    // 比灰掉的更糟。`tunnel_item_id` 的命名法先留着，接上点击时不必改协议。
    let mut items = Vec::with_capacity(list.len());
    for entry in list {
        let state = match entry.state {
            TunnelState::Reconnecting { attempt } => format!("重连中（第 {attempt} 次）"),
            other => tunnel_state_label(other).to_owned(),
        };
        items.push(
            MenuItemBuilder::with_id(
                tunnel_item_id(entry.handle),
                format!("{} · {state}", entry.name),
            )
            .enabled(false)
            .build(app)?,
        );
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|item| item as _).collect();
    SubmenuBuilder::with_id(app, ID_TUNNELS, "隧道")
        .items(&refs)
        .build()
}

/// 隧道状态的中文文案（托盘是给用户看的，不是日志）。
fn tunnel_state_label(state: TunnelState) -> &'static str {
    match state {
        TunnelState::Connecting => "连接中",
        TunnelState::Connected => "已连接",
        TunnelState::Reconnecting { .. } => "重连中",
        TunnelState::Failed => "失败",
        TunnelState::Stopped => "已停止",
    }
}

fn on_menu_event(app: &AppHandle<Wry>, event: MenuEvent) {
    match event.id().as_ref() {
        ID_TOGGLE_WINDOW => toggle_window(app),
        ID_QUIT => quit(app),
        // `tunnels` / `tunnels.empty` / `tunnel.<id>` 现在没有处理：空列表是禁用的，
        // 隧道项要等阶段 6 才有实体可聚焦（命名法见 `tunnel_item_id`）。
        _ => {}
    }
}

/// 显示/隐藏主窗口。显示时**显式 `set_focus`** —— 否则窗口回来了却还在别的窗口后面，
/// 用户会以为"托盘点了没反应"。
fn toggle_window(app: &AppHandle<Wry>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        tracing::warn!(window = MAIN_WINDOW, "window not found");
        return;
    };
    match window.is_visible() {
        Ok(true) => {
            if let Err(err) = window.hide() {
                tracing::warn!(%err, "window hide failed");
            }
        }
        Ok(false) => {
            if let Err(err) = window.show() {
                tracing::warn!(%err, "window show failed");
            }
            if let Err(err) = window.set_focus() {
                tracing::warn!(%err, "window focus failed");
            }
        }
        Err(err) => tracing::warn!(%err, "window state query failed"),
    }
}

/// 退出：**走 plan 0204 的收尾入口**（`shutdown_all` → 让事件循环自己退出），
/// 不另写一条退出路径 —— 少收一次就是残留（`AGENTS.md` §3.3）。
///
/// 为什么不是 `std::process::exit`：它会跳过 tauri 的收尾与 `RunEvent::Exit`，
/// 于是"退出时回收"的第二个调用点再也不会被跑到（那条路径正是 0204 的验收对象）。
/// 这里多收一次是安全的：`shutdown_all` 幂等（第二次清单已空）。
fn quit(app: &AppHandle<Wry>) {
    crate::log_reclaim(&app.state::<Sessions>().shutdown_all(), "tray");
    app.exit(0);
}
