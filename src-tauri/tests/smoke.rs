//! Victauri smoke tests — validates your Tauri app through the MCP bridge.
//!
//! Requires a running Tauri dev server.
//! Run with: VICTAURI_E2E=1 cargo test --test smoke

// 豁免 workspace 的 `clippy::unwrap_used`（根 Cargo.toml）：本文件是**测试**，
// unwrap 在这里就是断言手段，不属于 AGENTS.md §0「command 边界或长驻任务」。
// 生产代码没有这条豁免。⚠️ 本文件由 victauri-test 生成，重生成后需重新补上这一行。
#![allow(clippy::unwrap_used)]

use victauri_test::VictauriClient;

fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("Skipping: set VICTAURI_E2E=1 with your Tauri dev server running");
        return true;
    }
    false
}

#[tokio::test]
async fn connect_and_check_plugin_info() {
    if skip_unless_e2e() {
        return;
    }

    let mut client = VictauriClient::discover()
        .await
        .expect("Failed to connect — is your Tauri dev server running?");

    let info = client.get_plugin_info().await.unwrap();
    assert!(
        info.get("version").is_some(),
        "plugin_info should have version"
    );
    eprintln!("Connected to Victauri v{}", info["version"]);
}

#[tokio::test]
async fn screenshot_captures_window() {
    if skip_unless_e2e() {
        return;
    }

    let mut client = VictauriClient::discover().await.unwrap();
    let result = client.screenshot().await.unwrap();

    let has_image = result.get("image").is_some()
        || result.get("data").is_some()
        || result.get("base64").is_some()
        || result.pointer("/result/content/0/data").is_some();
    assert!(has_image, "screenshot should return image data");
    eprintln!("Screenshot captured successfully");
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
            "  [{}] {}",
            if result.passed { "PASS" } else { "FAIL" },
            result.description,
        );
    }
    assert!(
        report.all_passed(),
        "IPC integrity checks should pass: {:?}",
        report
            .failures()
            .iter()
            .map(|f| &f.description)
            .collect::<Vec<_>>()
    );
}
