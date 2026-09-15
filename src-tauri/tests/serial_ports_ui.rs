//! plan 1102 的**端到端**验收：端口枚举与参数接进界面。
//!
//! 判据（ROADMAP 原文）=「界面上看到本机枚举结果并据此（或手输路径）打开；取值越界时显示
//! 字段与取值」。逐条落点：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 界面看到本机枚举结果 | 面板上那个计数 == `invoke_command("serial_ports")` 的条数（**不写死 32**：CI 上没有这些设备） |
//! | 据此打开 | 列表非空时点第一条 → "设备路径"那一栏变成它（列表为空则**显式跳过并写明原因**） |
//! | 手输路径打开 | 手输 PTY 从端的路径 → 「打开」→ 标签页说已连接、双向字节都到（**不依赖列表里有能用的条目**） |
//! | 取值越界显示字段与取值 | 数据位填 9 → 「打开」→ 那个面的报错行里有 `data_bits = 9`；关掉它之后
//!   `sessions` probe 回到打开前（**没有登记**：参数在碰会话表之前就被拒了） |
//! | 填不出来的值 | 数据位填 `abc` → **不开面**，面板自己说清是哪一栏（那一档是 IPC 边界的限制） |
//!
//! ## 两条读取是彼此独立的
//!
//! 库**故意不解锁**：端口那一块照常列出结果，而配置池那一块说"先解锁" —— 这正是"三条输入
//! 并列、谁都不挡谁"（问题 #150 的处置：列表不等于"可用端口"，更不该由它决定别的输入）。
//!
//! ## 设备从哪来
//!
//! 与 `serial_session` 同一份脚手架（[`support::FakeSerialDevice`]）：本机 `/dev` 下没有串口
//! 设备（问题 #150），所以自己造一对 PTY、把从端的路径**手输**进界面。列表里那 32 条
//! `/dev/ttyS*` 一条都打不开（udev 报了 devnode、`/dev` 下却没有），所以判据的收口是手输。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, FakeSerialDevice, click, connect_and_prepare, fill_input, open_serial_panel,
    text, type_line, wait_connected, wait_js, wait_text_contains,
};
use victauri_test::VictauriClient;

/// 设备 → 界面那一串。**与 `serial_session` 的那两串不同**：一眼能从日志里认出是谁发的。
const FROM_DEVICE: &str = "from-device-1102";
/// 界面 → 设备那一串。
const TO_DEVICE: &str = "to-device-1102";

/// `sessions` probe —— "没有残留注册"这条断言的读数口（串口没有本地进程可看）。
async fn sessions_probe(client: &mut VictauriClient) -> Value {
    client
        .call_tool("app_state", json!({ "probe": "sessions" }))
        .await
        .expect("读不到 sessions probe —— 它注册进 lib.rs 了吗？")
}

/// 等 `sessions` probe 回到某个读数（两处都要等它，所以收成一条）。
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

/// 表单里某一栏此刻的取值（读的是**界面上的那一栏**）。
async fn field(client: &mut VictauriClient, name: &str) -> String {
    text(
        &client
            .eval_js(&format!(
                "document.querySelector('.serial-input[data-serial-field=\"{name}\"]')?.value ?? ''"
            ))
            .await
            .unwrap(),
    )
}

