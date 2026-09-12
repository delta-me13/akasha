//! Integration tests — auto-generated from discovered #[tauri::command] functions.
//!
//! Run with: VICTAURI_E2E=1 cargo test --test integration

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

// ── Health check ─────────────────────────────────────────────────────────

#[tokio::test]
async fn full_stack_health_check() {
    if skip_unless_e2e() {
        return;
    }
    let mut client = VictauriClient::discover()
        .await
        .expect("Failed to connect — is your Tauri dev server running?");

    let report = client
        .verify()
        .ipc_healthy()
        .no_console_errors()
        .run()
        .await
        .unwrap();

    report.assert_all_passed();
}

// ── Command tests ────────────────────────────────────────────────────────
// Generated from discovered #[tauri::command] functions.
// Adapt each test to match your command's expected arguments and behavior.

#[tokio::test]
async fn command_greet() {
    if skip_unless_e2e() {
        return;
    }
    let mut client = VictauriClient::discover().await.unwrap();

    // ⚠️ 生成器当初写的是 `invoke_command("greet", None)`，而 `greet` 需要 `name` ——
    // 于是这条用例**一直**是红的（它不在 `ready` 里，而 `just test-e2e` 要真 app，
    // 谁都没跑过它）。按本文件开头的 "Adapt each test" 补上参数。
    let result = client
        .invoke_command("greet", Some(serde_json::json!({ "name": "akasha" })))
        .await;
    assert!(
        result.is_ok(),
        "greet should respond without error: {:?}",
        result.err()
    );
}
