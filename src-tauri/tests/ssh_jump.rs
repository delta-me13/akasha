//! plan 0505 的**端到端**验收：真 app 上经跳板连一台**只有跳板看得见**的主机。
//!
//! 判据（ROADMAP 原文）=「ProxyJump 可连通**只对跳板机可见**的目标」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 目标**只在跳板的网络里** | 池里那一行的 `host` 是 `.invalid` 的名字，而它**在本机解析不出来**（用例自己解析一次） |
//! | 连接真的经过跳板 | 跳板服务端记到了**恰好一条** `direct-tcpip`，host/port 与池里那行一致 |
//! | 提示是**每一跳各一轮** | 用户按类型答完了"跳板的密钥 + 跳板的口令 + 目标的密钥 + 目标的口令" |
//! | 字节到了目标 | 目标服务端收到终端里敲的那行；而它**够不着**客户端给的地址 |
//! | 界面上看得见跳板 | 主机选择器那一行显示"经跳板 …"（`jumpId` 过 IPC 的形状） |
//!
//! ## "只对跳板机可见"在这个用例里怎么成立
//!
//! 无特权环境里做不出真正的网络隔离（换 netns / 加防火墙都要 root），所以这条性质是靠**名字**
//! 造的：客户端拿到的目标是 `akasha-e2e-inner.invalid:22`，它**只在跳板服务端的中继表里**存在
//! （`.invalid` 是 RFC 2606 保留的、永远解析不出来的顶级域）。用例自己解析一次那个名字并断言
//! **失败** —— 于是"字节到了目标"这件事只可能经过跳板。
//!
//! ## 服务端都在**测试进程**里
//!
//! 跳板与目标两台都由本进程起（`akasha_lib::ssh::testing`），所以"跳板看到了什么请求"、"目标收到了
//! 什么字节"、"界面上显示了什么"能在一条用例里对账。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::net::ToSocketAddrs;
use std::time::Duration;

use akasha_lib::ssh::testing::{Relay, Running, ServerOptions, start};
use akasha_lib::store::pools::hosts;
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, PromptScript, USER, answer_prompts, click, connect_and_prepare, forget,
    observed, open_vault, text, text_of, type_line, unlock, wait_connected, wait_js,
};
use victauri_test::VictauriClient;

/// 两跳各用各的登录口令 —— **不一样**是有意的：给错口令就认证失败，
/// 于是"每一跳各问各的凭据"这件事有服务端那一侧的证据。
const JUMP_PASSWORD: &str = "e2e-jump-login-password";
const TARGET_PASSWORD: &str = "e2e-inner-login-password";

const JUMP_NAME: &str = "e2e-ssh-jump";
const TARGET_NAME: &str = "e2e-ssh-behind-jump";

/// **只有跳板认识**的名字与端口：`22` 在跳板那张表里只是一个键，真正连到哪由表决定 ——
/// 与真实 `HostName` + `ProxyJump` 完全同形。
const INNER_NAME: &str = "akasha-e2e-inner.invalid";
const INNER_PORT: u16 = 22;

/// 起两台服务端：目标（只在跳板后面）与跳板（带着"我替你连到哪"的那张表）。
async fn two_hosts() -> (Running, Running) {
    let target = start(ServerOptions::password(TARGET_PASSWORD)).await;
    let jump = start(ServerOptions {
        password: Some(JUMP_PASSWORD.to_owned()),
        relay: vec![Relay {
            host: INNER_NAME.to_owned(),
            port: INNER_PORT,
            to: target.addr,
        }],
        ..ServerOptions::default()
    })
    .await;
    (jump, target)
}

/// 在库里摆好两行：跳板（直连）与目标（`jump_id` 指向跳板）。
fn seed(path: &std::path::Path, jump_port: u16) -> (u64, u64) {
    let conn = open_vault(path);

    // 先清干净（重跑）。删的顺序要紧：目标引用着跳板（外键 `ON DELETE RESTRICT`）。
    forget(&conn, TARGET_NAME, INNER_NAME, INNER_PORT);
    forget(&conn, JUMP_NAME, "127.0.0.1", jump_port);

    let jump = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: JUMP_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port: jump_port,
            user: USER.to_owned(),
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();
    let target = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: TARGET_NAME.to_owned(),
            // ⚠️ **只有跳板认识**的名字（理由见文件头）。
            host: INNER_NAME.to_owned(),
            port: INNER_PORT,
            user: USER.to_owned(),
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: Some(jump),
        },
    )
    .unwrap();

    // 行 id 是 `i64`，IPC 那一侧是 `u32` 代理 —— 这里只用来在界面上找元素。
    (jump as u64, target as u64)
}