/// 当前活动标签页的标题。
async fn active_title(client: &mut VictauriClient) -> String {
    text(
        &client
            .eval_js("document.querySelector('.tab.is-active .tab-label')?.textContent ?? ''")
            .await
            .unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_panel_lists_the_ports_and_opens_what_the_form_says() {
    if support::skip_unless_e2e() {
        return;
    }

    let mut device = FakeSerialDevice::new();
    let Some((mut client, _fixture, _path)) = connect_and_prepare().await else {
        return;
    };

    // ── 1. 后端枚举一次：判据"界面看到的是本机枚举结果"的**另一半**（用来对账）────
    let listed = client
        .invoke_command("serial_ports", None)
        .await
        .expect("serial_ports 调不通 —— 它登记进 bindings.rs 了吗？");
    let ports = listed.as_array().cloned().unwrap_or_default();
    let expected = ports.len();
    eprintln!("枚举：后端 {expected} 条：{listed}");

    let tabs_before = tab_count(&mut client).await;
    let sessions_before = sessions_probe(&mut client).await;

    // ── 2. 界面上那个计数与后端一致；池那一块读不出来（库锁着）不影响它 ──────────
    open_serial_panel(&mut client).await;
    wait_js(
        &mut client,
        &format!(
            "document.querySelector('.serial-ports')?.getAttribute('data-serial-ports') === '{expected}'"
        ),
        10_000,
        "面板上的端口数与后端枚举一致",
    )
    .await;
    wait_text_contains(&mut client, "[data-pool-problem]", "先解锁").await;
    eprintln!("界面：端口列表 {expected} 条（与后端一致）；池那一块显示先解锁，端口照常");

    // ── 3. 点一条枚举结果 → 它被填进"设备路径"那一栏 ─────────────────────────
    if expected > 0 {
        let first = ports[0]
            .pointer("/path")
            .and_then(Value::as_str)
            .expect("SerialPort 里必须有 path");
        click(&mut client, ".serial-port-item", "点第一条枚举结果").await;
        assert_eq!(
            field(&mut client, "port").await,
            first,
            "点一条端口该把它的路径填进那一栏"
        );
        eprintln!("据此填路径：{first}");
    } else {
        // 显式跳过并写明原因，而不是悄悄少验一条（CI 上的 runner 就是这一类机器）。
        eprintln!("跳过「点一条枚举结果」：本机枚举为空 —— 这一类机器上这是正常结果");
    }

    // ── 4. 手输设备的路径 → 打开 → 双向字节（判据"或手输路径"的收口）──────────
    fill_input(
        &mut client,
        ".serial-input[data-serial-field=\"port\"]",
        device.path(),
    )
    .await;
    click(&mut client, "[data-serial-open]", "打开这个串口会话").await;
    wait_connected(
        &mut client,
        (tabs_before + 1) as usize,
        "手输路径的串口会话",
    )
    .await;
    assert_eq!(
        active_title(&mut client).await,
        device.path(),
        "手输路径开的会话，标题就该是那条设备路径"
    );

    device.send(&format!("{FROM_DEVICE}\n"));
    wait_js(
        &mut client,
        &format!("window.__akashaTerminal.screenText(400).includes('{FROM_DEVICE}')"),
        30_000,
        "设备发来的字节出现在了终端上",
    )
    .await;
    type_line(&mut client, &format!("{TO_DEVICE}\n")).await;
    let got = device.wait_received(TO_DEVICE).await;
    eprintln!("手输路径：{FROM_DEVICE} 到了界面、{got:?} 回到了设备");

    click(&mut client, ".tab.is-active .tab-close", "关闭串口标签页").await;
    wait_js(
        &mut client,
        &format!("document.querySelectorAll('.tab').length === {tabs_before}"),
        CLOSE_TIMEOUT.as_millis() as u64,
        "串口标签页关掉了",
    )
    .await;
    wait_sessions(&mut client, &sessions_before).await;

    // ── 5. 取值越界：界面上当场看到**字段与取值** ───────────────────────────
    open_serial_panel(&mut client).await;
    fill_input(
        &mut client,
        ".serial-input[data-serial-field=\"port\"]",
        device.path(),
    )
    .await;
    fill_input(
        &mut client,
        ".serial-input[data-serial-field=\"dataBits\"]",
        "9",
    )
    .await;
    click(&mut client, "[data-serial-open]", "用 9 个数据位打开").await;
    wait_js(
        &mut client,
        &format!(
            "document.querySelectorAll('.tab').length === {}",
            tabs_before + 1
        ),
        10_000,
        "越界取值也开了一个面（会话是那个面自己发起的，报错由它显示）",
    )
    .await;
    wait_text_contains(
        &mut client,
        ".tab-pane.is-active .terminal-error",
        "data_bits = 9",
    )
    .await;
    eprintln!("越界取值：那个面的报错行里写着 data_bits = 9");
    click(
        &mut client,
        ".tab.is-active .tab-close",
        "关掉那个出错态标签页",
    )
    .await;
    let back = wait_sessions(&mut client, &sessions_before).await;
    eprintln!("越界取值：关掉之后 sessions probe = {back}（打开前 {sessions_before}）");

    // ── 6. 填不出来的值：**不开面**（这一档由面板自己拦，它不是取值域）─────────
    open_serial_panel(&mut client).await;
    fill_input(
        &mut client,
        ".serial-input[data-serial-field=\"dataBits\"]",
        "abc",
    )
    .await;
    click(&mut client, "[data-serial-open]", "用 abc 个数据位打开").await;
    wait_text_contains(&mut client, "[data-picker-problem]", "abc").await;
    assert_eq!(
        tab_count(&mut client).await,
        tabs_before,
        "寄不出去的参数不该开出一个面"
    );
    eprintln!("填不出来的值：面板自己拦下了（没有开面）");
    click(&mut client, ".serial-picker-close", "关掉串口面板").await;

    // ── 7. 收尾：锁上（本来就没解锁）并删掉自己造的库（`_fixture` 的析构负责删）──
    let _ = client.invoke_command("vault_lock", None).await;
}
