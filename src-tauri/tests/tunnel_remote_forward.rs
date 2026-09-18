//! plan 0604 的**端到端**验收：真 app 上的远程转发 `-R`。
//!
//! 判据（ROADMAP 原文）=「远端监听端口**可回连到本机服务**」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 规则能被界面看见 | 点 `.tab-new-tunnel` → `.tunnel-item[data-rule-id]` 出现 |
//! | 已连接 = **服务端那边端口在听** | probe `tunnels` 里该条带 `bind = "127.0.0.1:<规则端口>"` |
//! | 服务端真的收到了请求 | 服务端的 `tcpip_forward` 记录：地址/端口与规则一致且被认下 |
//! | **判据：远端监听端口可回连到本机服务** | 真实客户端（`curl`）连那个端口 → 本机服务答的响应体回来了 |
//! | 通道由**服务端发起** | 服务端的 `forwarded_tcpip_accepted ≥ 1` 与 `relayed_bytes > 0` |
//! | 本机服务真的被连过 | 本机 HTTP 服务的请求计数 `≥ 1`（它只听 `127.0.0.1`） |
//! | 每条入站连接各一条通道 | 同一端口再请求一次 → `forwarded_tcpip_accepted` 变成 2 |
//! | 本机目标不可达被**拒** | 服务端看到 `ConnectFailed`，而 `accepted` 计数**没变** |
//! | 停止即撤销 | 点"停止" → 远端端口不再接受连接；服务端收到 `cancel-tcpip-forward` |
//!
//! ## 两侧都在同一个进程里，那"远端"是什么
//!
//! 无特权环境做不出网络隔离（同 plan 0602 / 0603 的说明）。区分"远端"与"本机"的是
//! **谁在听**：远端监听端口由**测试服务端**在 `tcpip_forward` 里绑（它扮演 sshd），
//! 本机服务由用例自己绑。字节因此只可能这样走：
//! 用例（`curl`）→ 服务端的监听端口 → `forwarded-tcpip` 通道 → **真 app** → 本机 HTTP 服务。
//! `curl` 只对着服务端那个端口说话，本机服务只认那条通道来的连接 —— 两半的计数是这条推理的证据。
//!
//! ⚠️ 与 plan 0602 / 0603 的方向正好相反：那两条判据看的是服务端的 `direct_tcpip`
//! （**它**被要求连出去），这一条看的是 `forwarded_tcpip`（**它**主动开通道给我们）。
//! 所以这条用例**不需要中继表**：`ServerOptions::relay` 在 `-R` 这条路上用不到。
//!
//! ## 数据从哪来
//!
//! 转发规则池**没有写路径的界面**（plan 0601 的非目标），所以规则由这条用例直接写库 ——
//! 同 `tunnel_state` / `tunnel_local_forward` / `tunnel_dynamic_forward` 的做法。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use akasha_lib::ssh::testing::{ServerOptions, start};
use akasha_store::pools::{forwards, hosts};
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, USER, click, connect_and_prepare, connect_tunnel_through_prompts,
    connect_tunnel_until, forget, free_port, open_tunnel_panel, open_vault, text_of,
    tunnel_entries, unlock, wait_js, wait_text_contains, wait_tunnel_gone,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use victauri_test::VictauriClient;

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-remote-forward-password";
/// 池里那台主机的名字。
const HOST_NAME: &str = "e2e-remote-host";
/// 池里那条能转发通的规则名。
const FORWARD_NAME: &str = "e2e-remote-open";
/// 池里那条**本机目标没人听**的规则名（反例，见文件头）。
const DEAD_NAME: &str = "e2e-remote-dead";
/// 池里那条**远端端口拿不到**的规则名（服务端那一侧被占着）。
const TAKEN_NAME: &str = "e2e-remote-taken";
/// 本机服务会写进响应体的那串字节（`curl` 取回来的就是它）。
const BODY: &str = "akasha-remote-forward-e2e\n";
/// `curl` 的等待上限。
const CURL_TIMEOUT_SECS: &str = "20";

