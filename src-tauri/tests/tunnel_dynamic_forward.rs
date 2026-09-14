//! plan 0603 的**端到端**验收：真 app 上的动态转发 `-D`（SOCKS5）。
//!
//! 判据（ROADMAP 原文）=「配置 SOCKS5 代理后**能访问远端网络**」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 规则能被界面看见 | 点 `.tab-new-tunnel` → `.tunnel-item[data-rule-id]` 出现 |
//! | 已连接 = **端口在监听** | probe `tunnels` 里该条带 `bind = "127.0.0.1:<规则端口>"` |
//! | **判据：配置 SOCKS5 代理后能访问远端网络** | `curl --socks5-hostname` 指向该端口取回远端服务的响应体 |
//! | 目标由**客户端**说 | 服务端的 `direct_tcpip` 请求里 `host` 是 curl 在握手里给的那个名字 |
//! | 目标只可能经对端到达 | 目标名是 RFC 2606 保留域，用例自行解析一次并断言**失败** |
//! | 每条入站连接各开一条通道 | 同一端口再 curl 一次 → 请求数变成 2 |
//! | **非回环绑定被拒** | 库里那条 `0.0.0.0` 的规则：界面上说清哪条地址被拒；probe 里**没有**它 |
//! | 停止后端口释放 | 点"停止" → 再连该端口被拒 |
//! | 连接真的断了 | 服务端的 `connections_closed` | `≥ 1` |
//!
//! ## 为什么客户端是 `curl`
//!
//! 判据说的是"配置 SOCKS5 代理之后能访问远端网络"，而配置代理的是**浏览器与 curl 这类
//! 现成客户端**。库内用例（`akasha-ssh/tests/socks5_forward.rs`）用的是手写客户端，
//! 它能验协议细节，却证不了"现成客户端认这个服务端"。这一条补的正是那一半 ——
//! 所以 `curl` 是本用例的**前置**：没有它这条判据就没有证据（不能悄悄跳过）。
//!
//! ## "远端网络里的服务"在这个测试里是什么
//!
//! 一个进程内的 HTTP 服务端，通过测试服务端的**中继表**挂在一个只有对端认识的名字下
//! （`akasha-dynamic-forward.invalid`，RFC 2606 保留域）。所以"curl 拿到了那个响应体"
//! 只可能经过那条 SSH 通道 —— 与 plan 0602 是同一种构造（无特权环境做不出真正的网络隔离，
//! 见 `docs/STATUS.md` 的「待验证」）。
//!
//! ## 数据从哪来
//!
//! 转发规则池**没有写路径的界面**（plan 0601 的非目标），所以规则由这条用例直接写库 ——
//! 同 `tunnel_state` / `tunnel_local_forward` 的做法。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use akasha_ssh::testing::{Relay, ServerOptions, start};
use akasha_store::pools::{forwards, hosts};
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, USER, click, connect_and_prepare, connect_tunnel_through_prompts, forget,
    free_port, open_tunnel_panel, open_vault, text_of, tunnel_entries, unlock, wait_js,
    wait_text_contains, wait_tunnel_gone,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use victauri_test::VictauriClient;

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-dynamic-forward-password";
/// 池里那台主机的名字。
const HOST_NAME: &str = "e2e-socks5-host";
/// 池里那条能转发通的规则名。
const FORWARD_NAME: &str = "e2e-socks5-open";
/// 池里那条**绑非回环地址**的规则名（安全项，见文件头）。
const EXPOSED_NAME: &str = "e2e-socks5-exposed";
/// **只有对端认识**的名字（理由见文件头）。
const TARGET_NAME: &str = "akasha-dynamic-forward.invalid";
/// 远端服务会写进响应体的那串字节（curl 取回来的就是它）。
const BODY: &str = "akasha-dynamic-forward-e2e\n";
/// `curl` 的等待上限（SOCKS5 那一跳不该慢，慢就是坏了）。
const CURL_TIMEOUT_SECS: &str = "20";

/// 一个进程内的 HTTP 服务端 —— 它就是"远端网络里的一个服务"。
///
/// 为什么不是回声服务：判据要用 `curl` 来验，而 curl 需要一个说得通的 HTTP 响应才会
/// 退出码 0（对着一个回声服务它只会报协议错）。这里读掉请求头就回一个固定响应 ——
/// 用例不关心 curl 请求了什么，只关心**响应体是从远端服务那一侧回来的**。
async fn start_http() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("HTTP 服务绑定失败");
    let addr = listener.local_addr().expect("取 HTTP 服务地址失败");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
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
    addr
}

