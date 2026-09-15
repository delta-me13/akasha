//! plan 1101 的**端到端**验收：真 app 上开一个串口会话。
//!
//! 判据（ROADMAP 原文）=「真实 app 上打开一个串口会话并双向传字节；关闭标签页后
//! `live` / `registered` 归零」。逐条落点：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 从界面打开（**池行取值 → 表单 → 打开**） | 点 `.tab-new-serial` → 点池里那一行（表单的路径栏随之被填上）
//!   → 点「打开」→ 标签页出现且状态"已连接" |
//! | **设备 → 界面** | 往 PTY 主端写一串 → 屏幕文本里出现它 |
//! | **界面 → 设备** | 往标签页里敲一行 → 本进程从主端**读到同一串** |
//! | 关闭标签页即回收 | 标签页数回到打开前；`sessions` probe 的 `live` / `registered` 回到打开前的读数 |
//!
//! ## 设备从哪来
//!
//! 本机 `/dev` 下没有任何串口设备（问题 #150），所以用例自己造一对 PTY，把**从端的路径**
//! 当设备交给 app —— 造它的是 [`support::FakeSerialDevice`]（plan 1102 起与
//! `serial_ports_ui` 共用一份）。两串文本**故意不同**（`from-device-1101` 与
//! `to-device-1101`），于是"主端读到的那一串只可能来自 app"这件事不依赖行规程的行为：
//! ECHO（若还在）只会把主端自己写的那一串回给主端。
//!
//! ## "归零"的口径
//!
//! app 一启动就有那个本地终端标签页，所以这里断言的是"关掉串口标签页之后回到**打开前**
//! 的读数"（与 `ssh_session` 同一条口径），而不是字面的 0。串口**没有本地进程**
//! （`Capabilities::NONE`），所以这条判据只能看注册表 —— 与 ADR-0003 D4 同一条理由。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::path::Path;

use akasha_store::pools::serial;
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, FakeSerialDevice, click, connect_and_prepare, open_serial_panel, open_vault,
    text, type_line, unlock, wait_connected, wait_js,
};
use victauri_test::VictauriClient;

/// 池里那一行的名字（也是标签页标题）。**不是**用户的。
const SERIAL_NAME: &str = "e2e-serial-port";
/// 设备 → 界面那一串。
const FROM_DEVICE: &str = "from-device-1101";
/// 界面 → 设备那一串。两组字节故意不同（见文件头）。
const TO_DEVICE: &str = "to-device-1101";

/// 在库里把这次要开的那一行摆好（重跑不依赖上一次跑干净了）。
fn seed(path: &Path, device: &str) {
    let conn = open_vault(path);
    for row in serial::serials(&conn).unwrap() {
        if row.name == SERIAL_NAME {
            serial::delete_serial(&conn, row.id).unwrap();
        }
    }
    serial::insert_serial(
        &conn,
        &serial::NewSerial {
            name: SERIAL_NAME.to_owned(),
            port: device.to_owned(),
            baud: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: serial::Parity::None,
            flow: serial::Flow::None,
        },
    )
    .unwrap();
}

