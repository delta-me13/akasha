//! plan 0701 的**端到端**验收：**不打开任何终端**，直接开 SFTP，两侧各自列目录。
//!
//! 判据（ROADMAP 原文）=「直接打开 SFTP 即可用，无终端依赖」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 不需要先开终端 | SFTP 打开前后**终端标签页数不变**（app 自带的那一个不算前置） |
//! | 两侧独立选来源（plan 0702 起这一栏也能选「本机」） | 池里两台主机分别指向**两台**进程内服务端（目录内容不同） |
//! | **两侧各自列目录成功** | 左栏列出 A 的条目、右栏列出 B 的条目；且左栏**没有** B 的条目 |
//! | 两侧互不影响 | 只连左侧时，右侧仍是 `disconnected` |
//! | 连接真的建立了 | 两台服务端各自记到 **1 次** `sftp` 子系统请求 |
//! | 会话归属 | `sessions` 的 `live`/`registered` = 本地终端 + 这个 SFTP 会话 |
//! | 关闭是显式的 | 点「结束会话」→ 探针清空、注册表回到打开前、两台服务端都看到连接断开 |
//!
//! ## 服务端在**测试进程**里
//!
//! 同 `ssh_session` / `tunnel_state`：服务端在测试进程，app 连过来。两台的 SFTP 根目录
//! **内容不同** —— 否则"两侧都列出了同一个列表"也能通过，而那是这条判据最该分得开的情形。
//!
//! ## 数据从哪来
//!
//! 主机池的增删改**没有界面**（仍未规划），所以两行主机由这条用例直接写库 ——
//! 与 plan 0505 构造跳板链、plan 0601 构造转发规则是同一种做法。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use akasha_lib::ssh::testing::{ServerOptions, SftpItem, start};
use akasha_lib::store::pools::hosts;
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, USER, click, connect_and_prepare, forget, observed, open_vault, text, text_of,
    unlock, wait_js,
};
use victauri_test::VictauriClient;

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-sftp-login-password";
/// 池里左侧那台主机的名字。
const LEFT_NAME: &str = "e2e-sftp-left";
/// 池里右侧那台主机的名字。
const RIGHT_NAME: &str = "e2e-sftp-right";