/// 经 SOCKS5 端口取一次远端服务（**真实客户端**）。
///
/// 用 `--socks5-hostname`：域名交给代理去解析 —— 那正是动态转发的要点（本机解析就等于
/// 绕开跳板机），而 `--socks5` 会先在本地解析、拿不到 IP 就直接失败。
fn curl_socks5(port: u16, host: &str, target_port: u16) -> (bool, String, String) {
    let output = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--max-time")
        .arg(CURL_TIMEOUT_SECS)
        .arg("--socks5-hostname")
        .arg(format!("127.0.0.1:{port}"))
        .arg(format!("http://{host}:{target_port}/probe"))
        .output()
        .expect("curl 起不来");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// 一个进程内的 SOCKS5 客户端（**判据那一半要第三方**，这一条只做反例）。
///
/// 反例只在这里做一次：`REP` 的分类由库内用例穷尽覆盖（`socks5_forward`），而 curl 的
/// 报错文案逐版本会变，拿它当判据是那种"改一次上游就红"的断言。
async fn socks5_rep(port: u16, host: &str, target_port: u16) -> u8 {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("连 SOCKS5 端口失败");
    stream
        .write_all(&[0x05, 0x01, 0x00])
        .await
        .expect("写问候失败");
    let mut chosen = [0u8; 2];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut chosen))
        .await
        .expect("等问候回复超时")
        .expect("读问候回复失败");
    assert_eq!(chosen, [0x05, 0x00], "服务端必须选中无认证");

    let mut request = vec![0x05, 0x01, 0x00, 0x03, u8::try_from(host.len()).unwrap()];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&target_port.to_be_bytes());
    stream.write_all(&request).await.expect("写请求失败");

    let mut head = [0u8; 10];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut head))
        .await
        .expect("等 REP 超时")
        .expect("读 REP 失败");
    assert_eq!(head[0], 0x05, "REP 的版本必须是 5");
    assert_eq!(head[3], 0x01, "占位的 BND.ADDR 是 IPv4 形态");
    head[1]
}