/// `sessions` probe：判据"关闭之后回到打开前的读数"的**唯一**读数口。
async fn sessions_probe(client: &mut VictauriClient) -> Value {
    client
        .call_tool("app_state", json!({ "probe": "sessions" }))
        .await
        .expect("读不到 sessions probe —— 它注册进 lib.rs 了吗？")
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

/// 表单里某一栏的取值（读的是**界面上的那一栏**，不是我们发出去的参数）。
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_serial_session_flows_bytes_both_ways_and_closes_clean() {
    if support::skip_unless_e2e() {
        return;
    }

    // ── 1. 设备（在**本进程**里）：一对 PTY，从端的路径当串口 ───────────────────
    let mut device = FakeSerialDevice::new();

    // ── 2. 库在哪、现在是什么状态、必要时先锁上 ──────────────────────────────
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };

    // ── 3. 种子数据 + 解锁（**只有"列出池里有哪些"这一步需要解锁**）────────────
    seed(&path, device.path());
    unlock(&mut client).await;

    let listed = client
        .invoke_command("vault_serials", None)
        .await
        .expect("vault_serials 调不通 —— 它登记进 bindings.rs 了吗？");
    let rows = listed.as_array().cloned().unwrap_or_default();
    let ours: Vec<&Value> = rows
        .iter()
        .filter(|row| row.pointer("/name").and_then(Value::as_str) == Some(SERIAL_NAME))
        .collect();
    assert_eq!(ours.len(), 1, "池里应当只有我们种下的那一行：{listed}");
    let serial_id = ours[0]
        .pointer("/id")
        .and_then(Value::as_u64)
        .expect("SerialEntry 里必须有 id");
    assert_eq!(
        ours[0].pointer("/port").and_then(Value::as_str),
        Some(device.path()),
        "列出来的设备路径必须就是这一对 PTY 的从端"
    );
    eprintln!("串口池：{listed}");

    // ── 4. 界面：点"串口" → 点池里那一行（填表单）→ 点「打开」────────────────
    let tabs_before = tab_count(&mut client).await;
    let sessions_before = sessions_probe(&mut client).await;
    open_serial_panel(&mut client).await;
    wait_js(
        &mut client,
        &format!("!!document.querySelector('.serial-picker-item[data-serial-id=\"{serial_id}\"]')"),
        10_000,
        "串口面板列出了池里那一行",
    )
    .await;
    click(
        &mut client,
        &format!(".serial-picker-item[data-serial-id=\"{serial_id}\"]"),
        "用这条串口配置填表单",
    )
    .await;
    // 池行 → 表单：**六个字段都要真的到那一栏**（plan 1102 的"池行取值"）。
    assert_eq!(field(&mut client, "port").await, device.path());
    assert_eq!(field(&mut client, "baud").await, "115200");
    assert_eq!(field(&mut client, "dataBits").await, "8");
    assert_eq!(field(&mut client, "stopBits").await, "1");
    click(&mut client, "[data-serial-open]", "打开这个串口会话").await;
    wait_connected(&mut client, (tabs_before + 1) as usize, "串口会话").await;

    let title = text(
        &client
            .eval_js("document.querySelector('.tab.is-active .tab-label')?.textContent ?? ''")
            .await
            .unwrap(),
    );
    assert_eq!(
        title, SERIAL_NAME,
        "这一行是照池里那条配置填的、一个字没改，标题该是那条配置的名字"
    );
    eprintln!("界面：串口标签页已连接（打开前 {tabs_before} 个标签页）");

    // ── 5. 设备 → 界面 ─────────────────────────────────────────────────────
    device.send(&format!("{FROM_DEVICE}\n"));
    wait_js(
        &mut client,
        &format!("window.__akashaTerminal.screenText(400).includes('{FROM_DEVICE}')"),
        30_000,
        "设备发来的字节出现在了终端上",
    )
    .await;
    eprintln!("设备 → 界面：{FROM_DEVICE}");

    // ── 6. 界面 → 设备（另一组字节，所以主端读到的那一串只可能来自 app）────────
    type_line(&mut client, &format!("{TO_DEVICE}\n")).await;
    let got = device.wait_received(TO_DEVICE).await;
    eprintln!("界面 → 设备：主端读到了 {got:?}");

    // ── 7. 关标签页 = 立刻丢弃这个 Session（只丢它自己）────────────────────────
    click(&mut client, ".tab.is-active .tab-close", "关闭串口标签页").await;
    wait_js(
        &mut client,
        &format!("document.querySelectorAll('.tab').length === {tabs_before}"),
        CLOSE_TIMEOUT.as_millis() as u64,
        "串口标签页关掉了",
    )
    .await;

    let deadline = std::time::Instant::now() + CLOSE_TIMEOUT;
    let sessions = loop {
        let now = sessions_probe(&mut client).await;
        if now == sessions_before {
            break now;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "关掉标签页之后后端还登记着会话：{now}（打开前是 {sessions_before}）"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert_eq!(
        sessions.pointer("/live"),
        sessions.pointer("/registered"),
        "两张表分叉了（有会话可查询、却无人管理）：{sessions}"
    );
    eprintln!("关标签页：sessions probe = {sessions}（打开前 {sessions_before}）");

    // ── 8. 收尾：锁上并删掉自己造的库（`_fixture` 的析构负责删）─────────────
    let _ = client.invoke_command("vault_lock", None).await;
}