/// 在库里把这次要用的两行摆好：两台主机，各指向一台服务端。
///
/// 返回 `(左侧的主机 id, 右侧的主机 id)`。名字决定 `vault_hosts` 的顺序
/// （它按名字排序），所以 `left` 在前 —— 面板两栏各选一台，顺序不影响判据。
fn seed(path: &Path, left_port: u16, right_port: u16) -> (u32, u32) {
    let conn = open_vault(path);
    forget(&conn, LEFT_NAME, "127.0.0.1", left_port);
    forget(&conn, RIGHT_NAME, "127.0.0.1", right_port);

    let left = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: LEFT_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port: left_port,
            user: USER.to_owned(),
            // 口令认证：这条用例不依赖 agent，也不依赖任何密钥材料。
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();

    let right = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: RIGHT_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port: right_port,
            user: USER.to_owned(),
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();

    (u32::try_from(left).unwrap(), u32::try_from(right).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_lists_both_panes_without_any_terminal() {
    if support::skip_unless_e2e() {
        return;
    }

    // ── 1. 两台服务端（在**本进程**里）：SFTP 根目录的内容**不同** ───────────
    let mut left_options = ServerOptions::password(PASSWORD);
    left_options.sftp = Some(vec![
        SftpItem::file("left-alpha.txt"),
        SftpItem::dir("left-dir"),
    ]);
    let left = start(left_options).await;

    let mut right_options = ServerOptions::password(PASSWORD);
    right_options.sftp = Some(vec![SftpItem::file("right-beta.txt")]);
    let right = start(right_options).await;
    eprintln!(
        "构造: 左服务端=127.0.0.1:{} 右服务端=127.0.0.1:{}",
        left.addr.port(),
        right.addr.port()
    );

    // ── 2. 库在哪、必要时先锁上 ──────────────────────────────────────────────
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };

    // ── 3. 种子数据 + 解锁 ─────────────────────────────────────────────────
    let (left_id, right_id) = seed(&path, left.addr.port(), right.addr.port());
    unlock(&mut client).await;

    // 判据的前半：SFTP 的**前置里没有终端** —— 所以它出现前后，终端标签页数与
    // 注册表里已有的会话数都不该变（除了它自己那一个）。
    //
    // ⚠️ 基准必须**现读**，不能写死 1：各 E2E 目标**共用一个 app**，前面的目标
    //（`tab_close` 那类）会把标签页关到零 —— 写死数字会让这条用例红在与本次改动
    // 无关的原因上（实测：第一次跑时这里就是 0）。
    let tabs_before = tab_count(&mut client).await;
    let sessions_before = sessions_probe(&mut client).await;
    let live_before = number(&sessions_before, "/live");
    assert_eq!(
        live_before,
        number(&sessions_before, "/registered"),
        "两张表必须相等（读过基准之后的断言都以它为基准）：{sessions_before}"
    );
    eprintln!("界面: 标签页={tabs_before} 会话={live_before}");

    // ── 4. 界面：打开 SFTP 面板，新建一个会话 ───────────────────────────────
    open_sftp_panel(&mut client).await;
    start_session(&mut client).await;

    // ── 5. 两侧各自选来源（左右指向**不同**的服务端）─────────────────────────
    choose_host(&mut client, "left", left_id).await;
    choose_host(&mut client, "right", right_id).await;

    // ── 6. 只连左侧：右侧必须仍是未连接（"两侧独立"）────────────────────────
    let asked = connect_side(&mut client, "left", &left.fingerprint).await;
    eprintln!("会话: 侧=左 提示数={}", asked.len());
    assert_eq!(
        sftp_state(&mut client, "right").await,
        "disconnected",
        "只连了左侧，右侧不该被顺手连上（两侧各是一条独立连接）"
    );

    // ── 7. 连右侧 ──────────────────────────────────────────────────────────
    let asked = connect_side(&mut client, "right", &right.fingerprint).await;
    eprintln!("会话: 侧=右 提示数={}", asked.len());

    // ── 8. 判据：两侧各自列目录成功，且**列的是各自那一台** ──────────────────
    wait_js(
        &mut client,
        "!!document.querySelector('.sftp-pane[data-side=\"left\"] \
         .sftp-entry[data-entry-name=\"left-alpha.txt\"]')",
        10_000,
        "左栏列出了左侧服务端的条目",
    )
    .await;
    wait_js(
        &mut client,
        "!!document.querySelector('.sftp-pane[data-side=\"right\"] \
         .sftp-entry[data-entry-name=\"right-beta.txt\"]')",
        10_000,
        "右栏列出了右侧服务端的条目",
    )
    .await;
    assert!(
        !js_bool(
            &mut client,
            "!!document.querySelector('.sftp-pane[data-side=\"left\"] \
             .sftp-entry[data-entry-name=\"right-beta.txt\"]')"
        )
        .await,
        "左栏列出了**右侧**服务端的条目 —— 两栏串了，而这条判据正是要分开它们"
    );

    // 探针：一个会话、两侧都 connected、路径是服务端 `realpath` 给出的结果。
    let entries = sftp_entries(&mut client).await;
    assert_eq!(entries.len(), 1, "只应有一个 SFTP 会话：{entries:?}");
    let session = &entries[0];
    let sides = session
        .pointer("/sides")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(sides.len(), 2, "一个 SFTP 会话永远有两侧：{session}");
    for side in ["left", "right"] {
        let found = sides
            .iter()
            .find(|info| info.pointer("/side").and_then(Value::as_str) == Some(side))
            .unwrap_or_else(|| panic!("探针里缺 {side} 这一侧：{session}"));
        assert_eq!(
            found.pointer("/state").and_then(Value::as_str),
            Some("connected"),
            "{side} 这一侧应当是 connected：{found}"
        );
        assert_eq!(
            found.pointer("/path").and_then(Value::as_str),
            Some("/"),
            "{side} 这一侧的当前目录应当是服务端 `realpath` 给出的那个：{found}"
        );
    }

    // 服务端那一侧：**各**记到 1 次 sftp 子系统请求（"列目录成功"不能只看我们自己说）。
    assert_eq!(
        observed(&left).sftp_subsystems,
        1,
        "左侧服务端应当被请求过一次 sftp 子系统"
    );
    assert_eq!(
        observed(&right).sftp_subsystems,
        1,
        "右侧服务端应当被请求过一次 sftp 子系统"
    );

    // ── 9. 判据的前半：全程没有多出终端标签页 ────────────────────────────────
    assert_eq!(
        tab_count(&mut client).await,
        tabs_before,
        "SFTP 不该依赖终端 —— 它的出现不该带来任何终端标签页"
    );

    // 注册表：已有的会话 + 这个 SFTP 会话，且两个数必须相等。
    let sessions = sessions_probe(&mut client).await;
    assert_eq!(
        (number(&sessions, "/live"), number(&sessions, "/registered")),
        (live_before + 1, live_before + 1),
        "开一个 SFTP 会话只该多出**一个**会话，且两张表必须相等：{sessions}"
    );

    // ── 10. 结束会话：两侧断开、注册表回退 ──────────────────────────────────
    click(&mut client, ".sftp-stop", "结束 SFTP 会话").await;
    wait_js(
        &mut client,
        "!document.querySelector('.sftp-pane[data-side=\"left\"]')",
        10_000,
        "会话结束后两栏消失",
    )
    .await;
    assert!(
        sftp_entries(&mut client).await.is_empty(),
        "关掉之后探针该是空的"
    );
    let sessions = sessions_probe(&mut client).await;
    assert_eq!(
        (number(&sessions, "/live"), number(&sessions, "/registered")),
        (live_before, live_before),
        "关掉之后该回到打开之前的水平，且两张表必须相等：{sessions}"
    );

    // 对端：两台服务端**各自**看到那条连接断了 —— 这是"连接真的没了"的外部证据
    //（断开是在 runtime 上排队的，所以这里等它落地）。
    for (label, server) in [("左", &left), ("右", &right)] {
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        loop {
            let closed = observed(server).connections_closed;
            if closed >= 1 {
                eprintln!("服务端: 侧={label} 断开连接数={closed}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{label}侧会话关掉了，服务端却还看到连接挂着（只有 {closed} 条断开）"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    let _ = client.invoke_command("vault_lock", None).await;
}

// ── 驱动界面与读后端的辅助 ───────────────────────────────────────────────────

/// 打开 SFTP 面板（并保证它重新读一遍池子与已有会话）。
///
/// 同 `support::open_tunnel_panel` 的两条理由：入口是**切换**，而各 E2E 目标**共用一个 app**
/// （上一个目标可能已经把面板留在打开状态）；面板的主机表是**挂载时读一次**的。
async fn open_sftp_panel(client: &mut VictauriClient) {
    if !text_of(client, ".sftp-panel").await.is_empty() {
        click(
            client,
            ".sftp-close",
            "关闭 SFTP 面板（好让它重新读一次池子）",
        )
        .await;
    }
    click(client, ".tab-new-sftp", "打开 SFTP 面板").await;
    wait_js(
        client,
        "!!document.querySelector('.sftp-panel')",
        10_000,
        "SFTP 面板打开了",
    )
    .await;
}

/// 保证面板上是一个**新建的**会话。
///
/// 后端可能还留着上一个目标开过的会话（面板会接回去）—— 本用例要自己开一个。
async fn start_session(client: &mut VictauriClient) {
    if !text_of(client, ".sftp-stop").await.is_empty() {
        click(client, ".sftp-stop", "结束上一个目标留下的 SFTP 会话").await;
        wait_js(
            client,
            "!!document.querySelector('.sftp-start')",
            10_000,
            "回到新建状态",
        )
        .await;
    }
    click(client, ".sftp-start", "新建 SFTP 会话").await;
    wait_js(
        client,
        "!!document.querySelector('.sftp-pane[data-side=\"left\"]')",
        10_000,
        "两栏出现了",
    )
    .await;
}

/// 在某一栏里选中池里的那一行（plan 0702 起这一栏选的是**来源**：本机或一台主机）。
///
/// 为什么要绕过 React 的 value tracker（用原型上的 setter 再手动派发 `change`）：
/// 直接写 `select.value = …` 时 React 记着上一次的值，`change` 会被它当成"没变"。
async fn choose_host(client: &mut VictauriClient, side: &str, host_id: u32) {
    let value = format!("host:{host_id}");
    let script = format!(
        "(() => {{
            const select = document.querySelector('.sftp-pane[data-side=\"{side}\"] .sftp-origin');
            if (!select) return 'no-select';
            const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
            setter.call(select, '{value}');
            select.dispatchEvent(new Event('change', {{ bubbles: true }}));
            return select.value;
        }})()"
    );
    let chosen = text(&client.eval_js(&script).await.unwrap());
    assert_eq!(
        chosen, value,
        "{side} 那一栏的主机没选上（得到 {chosen:?}）"
    );
}

/// 连某一侧：点「连接」，按类型回答提示，直到那一侧变成 `connected`。
///
/// 为什么不写死"先密钥后口令"两步：链上每一跳各来一轮（plan 0505 的教训），
/// 写死步数会在拓扑一变时**静默少答一轮**，表现是"连不上"而不是"用例写错了"。
async fn connect_side(client: &mut VictauriClient, side: &str, fingerprint: &str) -> Vec<String> {
    let mut asked = Vec::new();
    click(
        client,
        &format!(".sftp-pane[data-side=\"{side}\"] .sftp-connect"),
        "连接这一侧",
    )
    .await;

    let deadline = Instant::now() + Duration::from_secs(90);
    let any_prompt = "!!document.querySelector('.ssh-prompt[data-prompt-kind=\"hostKey\"]') \
                      || !!document.querySelector('.ssh-prompt[data-prompt-kind=\"credential\"]')";
    loop {
        match sftp_state(client, side).await.as_str() {
            "connected" => return asked,
            "failed" => {
                let failure = text_of(
                    client,
                    &format!(".sftp-pane[data-side=\"{side}\"] .sftp-failure"),
                )
                .await;
                panic!("{side} 这一侧连接失败：{failure}");
            }
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "{side} 这一侧没连上（问到过的：{asked:?}）"
        );

        // 等"弹出任意一种提示"（超时也照常往下走：下一圈先看是不是已经连上了）。
        let _ = client
            .wait_for_expression(any_prompt, None, Some(3_000), None)
            .await;

        let presented = text_of(
            client,
            ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-fingerprint",
        )
        .await;
        if !presented.is_empty() {
            assert_eq!(
                presented, fingerprint,
                "提示里那串指纹不属于 {side} 这一侧的服务端"
            );
            asked.push(format!("hostKey:{presented}"));
            click(
                client,
                ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-accept",
                "接受这把主机密钥",
            )
            .await;
            continue;
        }

        let hint = text_of(
            client,
            ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-hint",
        )
        .await;
        if !hint.is_empty() {
            asked.push(format!("credential:{hint}"));
            support::fill_secret(client, PASSWORD).await;
            click(
                client,
                ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-submit",
                "提交口令",
            )
            .await;
        }
    }
}

/// 某一侧在界面上的状态（`data-sftp-state` 就是后端的短名）。
async fn sftp_state(client: &mut VictauriClient, side: &str) -> String {
    let raw = client
        .eval_js(&format!(
            "document.querySelector('.sftp-pane[data-side=\"{side}\"]')?.dataset.sftpState ?? ''"
        ))
        .await
        .unwrap();
    text(&raw)
}

/// 求一个 JS 布尔表达式的值。
async fn js_bool(client: &mut VictauriClient, expression: &str) -> bool {
    let raw = client
        .eval_js(&format!("({expression}) === true"))
        .await
        .unwrap();
    text(&raw) == "true"
}

/// 当前终端标签页数（判据"不需要先开终端"的读数口）。
async fn tab_count(client: &mut VictauriClient) -> usize {
    let raw = client
        .eval_js("document.querySelectorAll('.tab').length")
        .await
        .unwrap();
    text(&raw).parse().unwrap_or(0)
}

/// `app_state { probe: "sftp" }` 的原始列表。
async fn sftp_entries(client: &mut VictauriClient) -> Vec<Value> {
    let value = client
        .call_tool("app_state", json!({ "probe": "sftp" }))
        .await
        .expect("读不到 sftp probe —— 它注册进 lib.rs 了吗？");
    value.as_array().cloned().unwrap_or_default()
}

/// `app_state { probe: "sessions" }`。
async fn sessions_probe(client: &mut VictauriClient) -> Value {
    client
        .call_tool("app_state", json!({ "probe": "sessions" }))
        .await
        .expect("读不到 sessions probe")
}

/// 从探针结果里取一个数（取不到就是 0 —— 那是"这个字段不在"的意思，不是合法取值）。
fn number(value: &Value, pointer: &str) -> u64 {
    value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}
