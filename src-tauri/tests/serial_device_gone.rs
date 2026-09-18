//! plan 1103 的**端到端**验收：设备消失时串口会话以可读原因结束。
//!
//! 判据（ROADMAP 原文）=「把测试用的 PTY 主端关闭（等价于拔掉设备）时会话结束、没有残留注册」，
//! 条目名另加一句"以可读原因结束"。逐条落点：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 会话结束 | 标签页数回到打开前 —— 会话自己走 → 标签页跟着走（plan 0306 那条路） |
//! | 原因可读 | 应用级通知行 `[data-session-notice]` 里有"串口设备已断开"**与设备路径** |
//! | 没有残留注册 | `app_state { probe: "sessions" }` 回到打开前（串口没有本地进程可看） |
//!
//! ## 设备从哪来
//!
//! [`support::FakeSerialDevice::unpluggable`]：一对 PTY，从端的路径当设备，**只持有唯一一份主端**
//! —— 拔掉它就是关掉那份主端，从端那一路的读写当场失败。另两台（`serial_session` /
//! `serial_ports_ui`）用的是会搬字节的那台，它拔不掉（读线程持有一份主端副本），理由写在那
//! 个构造函数的文档里。
//!
//! ## 为什么"拔掉"只有这一种造法
//!
//! 本机 `/dev` 下没有串口设备（问题 #150），而"设备消失"在上游只有一种形态：从端那一路的
//! `read` 报错（实测 `BrokenPipe`；**不是** `Ok(0)` —— 后者会被读端当成"设备安静"继续等，
//! 那条会话就永远不结束）。关掉主端是造出这个形态的唯一手段。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, FakeSerialDevice, click, connect_and_prepare, fill_input, open_serial_panel,
    text, text_of, wait_connected, wait_js, wait_text_contains,
};
use victauri_test::VictauriClient;

/// 那句原因里的一部分（`serial` 的 `SerialError::DeviceGone` 的文案）。
const GONE: &str = "串口设备已断开";

/// `sessions` probe —— "没有残留注册"这条断言的读数口（串口没有本地进程）。
async fn sessions_probe(client: &mut VictauriClient) -> Value {
    client
        .call_tool("app_state", json!({ "probe": "sessions" }))
        .await
        .expect("读不到 sessions probe —— 它注册进 lib.rs 了吗？")
}

/// 等 `sessions` probe 回到某个读数。
async fn wait_sessions(client: &mut VictauriClient, want: &Value) -> Value {
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        let now = sessions_probe(client).await;
        if &now == want {
            return now;
        }
        assert!(
            Instant::now() < deadline,
            "sessions probe 没回到 {want}（现在是 {now}）"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 当前标签页数。
async fn tab_count(client: &mut VictauriClient) -> u64 {
    text(
        &client
            .eval_js("document.querySelectorAll('.tab').length.toString()")
            .await
            .unwrap(),
    )
    .parse()
    .expect("标签页数不是数字")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unplugging_the_device_ends_the_session_with_a_readable_reason() {
    if support::skip_unless_e2e() {
        return;
    }
    if let Some(reason) = support::fake_serial_skip_reason() {
        eprintln!("跳过 serial_device_gone：{reason}");
        return;
    }

    let mut device = FakeSerialDevice::unpluggable();
    let Some((mut client, _fixture, _path)) = connect_and_prepare().await else {
        return;
    };

    let tabs_before = tab_count(&mut client).await;
    let sessions_before = sessions_probe(&mut client).await;
    // 正反例成对：先确认这句话此刻**不在**界面上 —— 否则"拔掉之后它出现了"会被上一次运行
    // 留在通知行里的那一句顶过去（那是一条与本次改动无关的假通过）。
    let before = text_of(&mut client, "[data-session-notice]").await;
    assert!(
        !before.contains(GONE),
        "这条用例开始之前，界面上就挂着上一句设备断开：{before:?}"
    );

    // ── 1. 开一条串口会话（手输路径，与 plan 1102 同一条路）──────────────────
    open_serial_panel(&mut client).await;
    fill_input(
        &mut client,
        ".serial-input[data-serial-field=\"port\"]",
        device.path(),
    )
    .await;
    click(&mut client, "[data-serial-open]", "打开这个串口会话").await;
    wait_connected(&mut client, (tabs_before + 1) as usize, "串口会话").await;
    eprintln!("会话开起来了：{}", device.path());

    // ── 2. 拔掉设备（关掉唯一一份主端）──────────────────────────────────────
    device.unplug();
    eprintln!("已拔掉设备：关掉唯一一份主端，从端那一路的 read 当场报错");

    // ── 3. 会话自己结束：标签页跟着关，原因留在通知行上 ──────────────────────
    wait_js(
        &mut client,
        &format!("document.querySelectorAll('.tab').length === {tabs_before}"),
        CLOSE_TIMEOUT.as_millis() as u64,
        "拔掉设备之后标签页自己关掉",
    )
    .await;
    wait_text_contains(&mut client, "[data-session-notice]", GONE).await;
    wait_text_contains(&mut client, "[data-session-notice]", device.path()).await;
    let said = text_of(&mut client, "[data-session-notice]").await;
    eprintln!("通知行：{said:?}");

    // ── 4. 没有残留注册 ─────────────────────────────────────────────────────
    let back = wait_sessions(&mut client, &sessions_before).await;
    eprintln!("拔掉之后 sessions probe = {back}（打开前 {sessions_before}）");

    let _ = client.invoke_command("vault_lock", None).await;
}
