//! plan 0605 的**端到端**验收：真 app 上的**断线重连**。
//!
//! 判据（ROADMAP 原文）=「拔网线后进入「重连中」，耗尽次数后变「失败」**且托盘可见**；
//! 可手动重试」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 掉线进入「重连中」 | 切断 SSH 连接（测试服务端监听不变）→ 事件里出现 `reconnecting`（`attempt = 1`） |
//! | 界面看得见那一态 | 那一刻面板上写着「重连中（第 1 次）」 |
//! | **连得回来** | 事件里随后又有 `connected`；**两个转发端口都又能用**（真 `curl`） |
//! | 重连是"重新连一次" | 服务端的 `direct_tcpip` 多一条、`connections_closed ≥ 2` |
//! | **`-R` 重连要重新请求监听** | 服务端的 `tcpip_forward` 请求数**变成 2**，端口还是规则里那个 |
//! | 重连不再问一遍凭据 | 全程没有新的提示（否则 `connected` 永远等不到）—— 凭据缓存在内存里（D8） |
//! | 停止要能中止循环 | 重连途中点"停止" → 它从 probe 里消失，且 3 秒内**不再**出现 `connected` |
//! | **耗尽次数变「失败」且可见** | 停掉整个服务端 → 序列 `reconnecting(1,2,3)` + `failed`，全程 ≥ 7 s，probe / 面板 / 托盘三处都写着失败 |
//! | 失败之后仍可手动重试 | `tunnel_retry` → 又有一次 `connecting`，并再失败一次 |
//!
//! ## 怎么造"断线"
//!
//! 无特权环境拔不了网线，所以断线由**服务端那一侧**造：`Running::cut_connections()` 让
//! 测试服务端把已建立的会话断开（监听留着 —— 于是重连**接得上**），`Running::shutdown()`
//! 连监听一起停（于是重连**必然失败**，三条判据里的"耗尽"由它造）。
//!
//! 两种刺激都要真的走到**客户端**：客户端看到的是一次连接结束，与对端进程消失同形。
//!
//! ## 两侧都在同一个进程里，那"远端"是什么
//!
//! 同 plan 0602 / 0603 / 0604：区分"远端"与"本机"的是**谁在听**。
//! `local` 那条的目标名只有**服务端**认识（中继表），`remote` 那条的监听端口由**服务端**绑。
//! `curl` 只对着转发端口说话，本机 HTTP 服务只认经隧道来的连接 —— 两半的计数是这条推理的证据。
//!
//! ## 数据从哪来
//!
//! 转发规则池**没有写路径的界面**（plan 0601 的非目标），所以规则由这条用例直接写库 ——
//! 同 `tunnel_state` / `tunnel_local_forward` / `tunnel_dynamic_forward` / `tunnel_remote_forward`。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use akasha_lib::ssh::testing::{Relay, ServerOptions, start};
use akasha_store::pools::{forwards, hosts};
use serde_json::{Value, json};
use support::{
    USER, click, connect_and_prepare, connect_tunnel_through_prompts, forget, free_port,
    open_tunnel_panel, open_vault, text_of, tunnel_entries, unlock, wait_js, wait_text_contains,
    wait_tunnel_gone,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use victauri_test::VictauriClient;

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-reconnect-password";
/// 池里那台主机的名字。
const HOST_NAME: &str = "e2e-reconnect-host";
/// 池里那条本机转发（`-L`）规则的名字 —— 重连与耗尽的判据都盯它。
const LOCAL_NAME: &str = "e2e-reconnect-local";
/// 池里那条远程转发（`-R`）规则的名字 —— "重连要重新请求监听"盯它。
const REMOTE_NAME: &str = "e2e-reconnect-remote";
/// **只有对端认识**的名字（RFC 2606 保留域；`local` 那条的目标）。
const TARGET_NAME: &str = "akasha-reconnect.invalid";
/// 本机 HTTP 服务会写进响应体的那串字节。
const BODY: &str = "akasha-reconnect-e2e\n";
/// `curl` 的等待上限。
const CURL_TIMEOUT_SECS: &str = "20";
/// D13 的重连预算：`1s + 2s + 4s`。
const BUDGET: Duration = Duration::from_secs(7);
/// 等一个状态 / 一条事件的上限（连接尝试各自最长 10 秒，余量留够）。
const WAIT_MS: u64 = 60_000;

/// 一个进程内的 HTTP 服务端 —— 它就是"本机上的服务"，外加一个请求计数。
///
/// 计数说明**这个**服务端真的被连过（"响应体回来了"也可能是别的东西答的）。
async fn start_http() -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("HTTP 服务绑定失败");
    let addr = listener.local_addr().expect("取 HTTP 服务地址失败");
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                // 读到请求头结束（CRLFCRLF）就够了 —— 这是 GET，没有请求体。
                let mut head = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    match socket.read(&mut chunk).await {
                        Ok(0) => break,
                        Ok(read) => {
                            head.extend_from_slice(&chunk[..read]);
                            if head.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                        Err(_) => return,
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{BODY}",
                    BODY.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (addr, hits)
}

/// 经某个转发端口取一次本机服务（**真实客户端**）。
fn curl(port: u16) -> (bool, String, String) {
    let output = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--max-time")
        .arg(CURL_TIMEOUT_SECS)
        .arg(format!("http://127.0.0.1:{port}/probe"))
        .output()
        .expect("curl 起不来");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// 经某个转发端口取一次，并断言取到的就是本机服务写的那一串。
fn assert_curl(port: u16, what: &str) {
    let (ok, body, err) = curl(port);
    assert!(
        ok && body == BODY,
        "{what}：`curl http://127.0.0.1:{port}/probe` 拿到的不是本机服务的响应体\
         （退出码 {ok}，stdout {body:?}，stderr {err:?}）"
    );
}

/// 在库里把这次要用的东西摆好：一台主机 + 一条 `local` 规则 + 一条 `remote` 规则。
///
/// 返回 `(local 规则 id, remote 规则 id, local 的绑定端口, remote 的绑定端口)`。
fn seed(path: &Path, ssh_port: u16, http_port: u16) -> (i64, i64, u16, u16) {
    let conn = open_vault(path);

    // 先清干净（重跑）：主机行与规则行都按名字清。
    forget(&conn, HOST_NAME, "127.0.0.1", ssh_port);
    for row in forwards::forwards(&conn).unwrap() {
        if row.name == LOCAL_NAME || row.name == REMOTE_NAME {
            forwards::delete_forward(&conn, row.id).unwrap();
        }
    }

    let host_id = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: HOST_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port: ssh_port,
            user: USER.to_owned(),
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();

    // 绑定端口现取一个空闲的：写死会在"那个端口恰好被占用"时红在与本次改动无关的原因上。
    let local_port = free_port();
    let remote_port = free_port();
    let local_rule = forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: LOCAL_NAME.to_owned(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".to_owned(),
            bind_port: local_port,
            // 目标由**对端**解析（这个域名只在它的中继表里存在）。
            target_host: Some(TARGET_NAME.to_owned()),
            target_port: Some(http_port),
            host_id,
            autostart: false,
        },
    )
    .unwrap();
    let remote_rule = forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: REMOTE_NAME.to_owned(),
            // `-R`：绑定在**服务端**那一侧，目标才是本机服务。
            direction: forwards::Direction::Remote,
            bind_host: "127.0.0.1".to_owned(),
            bind_port: remote_port,
            target_host: Some("127.0.0.1".to_owned()),
            target_port: Some(http_port),
            host_id,
            autostart: false,
        },
    )
    .unwrap();

    (local_rule, remote_rule, local_port, remote_port)
}

