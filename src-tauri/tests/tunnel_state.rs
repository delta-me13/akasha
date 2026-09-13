//! plan 0601 的**端到端**验收：真 app 上开一条隧道，看它的状态与事件。
//!
//! 判据（ROADMAP 原文）=「五态**可观测**；状态变化**发事件**」。这条用例逐条盯着它们：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 规则能被界面看见 | 点 `.tab-new-tunnel` → `.tunnel-item[data-rule-id]` 出现 |
//! | `连接中 → 已连接` | 答完主机密钥与口令 → probe `tunnels` 说 `connected` |
//! | 状态变化**发事件** | `window.__akashaTunnels.events` 里按序有 `connecting` 与 `connected`，handle 与 probe 一致 |
//! | `→ 已停止` | 点"停止" → probe 里那条消失，且 `sessions` 的 live 与 registered 相等 |
//! | `连接中 → 失败`（失败必须可见） | 打不开的那条：命令回非空 `failure`、probe 说 `failed`、事件里也有 `failed` |
//! | 手动重试（D12） | `tunnel_retry` → 状态回到 `connecting`，并再失败一次 |
//! | **不含重连** | 全程**没有** `reconnecting` —— 驱动它的重连循环是 plan 0605 |
//!
//! ## 服务端在**测试进程**里
//!
//! 同 `ssh_session` / `ssh_jump`：服务端在测试进程，app 连过来。隧道在 plan 0601 里
//! **只建立连接、不转发**（三种转发机制在 0602 / 0603 / 0604），所以这条用例断言的是
//! **状态机与事件**，不是字节流。
//!
//! ## 数据从哪来
//!
//! 转发规则池**没有写路径的界面**（plan 0601 的非目标），所以规则由这条用例直接写库 ——
//! 与 plan 0505 构造跳板链是同一种做法。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use akasha_ssh::testing::{ServerOptions, start};
use akasha_store::pools::{forwards, hosts};
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, USER, click, connect_and_prepare, fill_secret, forget, open_vault, text,
    text_of, unlock, wait_js,
};
use victauri_test::VictauriClient;

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-ssh-login-password";
/// 池里那台主机的名字。
const HOST_NAME: &str = "e2e-tunnel-host";
/// 池里那台**连不上**的主机的名字。
///
/// ⚠️ "失败"必须构造在**主机**这一层：plan 0601 的隧道只建立到规则所属主机的连接
/// （`target_host` / `target_port` 是 plan 0602 的 `-L` 才用到的东西）——
/// 把目标端口写成 1 不会让它失败，因为这一版根本不连目标。
const BROKEN_HOST_NAME: &str = "e2e-tunnel-unreachable";
/// 池里那条能连通的转发规则名。
const TUNNEL_NAME: &str = "e2e-tunnel-open";
/// 池里那条连不上的转发规则名（它的主机是那个没人监听的端口）。
const BROKEN_NAME: &str = "e2e-tunnel-broken";
/// "连不上"用的端口：1 号端口没有服务、且普通进程连不上它 —— 它给的是拒绝连接，
/// 不是超时（用例因此不必等满 `connect_timeout`）。
const UNREACHABLE_PORT: u16 = 1;
/// 规则里那个绑定端口。**plan 0601 不监听它**（本地监听是 plan 0602），
/// 但库的 `CHECK` 要求它是一个合法端口。
const BIND_PORT: u16 = 46_010;

/// 在库里把这次要用的东西摆好：两台主机 + 两条转发规则。
///
/// 返回 `(能连通的那条规则, 连不上的那条规则)`。
fn seed(path: &Path, port: u16) -> (i64, i64) {
    let conn = open_vault(path);

    // 先清干净（重跑）：主机行与规则行都按名字清。
    forget(&conn, HOST_NAME, "127.0.0.1", port);
    forget(&conn, BROKEN_HOST_NAME, "127.0.0.1", UNREACHABLE_PORT);
    for row in forwards::forwards(&conn).unwrap() {
        if row.name == TUNNEL_NAME || row.name == BROKEN_NAME {
            forwards::delete_forward(&conn, row.id).unwrap();
        }
    }

    let host_id = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: HOST_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port,
            user: USER.to_owned(),
            // 口令认证：这条用例不依赖 agent，也不依赖任何密钥材料。
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();

    let broken_host_id = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: BROKEN_HOST_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port: UNREACHABLE_PORT,
            user: USER.to_owned(),
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();

    let open = forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: TUNNEL_NAME.to_owned(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".to_owned(),
            bind_port: BIND_PORT,
            target_host: Some("127.0.0.1".to_owned()),
            target_port: Some(port),
            host_id,
            autostart: false,
        },
    )
    .unwrap();

    let broken = forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: BROKEN_NAME.to_owned(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".to_owned(),
            bind_port: BIND_PORT + 1,
            target_host: Some("127.0.0.1".to_owned()),
            target_port: Some(port),
            // 这一行指向那台**连不上**的主机 —— 失败在这一层。
            host_id: broken_host_id,
            autostart: false,
        },
    )
    .unwrap();

    (open, broken)
}