/// 在库里把这次要用的东西摆好：一台主机 + 两条动态转发规则。
///
/// 返回 `(能转发通的那条规则, 绑非回环地址的那条规则, 绑定端口)`。
fn seed(path: &Path, ssh_port: u16) -> (i64, i64, u16) {
    let conn = open_vault(path);

    // 先清干净（重跑）：主机行与规则行都按名字清。
    forget(&conn, HOST_NAME, "127.0.0.1", ssh_port);
    for row in forwards::forwards(&conn).unwrap() {
        if row.name == FORWARD_NAME || row.name == EXPOSED_NAME {
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
    // `dynamic` **没有目标** —— 这是库里的 `CHECK` 拦着的不变量，所以这里如实传 `None`。
    let rule = |name: &str, bind_host: &str, bind_port: u16| forwards::NewForward {
        name: name.to_owned(),
        direction: forwards::Direction::Dynamic,
        bind_host: bind_host.to_owned(),
        bind_port,
        target_host: None,
        target_port: None,
        host_id,
        autostart: false,
    };

    let open_rule =
        forwards::insert_forward(&conn, &rule(FORWARD_NAME, "127.0.0.1", bind_port)).unwrap();
    // 非回环那条：`0.0.0.0` 是无认证 SOCKS5 **不许**绑的地址（plan 0603 的安全项）。
    let exposed_rule =
        forwards::insert_forward(&conn, &rule(EXPOSED_NAME, "0.0.0.0", free_port())).unwrap();

    (open_rule, exposed_rule, bind_port)
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_socks5_port_forwards_whatever_the_client_names() {
    if support::skip_unless_e2e() {
        return;
    }

    // `curl` 是这条判据的前置：判据说的是"配置 SOCKS5 代理之后能访问远端网络"，
    // 而配代理的是现成客户端。没有它就悄悄跳过，等于让这条判据没有证据。
    assert!(
        Command::new("curl")
            .arg("--version")
            .output()
            .is_ok_and(|out| out.status.success()),
        "这条用例需要 curl：判据的客户端必须是现成的 SOCKS5 客户端"
    );

    // ── 1. HTTP 服务 + SSH 服务端（中继表是那个名字唯一的存在方式）──────────────
    let http = start_http().await;
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
        "服务端：127.0.0.1:{}（HTTP 服务在 {}）",
        server.addr.port(),
        http.port()
    );
    // 构造前提：这个名字在本机解析不出来。没有这一条，下面的断言分不清
    // "字节经了隧道"与"这个名字其实能直连"。
    assert!(
        (TARGET_NAME, http.port()).to_socket_addrs().is_err(),
        "构造前提破了：{TARGET_NAME} 在本机居然解析得出来"
    );

    // ── 2. 库 + 种子数据 + 解锁 ──────────────────────────────────────────────
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };
    let (open_rule, exposed_rule, bind_port) = seed(&path, server.addr.port());
    unlock(&mut client).await;

    // ── 3. 界面：打开隧道面板 → 打开那条规则 → 答完提示 ──────────────────────
    open_tunnel_panel(&mut client).await;
    wait_js(
        &mut client,
        &format!("!!document.querySelector('.tunnel-item[data-rule-id=\"{open_rule}\"]')"),
        10_000,
        "面板列出了池里的规则",
    )
    .await;
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{open_rule}\"] .tunnel-open"),
        "打开这条隧道",
    )
    .await;
    let (handle, asked) =
        connect_tunnel_through_prompts(&mut client, open_rule, &server.fingerprint, PASSWORD).await;
    assert!(handle > 0, "probe 里必须有 handle");
    eprintln!("隧道已连接：handle={handle}，问到过 {asked:?}");

    let want = format!("127.0.0.1:{bind_port}");
    assert_eq!(
        bound_of(&mut client, open_rule).await.as_deref(),
        Some(want.as_str()),
        "已连接的隧道必须报出它监听的端口"
    );

    // ── 4. 判据：配置 SOCKS5 代理后能访问远端网络（现成客户端）────────────────
    let (ok, stdout, stderr) =
        tokio::task::spawn_blocking(move || curl_socks5(bind_port, TARGET_NAME, http.port()))
            .await
            .expect("curl 那条阻塞任务没回话");
    assert!(ok, "curl 经 SOCKS5 取远端服务失败（stderr={stderr:?}）");
    assert_eq!(stdout, BODY, "curl 取回来的不是远端服务写的那一串字节");
    eprintln!("curl --socks5-hostname 取回：{stdout:?}");

    // 对端那一半：它被要求连的**正是 curl 在握手里说的那个名字**，字节也真的搬过去了。
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        if !support::observed(&server).direct_tcpip.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "服务端没记下任何 direct-tcpip 请求"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let seen = support::observed(&server);
    assert_eq!(
        seen.direct_tcpip.len(),
        1,
        "对端应当恰好被要求转发一次，实际：{:?}",
        seen.direct_tcpip
    );
    assert_eq!(seen.direct_tcpip[0].host, TARGET_NAME);
    assert_eq!(seen.direct_tcpip[0].port, u32::from(http.port()));
    assert!(
        server.shared.relayed_bytes() > 0,
        "中继应当搬过字节（对端那一侧的计数）"
    );

    // ── 5. 每条入站连接各开一条通道 + 目标由客户端逐条说 ──────────────────────
    let (ok, stdout, stderr) =
        tokio::task::spawn_blocking(move || curl_socks5(bind_port, TARGET_NAME, http.port()))
            .await
            .expect("curl 那条阻塞任务没回话");
    assert!(ok, "第二次 curl 也该通（stderr={stderr:?}）");
    assert_eq!(stdout, BODY);
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        if support::observed(&server).direct_tcpip.len() == 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "第二条入站连接没开出第二条通道：{:?}",
            support::observed(&server).direct_tcpip
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 失败分类真的到了客户端：中继表里没有的名字 → `REP 0x02`（不是 0x01）。
    assert_eq!(
        socks5_rep(bind_port, "akasha-dynamic-forward-unknown.invalid", 9).await,
        0x02,
        "对端拒绝开通道时必须回「规则不允许」0x02"
    );

    // ── 6. 安全项：绑 `0.0.0.0` 的那条**根本起不来** ──────────────────────────
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{exposed_rule}\"] .tunnel-open"),
        "打开那条绑 0.0.0.0 的规则",
    )
    .await;
    wait_text_contains(&mut client, ".tunnel-failure", "只允许绑回环地址").await;
    let shown = text_of(&mut client, ".tunnel-failure").await;
    assert!(
        shown.contains("0.0.0.0"),
        "失败里要能看出是哪条地址（这一条是 0.0.0.0）：{shown:?}"
    );
    eprintln!("非回环那条：{shown}");
    assert!(
        !tunnel_entries(&mut client)
            .await
            .iter()
            .any(|entry| entry.pointer("/ruleId").and_then(Value::as_i64) == Some(exposed_rule)),
        "地址不许绑 = 根本没登记成，probe 里不该有它：{:?}",
        tunnel_entries(&mut client).await
    );

    // ── 7. 停止：端口释放 + 那条连接断开 ─────────────────────────────────────
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{open_rule}\"] .tunnel-stop"),
        "停止这条隧道",
    )
    .await;
    wait_tunnel_gone(&mut client, handle).await;

    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        // 监听撤掉之后连过去是"连接被拒"，不是"超时"。
        let refused = tokio::time::timeout(
            Duration::from_secs(2),
            TcpStream::connect(("127.0.0.1", bind_port)),
        )
        .await
        .map(|result| result.is_err())
        .unwrap_or(false);
        if refused {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "隧道停掉了，端口 {bind_port} 却还在接受连接"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
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

    // ── 8. 收尾：锁库 ────────────────────────────────────────────────────────
    let _ = client.invoke_command("vault_lock", None).await;
}
