//! Victauri smoke tests — validates your Tauri app through the MCP bridge.
//!
//! 入口是 `just test-e2e`：它会**自己起**一套 app（已有 `just dev` 在跑就直接复用），
//! 跑完再收掉。单独跑也可以：`VICTAURI_E2E=1 cargo test --test smoke -- --test-threads=1`。
//!
//! 平台能力的差异在这里**显式说明**（例如 Wayland 下拿不到原生窗口句柄），
//! 覆盖面交给 CI 矩阵（`AGENTS.md` §12）—— 不做"静默跳过"。

// 豁免 workspace 的 `clippy::unwrap_used`（根 Cargo.toml）：本文件是**测试**，
// unwrap 在这里就是断言手段，不属于 AGENTS.md §0「command 边界或长驻任务」。
// 生产代码没有这条豁免。⚠️ 本文件由 victauri-test 生成，重生成后需重新补上这一行。
#![allow(clippy::unwrap_used)]

use victauri_test::VictauriClient;

/// 本项目的 bundle identifier。连上先确认"这是 akasha"——
/// 每个 Victauri app 都注册在同一个默认端口上，连到别的实例会让后面所有结论都失去意义
/// （`AGENTS.md` §7「Victauri 使用纪律」第一条）。
const APP_IDENTIFIER: &str = "fans.cyrene.akasha-terminal";

fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("跳过: 未设置 VICTAURI_E2E=1（该变量由 just test-e2e 设置）");
        return true;
    }
    false
}

/// 当前环境是不是**拿不到原生窗口句柄**。
///
/// 窗口截图依赖平台原生句柄，而 Victauri 只实现了 Win32 / AppKit / Xlib / Xcb
/// （victauri-plugin 的 `get_native_handle`）。Wayland 会话里 webview 是 Wayland
/// surface，落进最后那支 `_ =>`，于是 `screenshot` 报
/// `unsupported window handle type on this platform`。
///
/// 这是**环境能力**，不是回归：跳过并写明原因（和 `terminal_render` 里
/// "驱动没给 `WEBGL_lose_context`" 时的处理一致），**不把它说成绿**。
/// 覆盖这条路径的是 CI 矩阵：Ubuntu 侧跑在 xvfb（X11）上，另有 Windows / macOS。
fn native_capture_skip_reason() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        // `GDK_BACKEND=x11` 会把 GTK 按到 XWayland 上，那时原生句柄又是 Xlib 了。
        let forced_x11 = std::env::var("GDK_BACKEND").is_ok_and(|backend| backend == "x11");
        if wayland && !forced_x11 {
            return Some(
                "Wayland 会话：原生句柄是 Wayland surface，Victauri 只认 Xlib/Xcb/Win32/AppKit",
            );
        }
    }
    None
}

#[tokio::test]
async fn connect_and_check_plugin_info() {
    if skip_unless_e2e() {
        return;
    }

    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— 用 `just test-e2e`（它会自己起 app）");

    let info = client.get_plugin_info().await.unwrap();
    assert!(
        info.get("version").is_some(),
        "plugin_info should have version"
    );
    assert_eq!(
        info.pointer("/app/identifier")
            .and_then(serde_json::Value::as_str),
        Some(APP_IDENTIFIER),
        "端口上连到的不是 akasha —— 后面的结论都不可信：{info}"
    );
    eprintln!(
        "服务端: 标识={APP_IDENTIFIER} 版本={}",
        info["version"].as_str().unwrap_or("?")
    );
}

#[tokio::test]
async fn screenshot_captures_window() {
    if skip_unless_e2e() {
        return;
    }
    if let Some(reason) = native_capture_skip_reason() {
        eprintln!("跳过: {reason}");
        return;
    }

    let mut client = VictauriClient::discover().await.unwrap();
    let result = client.screenshot().await.unwrap();

    let has_image = result.get("image").is_some()
        || result.get("data").is_some()
        || result.get("base64").is_some()
        || result.pointer("/result/content/0/data").is_some();
    assert!(has_image, "screenshot should return image data: {result}");
}

#[tokio::test]
async fn ipc_integrity_passes() {
    if skip_unless_e2e() {
        return;
    }

    let mut client = VictauriClient::discover().await.unwrap();
    let report = client
        .verify()
        .ipc_healthy()
        .no_console_errors()
        .run()
        .await
        .unwrap();

    for result in &report.results {
        eprintln!(
            "报告: 结果={} 检查={}",
            if result.passed { "PASS" } else { "FAIL" },
            result.description,
        );
    }
    // 失败时把 `detail` 一起打出来：只报「哪条检查失败」定位不到东西 ——
    // 这条用例第一次红就是因为探针在频道结束帧上抛了 TypeError，而当时报告里
    // 只有一句 `no console errors`（问题 #39）。
    assert!(
        report.all_passed(),
        "IPC integrity checks should pass:\n{}",
        report
            .failures()
            .iter()
            .map(|f| format!("  - {}: {}", f.description, f.detail))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