/// `app_state { probe: "tunnels" }` 的原始列表。
async fn tunnel_entries(client: &mut VictauriClient) -> Vec<Value> {
    let value = client
        .call_tool("app_state", json!({ "probe": "tunnels" }))
        .await
        .expect("读不到 tunnels probe —— 它注册进 lib.rs 了吗？");
    value.as_array().cloned().unwrap_or_default()
}

/// 前端记下的 `tunnel_state` 事件（测试接口，见 `src/ipc/tunnels.ts`）。
async fn tunnel_events(client: &mut VictauriClient) -> Vec<Value> {
    let raw = client
        .eval_js("JSON.stringify(window.__akashaTunnels?.events ?? [])")
        .await
        .unwrap();
    let encoded = text(&raw);
    serde_json::from_str(&encoded).unwrap_or_default()
}

/// 等 probe 里某条隧道到达某个状态，返回它的 handle。
async fn wait_state(client: &mut VictauriClient, rule_id: i64, want: &str) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        for entry in tunnel_entries(client).await {
            let matches_rule = entry.pointer("/ruleId").and_then(Value::as_i64) == Some(rule_id);
            let state = entry.pointer("/state").and_then(Value::as_str);
            if matches_rule && state == Some(want) {
                return entry
                    .pointer("/handle")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
            }
        }
        assert!(
            Instant::now() < deadline,
            "等不到规则 {rule_id} 变成 {want}：{:?}",
            tunnel_entries(client).await
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 边答提示边等那条隧道连上，返回它收到的提示。
///
/// 为什么不写死"先密钥后口令"两步：链上每一跳各来一轮（plan 0505 的教训），
/// 写死步数会在拓扑一变时**静默少答一轮**，表现是"连不上"而不是"用例写错了"。
async fn connect_through_prompts(
    client: &mut VictauriClient,
    rule_id: i64,
    fingerprint: &str,
) -> (u64, Vec<String>) {
    let mut asked = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(90);
    let any_prompt = "!!document.querySelector('.ssh-prompt[data-prompt-kind=\"hostKey\"]') \
                      || !!document.querySelector('.ssh-prompt[data-prompt-kind=\"credential\"]')";
    loop {
        for entry in tunnel_entries(client).await {
            if entry.pointer("/ruleId").and_then(Value::as_i64) == Some(rule_id)
                && entry.pointer("/state").and_then(Value::as_str) == Some("connected")
            {
                let handle = entry
                    .pointer("/handle")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                return (handle, asked);
            }
        }
        if Instant::now() >= deadline {
            let problem = text_of(client, ".tunnel-failure").await;
            panic!("提示问答没走完就连不上（问到过的：{asked:?}；面板上的失败={problem:?}）");
        }
        let _ = client
            .wait_for_expression(any_prompt, None, Some(3_000), None)
            .await;

        let shown = text_of(
            client,
            ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-fingerprint",
        )
        .await;
        if !shown.is_empty() {
            assert_eq!(shown, fingerprint, "提示里的指纹必须就是服务端的");
            asked.push(format!("hostKey:{shown}"));
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
            fill_secret(client, PASSWORD).await;
            click(
                client,
                ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-submit",
                "提交口令",
            )
            .await;
            continue;
        }
    }
}

/// 等一条隧道从后端消失（停止之后）。
async fn wait_gone(client: &mut VictauriClient, handle: u64) {
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        let left = tunnel_entries(client).await;
        if !left
            .iter()
            .any(|entry| entry.pointer("/handle").and_then(Value::as_u64) == Some(handle))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "停掉的隧道还在 probe 里：{left:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tunnel_reports_five_states_through_events_and_can_be_stopped() {
    if support::skip_unless_e2e() {
        return;
    }

    // ── 1. 服务端（在**本进程**里）：随机端口 + 只认口令 ─────────────────────
    let server = start(ServerOptions::password(PASSWORD)).await;
    eprintln!(
        "服务端：127.0.0.1:{} 指纹 {}",
        server.addr.port(),
        server.fingerprint
    );

    // ── 2. 库在哪、必要时先锁上 ──────────────────────────────────────────────
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };

    // ── 3. 种子数据 + 解锁 ─────────────────────────────────────────────────
    let (open_rule, broken_rule) = seed(&path, server.addr.port());
    unlock(&mut client).await;

    // 规则池的只读命令：界面凭什么列出规则（plan 0601 新增）。
    let listed = client
        .invoke_command("vault_forwards", None)
        .await
        .expect("vault_forwards 调不通 —— 它登记进 bindings.rs 了吗？");
    let rows = listed.as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 2, "池里应当只有我们种下的两条：{listed}");
    assert_eq!(
        rows[0].pointer("/direction").and_then(Value::as_str),
        Some("local"),
        "方向过 IPC 是一个稳定短名：{listed}"
    );
    eprintln!("转发规则池：{listed}");

    // ── 4. 界面：打开隧道面板，规则列出来 ────────────────────────────────────
    click(&mut client, ".tab-new-tunnel", "打开隧道面板").await;
    wait_js(
        &mut client,
        &format!("!!document.querySelector('.tunnel-item[data-rule-id=\"{open_rule}\"]')"),
        10_000,
        "面板列出了池里的规则",
    )
    .await;

    // ── 5. 打开那条能连通的：连接中 → 已连接 ────────────────────────────────
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{open_rule}\"] .tunnel-open"),
        "打开这条隧道",
    )
    .await;
    let (handle, asked) =
        connect_through_prompts(&mut client, open_rule, &server.fingerprint).await;
    assert!(handle > 0, "probe 里必须有 handle");
    eprintln!("隧道已连接：handle={handle}，问到过 {asked:?}");

    // 界面上的状态与后端一致（`data-tunnel-state` 的取值就是后端那个短名）。
    wait_js(
        &mut client,
        &format!(
            "document.querySelector('.tunnel-item[data-rule-id=\"{open_rule}\"] \
             .tunnel-state')?.dataset.tunnelState === 'connected'"
        ),
        10_000,
        "界面上的状态跟着后端变成 connected",
    )
    .await;

    // ── 6. 判据：状态变化**发事件**，且按 `SessionId` 路由 ───────────────────
    let events = tunnel_events(&mut client).await;
    let states: Vec<&str> = events
        .iter()
        .filter(|event| event.pointer("/handle").and_then(Value::as_u64) == Some(handle))
        .filter_map(|event| event.pointer("/state").and_then(Value::as_str))
        .collect();
    assert!(
        states.contains(&"connecting"),
        "事件序列里必须有 connecting（连接要几秒，界面该立刻看到它在连）：{events:?}"
    );
    assert!(
        states.contains(&"connected"),
        "事件序列里必须有 connected：{events:?}"
    );
    assert!(
        !states.contains(&"reconnecting"),
        "plan 0601 不由真实路径产生「重连中」（那是 0605 的循环）：{events:?}"
    );
    assert_eq!(
        events
            .iter()
            .find(|event| event.pointer("/state").and_then(Value::as_str) == Some("connecting"))
            .and_then(|event| event.pointer("/attempt"))
            .cloned(),
        Some(Value::Null),
        "非「重连中」的状态不该带 attempt（拿不到就不写，不填 0）：{events:?}"
    );
    eprintln!("事件序列（handle={handle}）：{states:?}");

    // 服务端那一侧：它确实看到一次口令认证（不是"状态说连上了"就算）。
    let passwords = support::observed(&server).passwords;
    assert_eq!(
        passwords.last().map(String::as_str),
        Some(PASSWORD),
        "服务端该看到这次认证用的口令：{passwords:?}"
    );

    // ── 7. 停止：→ 已停止 + 注销 ────────────────────────────────────────────
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{open_rule}\"] .tunnel-stop"),
        "停止这条隧道",
    )
    .await;
    wait_gone(&mut client, handle).await;
    let sessions = client
        .call_tool("app_state", json!({ "probe": "sessions" }))
        .await
        .expect("读不到 sessions probe");
    assert_eq!(
        (
            sessions.pointer("/live").and_then(Value::as_u64),
            sessions.pointer("/registered").and_then(Value::as_u64)
        ),
        (Some(1), Some(1)),
        "停掉之后只剩本地那个终端会话，且两张表必须相等：{sessions}"
    );
    let stopped = tunnel_events(&mut client).await;
    assert!(
        stopped.iter().any(|event| {
            event.pointer("/handle").and_then(Value::as_u64) == Some(handle)
                && event.pointer("/state").and_then(Value::as_str) == Some("stopped")
        }),
        "停止也要发事件（用户看得见的那一步）：{stopped:?}"
    );
    eprintln!("停止：probe = {sessions}");

    // 对端：服务端看到那条**连接**断了 —— 这是"连接真的没了"的唯一外部证据
    // （`disconnect` 是在 runtime 上排队的，所以这里等它落地）。
    // ⚠️ 数的是**连接**级的那个计数：隧道没有通道（ADR-0003 D4），
    // 所以 `sessions_closed`（通道级）在这条路上永远是 0。
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        let closed = support::observed(&server).connections_closed;
        if closed >= 1 {
            eprintln!("停止：服务端看到 {closed} 条连接断开");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "隧道停掉了，服务端却还看到连接挂着（只有 {closed} 条断开）"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── 8. 连接中 → 失败（失败必须可见），再手动重试 ─────────────────────────
    let broken = client
        .invoke_command("tunnel_open", Some(json!({ "forwardId": broken_rule })))
        .await
        .expect("tunnel_open 调不通 —— 它登记进 bindings.rs 了吗？");
    let broken_handle = broken
        .pointer("/handle")
        .and_then(Value::as_u64)
        .expect("连不上也要给 handle（那条隧道仍然在册、可重试）");
    let failure = broken
        .pointer("/failure")
        .ok_or_else(|| panic!("连不上必须有原因：{broken}"))
        .unwrap();
    assert!(
        failure.is_object(),
        "failure 该是那套按「下一步动作」分类的错误：{broken}"
    );
    eprintln!("打不开的那条：{broken}");

    // 状态落在 `失败`（这正是"失败必须可见"的机器可读那一半）。
    wait_state(&mut client, broken_rule, "failed").await;
    let failed_events = tunnel_events(&mut client).await;
    assert!(
        failed_events.iter().any(|event| {
            event.pointer("/handle").and_then(Value::as_u64) == Some(broken_handle)
                && event.pointer("/state").and_then(Value::as_str) == Some("failed")
        }),
        "失败也要发事件（托盘与界面读的就是它）：{failed_events:?}"
    );

    // 手动重试（D12）：回到连接中，然后**再失败一次** —— 状态机真的又走了一遍。
    let before = failed_events.len();
    let retried = client
        .invoke_command("tunnel_retry", Some(json!({ "handle": broken_handle })))
        .await
        .expect("tunnel_retry 调不通");
    assert!(
        retried.pointer("/failure").map(Value::is_object) == Some(true),
        "重试之后仍然连不上（目标还是那个没人监听的端口）：{retried}"
    );
    let after = tunnel_events(&mut client).await;
    let retry_states: Vec<&str> = after
        .iter()
        .skip(before)
        .filter(|event| event.pointer("/handle").and_then(Value::as_u64) == Some(broken_handle))
        .filter_map(|event| event.pointer("/state").and_then(Value::as_str))
        .collect();
    assert!(
        retry_states.contains(&"connecting") && retry_states.contains(&"failed"),
        "重试必须**再走一遍**连接中 → 失败：{retry_states:?}"
    );
    eprintln!("重试：{retry_states:?}");

    // ── 9. 收尾：把失败那条也停掉，然后锁库 ──────────────────────────────────
    client
        .invoke_command("tunnel_stop", Some(json!({ "handle": broken_handle })))
        .await
        .expect("tunnel_stop 调不通");
    wait_gone(&mut client, broken_handle).await;
    assert!(
        tunnel_entries(&mut client).await.is_empty(),
        "两条都停掉之后 probe 该是空的"
    );
    let _ = client.invoke_command("vault_lock", None).await;
}