/// 回答提示五步里的"这一问" —— 循环本身搬到了 `support`（两个用例共用，见那边的文档）。
async fn answer(
    client: &mut VictauriClient,
    jump_port: u16,
    jump_fp: &str,
    target_fp: &str,
) -> Vec<String> {
    answer_prompts(
        client,
        PromptScript {
            tabs: 2,
            jump_port,
            jump_password: JUMP_PASSWORD,
            target_password: TARGET_PASSWORD,
            jump_fingerprint: jump_fp,
            target_fingerprint: target_fp,
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_through_a_bastion_reaches_a_host_only_it_can_see() {
    if support::skip_unless_e2e() {
        return;
    }

    // ── 1. 两台服务端（都在**本进程**里）────────────────────────────────────
    let (jump, target) = two_hosts().await;
    eprintln!(
        "构造: 跳板=127.0.0.1:{} 跳板指纹={} 目标={}:{} 目标地址={} 目标指纹={}",
        jump.addr.port(),
        jump.fingerprint,
        INNER_NAME,
        INNER_PORT,
        target.addr,
        target.fingerprint
    );

    // ── 2. "只对跳板机可见"的那一半：客户端拿到的名字**在本机解析不出来** ────
    // 这不是修辞：如果它能解析，下面那句"字节到了目标"就没法归功于跳板。
    let resolved = (INNER_NAME, INNER_PORT).to_socket_addrs();
    assert!(
        resolved.is_err(),
        "那个目标名在本机解析得出来（{:?}）—— 这条用例的构造前提就不成立了",
        resolved.map(|mut addrs| addrs.next())
    );
    eprintln!("构造: 目标名={INNER_NAME} 本机解析=失败");

    // ── 3. 库 + 种子数据 + 解锁 ─────────────────────────────────────────────
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };
    let (jump_id, target_id) = seed(&path, jump.addr.port());
    unlock(&mut client).await;

    // 池那一侧：跳板链在界面上看得见（`jumpId` 过 IPC 的形状）。
    let listed = client
        .invoke_command("vault_hosts", None)
        .await
        .expect("vault_hosts 调不通");
    eprintln!("池: 行数={}", listed.as_array().map_or(0, Vec::len));
    let rows = listed.as_array().cloned().unwrap_or_default();
    let target_row = rows
        .iter()
        .find(|row| row.pointer("/id").and_then(Value::as_u64) == Some(target_id))
        .expect("池里该有目标那一行");
    assert_eq!(
        target_row.pointer("/jumpId").and_then(Value::as_u64),
        Some(jump_id),
        "目标那一行要带跳板 —— 那是连接时真正会走的那条链"
    );
    assert_eq!(
        target_row.pointer("/host").and_then(Value::as_str),
        Some(INNER_NAME)
    );

    // ── 4. 界面：点 SSH → 选**目标**那行（它会经跳板）────────────────────────
    click(&mut client, ".tab-new-ssh", "打开主机选择器").await;
    wait_js(
        &mut client,
        &format!("!!document.querySelector('.host-picker-item[data-host-id=\"{target_id}\"]')"),
        10_000,
        "主机选择器列出了目标那一行",
    )
    .await;
    let badge = text_of(
        &mut client,
        &format!(".host-picker-item[data-host-id=\"{target_id}\"] .host-picker-jump"),
    )
    .await;
    assert!(
        badge.contains(JUMP_NAME),
        "选择器要显示这一行经谁连（拿到 {badge:?}）"
    );
    eprintln!("界面: 跳板={badge}");
    click(
        &mut client,
        &format!(".host-picker-item[data-host-id=\"{target_id}\"]"),
        "选那台经跳板的主机",
    )
    .await;

    // ── 5. 提示：**每一跳各一轮**（密钥 + 口令），按类型循环答完 ──────────────
    let asked = answer(
        &mut client,
        jump.addr.port(),
        &jump.fingerprint,
        &target.fingerprint,
    )
    .await;
    eprintln!("界面: 提示数={}", asked.len());
    assert!(
        asked
            .iter()
            .any(|entry| entry == &format!("hostKey:{}", jump.fingerprint)),
        "跳板的密钥该被问过一次（{asked:?}）"
    );
    assert!(
        asked
            .iter()
            .any(|entry| entry == &format!("hostKey:{}", target.fingerprint)),
        "**目标**的密钥也该被问过一次 —— 它是另一台机器（{asked:?}）"
    );
    wait_connected(&mut client, 2, "经跳板的 SSH 会话").await;
    let title = text(
        &client
            .eval_js("document.querySelector('.tab.is-active .tab-label')?.textContent ?? ''")
            .await
            .unwrap(),
    );
    assert_eq!(title, TARGET_NAME, "标签页标题该是目标那一行");

    // 两跳各收到各的口令 —— 给错了就认证不过，所以这一条同时说明"凭据没有串台"。
    let jump_seen = observed(&jump);
    let target_seen = observed(&target);
    assert_eq!(jump_seen.passwords, [JUMP_PASSWORD], "跳板收到的该是它那句");
    assert_eq!(
        target_seen.passwords,
        [TARGET_PASSWORD],
        "目标收到的该是它那句"
    );

    // ── 6. 跳板那一半：它被要求连的**正是那个只有它认识的名字** ───────────────
    assert_eq!(
        jump_seen.direct_tcpip.len(),
        1,
        "跳板应当恰好被要求转发一次，实际：{:?}",
        jump_seen.direct_tcpip
    );
    assert_eq!(jump_seen.direct_tcpip[0].host, INNER_NAME);
    assert_eq!(
        jump_seen.direct_tcpip[0].port,
        u32::from(INNER_PORT),
        "端口也要是池里那一列 —— 客户端不该自己改写成真实端口"
    );
    assert!(jump_seen.shell_data.is_empty(), "跳板上不该有终端数据");

    // ── 7. 字节能双向流：到了**目标**，而目标在客户端解析不出来的地址上 ───────
    type_line(&mut client, "echo via-the-bastion").await;
    wait_js(
        &mut client,
        "window.__akashaTerminal.screenText(200).includes('via-the-bastion')",
        support::CONNECT_TIMEOUT_MS,
        "目标的回声出现在了终端上",
    )
    .await;
    let seen = String::from_utf8_lossy(&observed(&target).shell_data).to_string();
    assert!(
        seen.contains("echo via-the-bastion"),
        "目标服务端没收到那行字节（收到 {seen:?}）"
    );
    assert!(
        jump.shared.relayed_bytes() > 0,
        "跳板上的中继应当搬过字节（它真的通着）"
    );
    eprintln!("服务端: 中继字节={}", jump.shared.relayed_bytes());

    // ── 8. 关标签页零残留 ──────────────────────────────────────────────────
    let closed = client
        .eval_js(
            "(() => { const tabs = document.querySelectorAll('.tab.is-active .tab-close'); \
             if (!tabs.length) return false; tabs[0].click(); return true; })()",
        )
        .await
        .unwrap();
    assert!(
        support::payload(&closed).as_bool().unwrap_or(false),
        "关不掉 SSH 标签页：{closed}"
    );
    wait_js(
        &mut client,
        "document.querySelectorAll('.tab').length === 1",
        CLOSE_TIMEOUT.as_millis() as u64,
        "SSH 标签页关掉了",
    )
    .await;

    let deadline = std::time::Instant::now() + CLOSE_TIMEOUT;
    let sessions = loop {
        let sessions = client
            .call_tool("app_state", json!({ "probe": "sessions" }))
            .await
            .expect("读不到 sessions probe");
        if sessions.pointer("/live").and_then(Value::as_u64) == Some(1)
            && sessions.pointer("/registered").and_then(Value::as_u64) == Some(1)
        {
            break sessions;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "关掉标签页之后后端还登记着会话：{sessions}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    eprintln!(
        "会话: live={} registered={}",
        sessions["live"], sessions["registered"]
    );

    // 对端：**整条链**（目标那条会话 + 跳板那条承载连接）都断了 —— 这是"连接真的没了"的
    // 唯一外部证据，而跳板那一条正是"载着我们的那条连接"。
    let deadline = std::time::Instant::now() + CLOSE_TIMEOUT;
    loop {
        let closed = observed(&target).sessions_closed;
        if closed >= 1 {
            eprintln!("服务端: 连接关闭={closed}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "标签页关掉了，目标服务端却还看到连接挂着"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── 9. 收尾 ────────────────────────────────────────────────────────────
    let _ = client.invoke_command("vault_lock", None).await;
}