/// 一个进程内的 HTTP 服务端 —— 它就是"**本机**上的服务"，外加一个请求计数。
///
/// 为什么要计数：判据是"远端端口回连到本机服务"，而"响应体回来了"也可能是别的东西答的。
/// 计数说明**这个**服务端真的被连过。
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

/// 经**远端**监听端口取一次本机服务（**真实客户端**）。
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

/// 挑一个**没人听**的端口：先占住再放掉。
///
/// 反例要的正是"连不上"这件事，所以不能拿一个还握在手里的监听端口。
fn dead_port() -> u16 {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("取一个空闲端口失败");
    held.local_addr().expect("取地址失败").port()
}

/// 在库里把这次要用的东西摆好：一台主机 + 两条 `remote` 规则。
///
/// `local_port` 是**本机服务**的端口，`taken_port` 是一个**已经被占着的**端口
/// （用例自己握着它，所以服务端一定绑不上）。返回
/// `(能转发通的那条, 目标没人听的, 远端端口拿不到的, 服务端那一侧的绑定端口)`。
///
/// ⚠️ `remote` 的 `bind_host` / `bind_port` 是**服务端**那一侧的监听地址，
/// `target_host` / `target_port` 才是**本机**服务 —— 库里从 plan 0601 起的那条 `CHECK`
/// 保证非 `dynamic` 的方向一定有目标。
fn seed(path: &Path, ssh_port: u16, local_port: u16, taken_port: u16) -> (i64, i64, i64, u16) {
    let conn = open_vault(path);

    // 先清干净（重跑）：主机行与规则行都按名字清。
    forget(&conn, HOST_NAME, "127.0.0.1", ssh_port);
    for row in forwards::forwards(&conn).unwrap() {
        if row.name == FORWARD_NAME || row.name == DEAD_NAME || row.name == TAKEN_NAME {
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
    let bind_port = free_port();
    let rule = |name: &str, bind_port: u16, target_port: u16| forwards::NewForward {
        name: name.to_owned(),
        direction: forwards::Direction::Remote,
        bind_host: "127.0.0.1".to_owned(),
        bind_port,
        target_host: Some("127.0.0.1".to_owned()),
        target_port: Some(target_port),
        host_id,
        autostart: false,
    };

    let open_rule =
        forwards::insert_forward(&conn, &rule(FORWARD_NAME, bind_port, local_port)).unwrap();
    let dead_rule =
        forwards::insert_forward(&conn, &rule(DEAD_NAME, free_port(), dead_port())).unwrap();
    // 远端端口拿不到的那条：服务端会去绑 `taken_port`，而它被用例自己占着。
    let taken_rule =
        forwards::insert_forward(&conn, &rule(TAKEN_NAME, taken_port, local_port)).unwrap();

    (open_rule, dead_rule, taken_rule, bind_port)
}

/// probe 里某条规则那一行现在报的监听地址。
async fn bound_of(client: &mut VictauriClient, rule_id: i64) -> Option<String> {
    tunnel_entries(client)
        .await
        .into_iter()
        .find(|entry| entry.pointer("/ruleId").and_then(Value::as_i64) == Some(rule_id))
        .and_then(|entry| {
            entry
                .pointer("/bind")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
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

/// 等某个远端端口不再接受连接。
async fn wait_refused(port: u16, what: &str) {
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        // 监听撤掉之后连过去是"连接被拒"，不是"超时"。
        let refused = tokio::time::timeout(
            Duration::from_secs(2),
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        .map(|result| result.is_err())
        .unwrap_or(false);
        if refused {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}：端口 {port} 还在接受连接"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remote_port_forwards_back_to_a_service_on_this_side() {
    if support::skip_unless_e2e() {
        return;
    }

    // `curl` 是这条判据的客户端：判据说的是"远端监听端口可回连到本机服务"，
    // 而那一步要一个现成的 TCP/HTTP 客户端来走。没有它就悄悄跳过，等于让这条判据没有证据。
    assert!(
        Command::new("curl")
            .arg("--version")
            .output()
            .is_ok_and(|out| out.status.success()),
        "这条用例需要 curl：判据的客户端必须是现成的客户端"
    );

    // ── 1. 本机 HTTP 服务 + SSH 服务端（`-R` 不需要中继表）──────────────────────
    let (http, hits) = start_http().await;
    let server = start(ServerOptions::password(PASSWORD)).await;
    eprintln!(
        "服务端：127.0.0.1:{}（本机 HTTP 服务在 127.0.0.1:{}）",
        server.addr.port(),
        http.port()
    );

    // ── 2. 库 + 种子数据 + 解锁 ──────────────────────────────────────────────
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };
    // 第三条规则要的"服务端绑不上的端口"：用例自己握着它，服务端因此一定绑不上。
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("占住一个端口失败");
    let taken_port = taken.local_addr().expect("取地址失败").port();
    // 三条规则的本机目标：一条指向 HTTP 服务，另两条的目标与绑定端口按各自的反例来定。
    let (open_id, dead_id, taken_id, bind_port) =
        seed(&path, server.addr.port(), http.port(), taken_port);
    unlock(&mut client).await;

    // ── 3. 界面：打开隧道面板 → 打开那条规则 → 答完提示 ──────────────────────
    open_tunnel_panel(&mut client).await;
    let handle = open_tunnel_for(&mut client, open_id, &server.fingerprint).await;
    eprintln!("隧道已连接：handle={handle}");

    // "已连接" = **服务端那边**端口在听。探针里的 `bind` 报的就是服务端那一侧。
    let want = format!("127.0.0.1:{bind_port}");
    assert_eq!(
        bound_of(&mut client, open_id).await.as_deref(),
        Some(want.as_str()),
        "已连接的隧道必须报出服务端实际监听的地址"
    );

    // 服务端那一半：它真的收到了请求，而且认下的端口就是规则里那个。
    let seen = support::observed(&server);
    assert_eq!(seen.forward_requests.len(), 1, "恰好一条请求：{seen:?}");
    assert_eq!(seen.forward_requests[0].address, "127.0.0.1");
    assert_eq!(seen.forward_requests[0].port, u32::from(bind_port));
    assert!(seen.forward_requests[0].accepted, "这条请求该被认下");
    assert_eq!(seen.forward_requests[0].bound_port, bind_port);

    // ── 4. 判据：远端监听端口可回连到本机服务（现成客户端）──────────────────
    let (ok, stdout, stderr) = tokio::task::spawn_blocking(move || curl(bind_port))
        .await
        .expect("curl 那条阻塞任务没回话");
    assert!(ok, "curl 经远端端口取本机服务失败（stderr={stderr:?}）");
    assert_eq!(stdout, BODY, "curl 取回来的不是本机服务写的那一串字节");
    eprintln!("curl 取回：{stdout:?}");

    // 通道是**服务端发起**的，字节也真的搬过去了；本机服务确实被连过。
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        if support::observed(&server).forwarded_tcpip_accepted == 1
            && hits.load(Ordering::SeqCst) >= 1
            && server.shared.relayed_bytes() > 0
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "服务端发起通道 / 本机服务被连 / 字节过通道，这三件没全发生：{:?}，hits={}",
            support::observed(&server),
            hits.load(Ordering::SeqCst)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── 5. 每条入站连接各开一条通道（不是"一条通道用到底"）───────────────────
    let (ok, stdout, stderr) = tokio::task::spawn_blocking(move || curl(bind_port))
        .await
        .expect("curl 那条阻塞任务没回话");
    assert!(ok, "第二次请求也该通（stderr={stderr:?}）");
    assert_eq!(stdout, BODY);
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        if support::observed(&server).forwarded_tcpip_accepted == 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "第二条入站连接没开出第二条通道：{:?}",
            support::observed(&server)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── 6. 反例：本机目标没人听 → 通道**被拒**（不是"接受了又断"）─────────────
    let dead_handle = open_tunnel_for(&mut client, dead_id, &server.fingerprint).await;
    let dead_port = support::observed(&server)
        .forward_requests
        .iter()
        .find(|request| request.bound_port != bind_port && request.accepted)
        .map(|request| request.bound_port)
        .expect("第二条规则也该让服务端绑上一个端口");
    let accepted_before = support::observed(&server).forwarded_tcpip_accepted;
    // 连一次就走：那条通道应当**根本没被接受**。
    let _ = TcpStream::connect(("127.0.0.1", dead_port)).await;

    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        let rejected = support::observed(&server).forwarded_tcpip_rejected;
        if !rejected.is_empty() {
            assert!(
                rejected
                    .iter()
                    .any(|reason| reason.contains("ConnectFailed")),
                "本机服务不可达时对端必须拒这条通道（ConnectFailed），实际：{rejected:?}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "服务端没记下任何被拒的通道：{:?}",
            support::observed(&server)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        support::observed(&server).forwarded_tcpip_accepted,
        accepted_before,
        "被拒的那条不许先接受再关 —— 对端的客户端会看到一条连上了就断的连接"
    );

    // ── 7. 反例：远端端口在服务端那一侧被占着 → 落到 `失败`，而不是"连不上" ─────
    //
    // ⚠️ 这一条与上面两条不同：请求失败发生在**连接建起来之后**（那个端口在服务端），
    // 所以那条隧道**仍然登记着**（可重试），而且界面上要看得到是哪一步没成。
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{taken_id}\"] .tunnel-open"),
        "打开那条远端端口被占着的规则",
    )
    .await;
    let taken_handle = connect_tunnel_until(
        &mut client,
        taken_id,
        &server.fingerprint,
        PASSWORD,
        "failed",
    )
    .await
    .0;
    assert!(
        taken_handle > 0,
        "远端监听失败也要登记成一条隧道（它可重试）"
    );
    wait_text_contains(&mut client, ".tunnel-failure", "远端监听").await;
    let shown = text_of(&mut client, ".tunnel-failure").await;
    assert!(
        shown.contains("没拿到"),
        "失败里要说清是「没拿到」（而不是「连不上」）：{shown:?}"
    );
    assert!(
        shown.contains(&taken_port.to_string()),
        "失败里要能看出是**哪个端口**没拿到：{shown:?}"
    );
    eprintln!("远端端口拿不到那条：{shown}");
    assert!(
        support::tunnel_events(&mut client)
            .await
            .iter()
            .any(
                |event| event.pointer("/handle").and_then(Value::as_u64) == Some(taken_handle)
                    && event.pointer("/state").and_then(Value::as_str) == Some("failed")
            ),
        "失败必须发事件（托盘与界面据此可见）：{:?}",
        support::tunnel_events(&mut client).await
    );
    let seen = support::observed(&server);
    let refused = seen
        .forward_requests
        .iter()
        .find(|request| request.port == u32::from(taken_port))
        .expect("服务端该收到那条请求");
    assert!(
        !refused.accepted,
        "那个端口被占着，服务端必须拒绝：{refused:?}"
    );

    // ── 8. 停止：远端端口释放 + 撤销请求到达 + 连接断开 ─────────────────────
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{open_id}\"] .tunnel-stop"),
        "停止这条隧道",
    )
    .await;
    wait_tunnel_gone(&mut client, handle).await;
    wait_refused(bind_port, "停止之后远端端口必须释放").await;

    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        let cancels = support::observed(&server).forward_cancellations;
        if !cancels.is_empty() {
            assert_eq!(
                cancels.first().map(String::as_str),
                Some(want.as_str()),
                "撤销请求要用规则里那个地址与端口：{cancels:?}"
            );
            eprintln!("停止：服务端收到撤销 {cancels:?}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "服务端没收到 cancel-tcpip-forward：{:?}",
            support::observed(&server)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── 9. 收尾：停掉那两条反例，确认两张表都归零 ─────────────────────────────
    for (rule, handle) in [(dead_id, dead_handle), (taken_id, taken_handle)] {
        click(
            &mut client,
            &format!(".tunnel-item[data-rule-id=\"{rule}\"] .tunnel-stop"),
            "停止反例那条隧道",
        )
        .await;
        wait_tunnel_gone(&mut client, handle).await;
    }

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

    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        let closed = support::observed(&server).connections_closed;
        if closed >= 2 {
            eprintln!("停止：服务端看到 {closed} 条连接断开");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "隧道停掉了，服务端却还看到连接挂着（只有 {closed} 条断开）"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── 10. 收尾：锁库 ───────────────────────────────────────────────────────
    let _ = client.invoke_command("vault_lock", None).await;
}
