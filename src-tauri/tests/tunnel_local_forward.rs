//! plan 0602 的**端到端**验收：真 app 上的本地转发 `-L`。
//!
//! 判据（ROADMAP 原文）=「转发端口**可访问远端服务**」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 规则能被界面看见 | 点 `.tab-new-tunnel` → `.tunnel-item[data-rule-id]` 出现 |
//! | 已连接 = **端口在监听** | probe `tunnels` 里该条带 `bind = "127.0.0.1:<规则端口>"` |
//! | **判据：转发端口可访问远端服务** | 连本地监听端口写一行再读回 —— 读到的就是写进去的那一行 |
//! | 目标只可能经对端到达 | 目标名是 RFC 2606 保留域 | 用例自行解析一次并断言**失败** |
//! | 走的是 `direct-tcpip` | 服务端的请求表 | 恰好 1 条，`host` / `port` 与规则一致 |
//! | 每条入站连接各开一条通道 | 同一端口再连一次 | 请求数变成 2 |
//! | 字节真的过了通道 | 服务端的 `relayed_bytes` | `> 0` |
//! | **端口被占用的报错可读** | 界面上点"打开"那条端口被占的规则 | `.tunnel-failure` 含该地址；probe 里**没有**它 |
//! | 停止后端口释放 | 点"停止" → 再连该端口 | 连接被拒 |
//! | 连接真的断了 | 服务端的 `connections_closed` | `≥ 1` |
//!
//! ## "远端服务"在这个测试里是什么
//!
//! 一个进程内的回声服务端，并通过测试服务端的**中继表**挂在一个只有对端认识的名字下
//! （`akasha-local-forward.invalid`，RFC 2606 保留域）。所以"字节到了回声服务"只可能
//! 经过那条 SSH 通道 —— 与 plan 0505 构造"只对跳板机可见"是同一种做法（无特权环境
//! 做不出真正的网络隔离，见 `docs/STATUS.md` 的「待验证」）。
//!
//! ## 数据从哪来
//!
//! 转发规则池**没有写路径的界面**（plan 0601 的非目标），所以规则由这条用例直接写库 ——
//! 同 `tunnel_state` / `ssh_jump` 的做法。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::time::{Duration, Instant};

use akasha_lib::ssh::testing::{Relay, ServerOptions, start};
use akasha_lib::store::pools::{forwards, hosts};
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
const PASSWORD: &str = "e2e-local-forward-password";
/// 池里那台主机的名字。
const HOST_NAME: &str = "e2e-forward-host";
/// 池里那条能转发通的规则名。
const FORWARD_NAME: &str = "e2e-forward-open";
/// 池里那条**端口被占用**的规则名（端口由本进程一直占着）。
const OCCUPIED_NAME: &str = "e2e-forward-occupied";
/// **只有对端认识**的名字（理由见文件头）。
const TARGET_NAME: &str = "akasha-local-forward.invalid";
/// 写进转发端口、再从它读回来的那串字节。
const PAYLOAD: &[u8] = b"akasha-local-forward-e2e\n";

/// 一个进程内的回声服务端 —— 它就是"远端服务"。
async fn start_echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("回声服务绑定失败");
    let addr = listener.local_addr().expect("取回声服务地址失败");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let (mut read, mut write) = socket.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    addr
}

/// 经本地监听端口走一次往返（写一行、读回同样多字节）。
async fn round_trip(port: u16) -> Vec<u8> {
    let mut client = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .expect("连本地监听端口超时")
    .expect("连本地监听端口失败");
    client.write_all(PAYLOAD).await.expect("写失败");
    let mut echoed = vec![0u8; PAYLOAD.len()];
    tokio::time::timeout(Duration::from_secs(10), client.read_exact(&mut echoed))
        .await
        .expect("等回声超时（转发没通）")
        .expect("读回声失败");
    echoed
}