/// probe 里某个句柄那一行。
async fn entry_of(client: &mut VictauriClient, handle: u64) -> Option<Value> {
    tunnel_entries(client)
        .await
        .into_iter()
        .find(|entry| entry.pointer("/handle").and_then(Value::as_u64) == Some(handle))
}

/// 打开某条规则的隧道，并把提示答完，返回 handle。
async fn open_tunnel_for(client: &mut VictauriClient, rule_id: i64, fingerprint: &str) -> u64 {
    wait_js(
        client,
        &format!("!!document.querySelector('.tunnel-item[data-rule-id=\"{rule_id}\"]')"),
        10_000,
        "面板列出了池里的规则",
    )
    .await;
    click(
        client,
        &format!(".tunnel-item[data-rule-id=\"{rule_id}\"] .tunnel-open"),
        "打开这条隧道",
    )
    .await;
    let (handle, asked) =
        connect_tunnel_through_prompts(client, rule_id, fingerprint, PASSWORD).await;
    assert!(handle > 0, "probe 里必须有 handle（问过：{asked:?}）");
    handle
}

/// 某个句柄收到过的状态事件，按发生顺序：`(状态, 次数)`。
async fn states_of(client: &mut VictauriClient, handle: u64) -> Vec<(String, Option<u64>)> {
    let raw = client
        .eval_js(&format!(
            "JSON.stringify((window.__akashaTunnels?.events ?? []) \
             .filter((e) => e.handle === {handle}) \
             .map((e) => [e.state, e.attempt ?? null]))"
        ))
        .await
        .unwrap();
    let encoded = support::text(&raw);
    let rows: Vec<Value> = serde_json::from_str(&encoded).unwrap_or_default();
    rows.iter()
        .filter_map(|row| {
            let state = row.get(0)?.as_str()?.to_owned();
            Some((state, row.get(1).and_then(Value::as_u64)))
        })
        .collect()
}