/// 在库里把这次要用的东西摆好：一台主机 + 两条本地转发规则。
///
/// 返回 `(能转发通的那条规则, 端口被占的那条规则, 绑定端口)`。
fn seed(path: &Path, ssh_port: u16, echo_port: u16, occupied_port: u16) -> (i64, i64, u16) {
    let conn = open_vault(path);

    // 先清干净（重跑）：主机行与规则行都按名字清。
    forget(&conn, HOST_NAME, "127.0.0.1", ssh_port);
    for row in forwards::forwards(&conn).unwrap() {
        if row.name == FORWARD_NAME || row.name == OCCUPIED_NAME {
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
    let rule = |name: &str, bind_port: u16| forwards::NewForward {
        name: name.to_owned(),
        direction: forwards::Direction::Local,
        bind_host: "127.0.0.1".to_owned(),
        bind_port,
        // 目标地址原样进库：它由**对端**解析（本机解析不出来，见文件头）。
        target_host: Some(TARGET_NAME.to_owned()),
        target_port: Some(echo_port),
        host_id,
        autostart: false,
    };

    let forward_rule = forwards::insert_forward(&conn, &rule(FORWARD_NAME, bind_port)).unwrap();
    let occupied_rule =
        forwards::insert_forward(&conn, &rule(OCCUPIED_NAME, occupied_port)).unwrap();

    (forward_rule, occupied_rule, bind_port)
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
async fn a_forwarded_port_reaches_a_service_only_the_remote_side_can_name() {
    if support::skip_unless_e2e() {
        return;
    }

    // ── 1. 回声服务 + SSH 服务端（中继表是那个名字唯一的存在方式）──────────────
    let echo = start_echo().await;
    let server = start(ServerOptions {
        password: Some(PASSWORD.to_owned()),
        relay: vec![Relay {
            host: TARGET_NAME.to_owned(),
            port: echo.port(),
            to: echo,
        }],
        ..ServerOptions::default()
    })
    .await;
    eprintln!(
        "服务端：127.0.0.1:{}（回声服务在 {}）",
        server.addr.port(),
        echo.port()
    );

    // 构造前提：这个名字在本机解析不出来。没有这一条，下面的断言分不清
    // "字节经了隧道"与"这个名字其实能直连"。
    assert!(
        (TARGET_NAME, echo.port()).to_socket_addrs().is_err(),
        "构造前提破了：{TARGET_NAME} 在本机居然解析得出来"
    );

    // ── 2. 库 + 种子数据 + 解锁 ──────────────────────────────────────────────
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };
    // **本进程**一直占着这个端口：app 在另一个进程里绑同一个地址会拿到 EADDRINUSE。
    // 它在整个用例期间必须活着 —— drop 掉就等于把端口还回去了。
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").expect("占用一个端口失败");
    let occupied_port = occupied.local_addr().expect("取占用端口失败").port();
    let (forward_rule, occupied_rule, bind_port) =
        seed(&path, server.addr.port(), echo.port(), occupied_port);
    unlock(&mut client).await;

    // ── 3. 界面：打开隧道面板 → 打开那条规则 → 答完提示 ──────────────────────
    open_tunnel_panel(&mut client).await;
    wait_js(
        &mut client,
        &format!("!!document.querySelector('.tunnel-item[data-rule-id=\"{forward_rule}\"]')"),
        10_000,
        "面板列出了池里的规则",
    )
    .await;
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{forward_rule}\"] .tunnel-open"),
        "打开这条隧道",
    )
    .await;
    let (handle, asked) =
        connect_tunnel_through_prompts(&mut client, forward_rule, &server.fingerprint, PASSWORD)
            .await;
    assert!(handle > 0, "probe 里必须有 handle");
    eprintln!("隧道已连接：handle={handle}，问到过 {asked:?}");

    // 从 plan 0602 起「已连接」意味着**端口在监听** —— probe 里必须看得见它。
    let want = format!("127.0.0.1:{bind_port}");
    assert_eq!(
        bound_of(&mut client, forward_rule).await.as_deref(),
        Some(want.as_str()),
        "已连接的隧道必须报出它监听的端口"
    );

    // ── 4. 判据：转发端口可访问远端服务 ──────────────────────────────────────
    assert_eq!(
        round_trip(bind_port).await,
        PAYLOAD,
        "转发端口读回的不是写进去的那一行"
    );

    // 对端那一半：它被要求连的**正是那个只有它认识的名字**，而且字节真的搬过去了。
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
    assert_eq!(seen.direct_tcpip[0].port, u32::from(echo.port()));
    assert!(
        server.shared.relayed_bytes() > 0,
        "中继应当搬过字节（对端那一侧的计数）"
    );
    eprintln!("对端：{:?}", seen.direct_tcpip);

    // ── 5. 每条入站连接各开一条通道（不是"一条通道用到底"）───────────────────
    assert_eq!(round_trip(bind_port).await, PAYLOAD, "第二条连接也该通");
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

    // ── 6. 端口被占用的那条：界面上有一句可读的失败，而且它**没有**登记 ───────
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{occupied_rule}\"] .tunnel-open"),
        "打开那条端口被占的规则",
    )
    .await;
    wait_text_contains(&mut client, ".tunnel-failure", &occupied_port.to_string()).await;
    let shown = text_of(&mut client, ".tunnel-failure").await;
    assert!(
        shown.contains("绑定失败"),
        "界面要说清是**端口**没拿到，而不是一句泛泛的失败：{shown:?}"
    );
    assert!(
        shown.contains(&format!("127.0.0.1:{occupied_port}")),
        "同一条失败里也要能看出是哪个地址（这一条是 127.0.0.1:{occupied_port}）：{shown:?}"
    );
    eprintln!("端口被占用的那条：{shown}");
    assert!(
        !tunnel_entries(&mut client).await.iter().any(|entry| {
            entry.pointer("/ruleId").and_then(Value::as_i64) == Some(occupied_rule)
        }),
        "端口没拿到 = 根本没登记成，probe 里不该有它：{:?}",
        tunnel_entries(&mut client).await
    );

    // ── 7. 停止：端口释放 + 那条连接断开 ─────────────────────────────────────
    click(
        &mut client,
        &format!(".tunnel-item[data-rule-id=\"{forward_rule}\"] .tunnel-stop"),
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