/// 等某个句柄达到某个状态（probe 里那一行）。
async fn wait_state(client: &mut VictauriClient, handle: u64, want: &str) {
    let deadline = Instant::now() + Duration::from_millis(WAIT_MS);
    loop {
        let now = entry_of(client, handle).await.and_then(|entry| {
            entry
                .pointer("/state")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        if now.as_deref() == Some(want) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "等不到句柄 {handle} 变成 {want}（现在是 {now:?}）：{:?}",
            tunnel_entries(client).await
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 等某个句柄**先进入「重连中」、之后又回到「已连接」**（按事件流的顺序看，不看某一刻的快照）。
///
/// 为什么不分成"等 reconnecting 再等 connected"两次快照：第一次要等的那一态可能在你去看
/// 之前就过去了（退避只有 1 秒），而事件流是追加的 —— 它记着"确实发生过"。
async fn wait_reconnected(client: &mut VictauriClient, handle: u64) {
    wait_js(
        client,
        &format!(
            "(() => {{ const events = (window.__akashaTunnels?.events ?? []) \
             .filter((e) => e.handle === {handle}); \
             const at = events.findIndex((e) => e.state === 'reconnecting'); \
             return at >= 0 && events.slice(at).some((e) => e.state === 'connected'); }})()"
        ),
        WAIT_MS,
        "重连没走完（重连中 → 已连接）",
    )
    .await;
}

/// 等某个句柄收到过第 `attempt` 次「重连中」。
async fn wait_attempt(client: &mut VictauriClient, handle: u64, attempt: u64) {
    wait_js(
        client,
        &format!(
            "(window.__akashaTunnels?.events ?? []).some((e) => e.handle === {handle} \
             && e.state === 'reconnecting' && e.attempt === {attempt})"
        ),
        WAIT_MS,
        &format!("没等到第 {attempt} 次「重连中」"),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_connection_is_retried_and_its_exhaustion_is_visible() {
    if support::skip_unless_e2e() {
        return;
    }
    // `curl` 是硬前提（同 `tunnel_dynamic_forward` / `tunnel_remote_forward`）：
    // 判据是"真的有个客户端连得上"，没有客户端就没有这条证据。
    assert!(
        Command::new("curl")
            .arg("--version")
            .output()
            .is_ok_and(|out| out.status.success()),
        "这条用例要一个真实的 HTTP 客户端（curl）"
    );

    // ── 1. HTTP 服务 + SSH 服务端（中继表是那个名字唯一的存在方式）──────────────
    let (http, hits) = start_http().await;
    let server = start(ServerOptions {
        password: Some(PASSWORD.to_owned()),
        relay: vec![Relay {
            host: TARGET_NAME.to_owned(),
            port: http.port(),
            to: http,
        }],
        ..ServerOptions::default()
    })
    .await;
    eprintln!(
        "服务端：127.0.0.1:{}（本机 HTTP 服务在 {}）",
        server.addr.port(),
        http.port()
    );

    // ── 2. 库 + 种子数据 + 解锁 ──────────────────────────────────────────────
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };
    let (local_rule, remote_rule, local_port, remote_port) =
        seed(&path, server.addr.port(), http.port());
    unlock(&mut client).await;

    // ── 3. 两条隧道都打开（两条连接）→ 两个端口都能用 ─────────────────────────
    open_tunnel_panel(&mut client).await;
    let local_handle = open_tunnel_for(&mut client, local_rule, &server.fingerprint).await;
    let remote_handle = open_tunnel_for(&mut client, remote_rule, &server.fingerprint).await;
    assert_curl(local_port, "本机转发（切之前）");
    assert_curl(remote_port, "远端转发（切之前）");
    let before = support::observed(&server);
    assert_eq!(
        before.forward_requests.len(),
        1,
        "远端那条此刻只该被请求过一次：{:?}",
        before.forward_requests
    );

    // ── 4. 切断线路 → 两条都进「重连中」→ 两条都回到「已连接」→ 端口又能用 ────
    let cut = server.cut_connections().await;
    assert!(
        cut >= 2,
        "两条隧道各一条连接，应当都切到（实际切了 {cut} 条）"
    );

    wait_attempt(&mut client, local_handle, 1).await;
    // 界面那一半：那一刻面板上写着「重连中（第 n 次）」—— 退避是 1 秒，读得到。
    let panel = format!(".tunnel-item[data-rule-id=\"{local_rule}\"] .tunnel-state");
    wait_text_contains(&mut client, &panel, "重连中").await;
    let shown = text_of(&mut client, &panel).await;
    assert!(
        shown.contains("第 1 次"),
        "面板要报出**第几次**（用户据此判断还要等多久）：{shown:?}"
    );
    eprintln!("面板上那一刻：{shown}");
    wait_attempt(&mut client, remote_handle, 1).await;

    wait_reconnected(&mut client, local_handle).await;
    wait_reconnected(&mut client, remote_handle).await;
    eprintln!("两条隧道都自己回来了");

    // **判据：连得回来** —— 两个转发端口都又能用（真实客户端）。
    assert_curl(local_port, "本机转发（重连之后）");
    assert_curl(remote_port, "远端转发（重连之后）");
    assert!(
        hits.load(Ordering::SeqCst) >= 4,
        "重连之后两个端口各该再取到一次"
    );

    // `-L`：重连是**另起一条连接**（新的 direct-tcpip 通道），不是把旧的接着用。
    let after = support::observed(&server);
    assert!(
        after.direct_tcpip.len() >= 2,
        "重连之后该有一条新的转发通道：{:?}",
        after.direct_tcpip
    );
    assert!(
        after.connections_closed >= 2,
        "切掉的两条连接都该在服务端记到断开（实际 {}）",
        after.connections_closed
    );
    // **`-R`：重连必须重新请求监听** —— 远端监听是那条连接的资源，连接一断它就没了。
    // 这一条是 plan 0604 留给 plan 0605 的那个陷阱，也是本条用例最要紧的断言之一。
    assert_eq!(
        after.forward_requests.len(),
        2,
        "远端转发重连时要重新发一次 tcpip_forward：{:?}",
        after.forward_requests
    );
    assert!(
        after
            .forward_requests
            .iter()
            .all(|request| request.accepted),
        "两次请求都该被认下：{:?}",
        after.forward_requests
    );
    assert_eq!(
        after.forward_requests[1].port,
        u32::from(remote_port),
        "重新请求的端口就是规则里那个（重连之后端口不许悄悄变）：{:?}",
        after.forward_requests
    );

    // ── 5. 重连途中停止：循环要跟着它停 ─────────────────────────────────────
    // 再切一次，然后在退还避里（1 秒）停掉本机那条。
    let connected_before = states_of(&mut client, local_handle)
        .await
        .iter()
        .filter(|(state, _)| state == "connected")
        .count();
    let cut = server.cut_connections().await;
    assert!(cut >= 2, "两条连接都该还在（实际切了 {cut} 条）");
    wait_js(
        &mut client,
        &format!(
            "(window.__akashaTunnels?.events ?? []).some((e) => e.handle === {local_handle} \
             && e.state === 'reconnecting')"
        ),
        WAIT_MS,
        "第二条路切了之后那条隧道没进「重连中」",
    )
    .await;
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{local_rule}\"] .tunnel-stop"),
        "重连途中停止这条隧道",
    )
    .await;
    wait_tunnel_gone(&mut client, local_handle).await;

    // 负控：退避是 1 秒，所以 3 秒之内若它又被自己的循环拉起来，`connected` 事件必然出现。
    // 这里等的是"出现"这件事 —— 超时（`ok = false`）才是通过。
    let resumed = client
        .wait_for_expression(
            &format!(
                "(window.__akashaTunnels?.events ?? []).filter((e) => e.handle === {local_handle} \
                 && e.state === 'connected').length > {connected_before}"
            ),
            None,
            Some(3_000),
            None,
        )
        .await
        .unwrap();
    assert_ne!(
        resumed.get("ok").and_then(Value::as_bool),
        Some(true),
        "停止之后那条隧道又被重连循环拉起来了：{resumed}"
    );
    eprintln!("停止之后 3 秒内没有新的 connected：重连循环真的停了");

    // ── 6. 耗尽次数 → 失败（probe / 面板 / 托盘三处都看得见）─────────────────
    // 远端那条先停掉：下面要让整个服务端消失，观察点只剩本机那条才干净。
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{remote_rule}\"] .tunnel-stop"),
        "停止远端那条",
    )
    .await;
    wait_tunnel_gone(&mut client, remote_handle).await;

    // 重新打开本机那条：它刚才是在重连途中被停掉的，要先真的连上，再让服务端消失。
    let handle = open_tunnel_for(&mut client, local_rule, &server.fingerprint).await;
    assert_curl(local_port, "重新打开之后");

    let started = Instant::now();
    let gone = server.shutdown().await;
    assert!(gone >= 1, "服务端消失时该切掉那条连接（实际 {gone} 条）");
    wait_state(&mut client, handle, "failed").await;
    let elapsed = started.elapsed();
    eprintln!("从服务端消失到「失败」用了 {elapsed:?}");
    assert!(
        elapsed >= BUDGET,
        "重连预算 1s + 2s + 4s 至少是 {BUDGET:?}，实际只用 {elapsed:?} —— 次数或退避不对"
    );
    assert!(
        elapsed < Duration::from_secs(30),
        "该在次数用完之后就放弃，实际等了 {elapsed:?}"
    );

    // 事件序列：三次「重连中」（次数 1 / 2 / 3）+ 一次「失败」，中间**不许有** connected。
    let tail: Vec<(String, Option<u64>)> = states_of(&mut client, handle)
        .await
        .into_iter()
        .skip_while(|(state, _)| state != "reconnecting")
        .collect();
    let attempts: Vec<Option<u64>> = tail
        .iter()
        .filter(|(state, _)| state == "reconnecting")
        .map(|(_, attempt)| *attempt)
        .collect();
    let states: Vec<String> = tail.iter().map(|(state, _)| state.clone()).collect();
    assert_eq!(
        attempts,
        vec![Some(1), Some(2), Some(3)],
        "重连次数必须是 1 → 2 → 3：{states:?}"
    );
    assert_eq!(
        states.last().map(String::as_str),
        Some("failed"),
        "次数用完之后该是「失败」：{states:?}"
    );
    assert!(
        !tail.iter().any(|(state, _)| state == "connected"),
        "耗尽那条路上不该再出现过「已连接」：{states:?}"
    );

    // 可见的三处：probe 的状态、界面上那句话、托盘菜单上那一行。
    let failed = entry_of(&mut client, handle)
        .await
        .expect("失败之后它仍该在册");
    assert_eq!(
        failed.pointer("/state").and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        failed.pointer("/bind").and_then(Value::as_str),
        None,
        "失败之后没有端口在听，probe 不该还报一个监听地址：{failed}"
    );
    wait_text_contains(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{local_rule}\"] .tunnel-state"),
        "失败",
    )
    .await;
    let tray = client
        .call_tool("app_state", json!({ "probe": "tray" }))
        .await
        .expect("读不到 tray probe —— 它注册进 lib.rs 了吗？");
    let labels = tray
        .pointer("/tunnels")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let wanted = format!("{LOCAL_NAME} · 失败");
    if tray.pointer("/ready").and_then(Value::as_bool) == Some(true) {
        assert!(
            labels
                .iter()
                .any(|label| label.as_str() == Some(wanted.as_str())),
            "托盘菜单里该有一行写着「{wanted}」：{labels:?}"
        );
        eprintln!("托盘：{labels:?}");
    } else {
        // 没有托盘宿主的机器（只读 runtime dir / 容器 / 部分 Wayland 合成器）上，
        // 那几行文字没有去处 —— 显式说明，而不是把"没有托盘"当成"状态不可见"。
        eprintln!("这台机器上建不起托盘，跳过托盘那一处断言（probe 里 ready = false）");
    }

    // ── 7. 耗尽之后仍可手动重试（D12）────────────────────────────────────────
    let before_retry = states_of(&mut client, handle).await.len();
    let retried = client
        .invoke_command("tunnel_retry", Some(json!({ "handle": handle })))
        .await
        .expect("tunnel_retry 调不通");
    assert!(
        retried.pointer("/failure").map(Value::is_object) == Some(true),
        "服务端已经没了，重试仍然连不上：{retried}"
    );
    let after_retry: Vec<String> = states_of(&mut client, handle)
        .await
        .into_iter()
        .skip(before_retry)
        .map(|(state, _)| state)
        .collect();
    assert!(
        after_retry.contains(&"connecting".to_owned())
            && after_retry.contains(&"failed".to_owned()),
        "手动重试要**再走一遍**连接中 → 失败：{after_retry:?}"
    );
    eprintln!("手动重试：{after_retry:?}");

    // ── 8. 收尾 ──────────────────────────────────────────────────────────────
    client
        .invoke_command("tunnel_stop", Some(json!({ "handle": handle })))
        .await
        .expect("tunnel_stop 调不通");
    wait_tunnel_gone(&mut client, handle).await;
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
        "停掉之后只剩 app 自带的那个终端会话，且两张表必须相等：{sessions}"
    );
    let _ = client.invoke_command("vault_lock", None).await;
}
