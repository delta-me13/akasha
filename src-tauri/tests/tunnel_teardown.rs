//! plan 0606 的**端到端**验收：**关闭一条转发 `Session`**（面板「停止」）。
//!
//! 判据（ROADMAP 原文）=「关闭转发 `Session` 后**连接数与重连任务数都归零**」。这条用例逐条
//! 盯着它，而两个数都从 `residue` 探针读（`AGENTS.md` §7：观察后端状态用 probe 读，
//! 不靠 grep 日志反推）：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 连接数跟着隧道走 | 两条隧道都连上 → `residue.sshConnections = 2`；关掉一条 → `1`；关掉两条 → `0` |
//! | 重连任务数跟着隧道走 | 同上，`residue.watchTasks` 与连接数同步 |
//! | **连接真的断了** | 对端（测试 SSH 服务端）**自己**看不到那条会话了 —— 本进程说 0 有可能只是我们丢了自己的句柄 |
//! | 关掉一条不动另一条 | 关掉 `-R` 之后 `-L` 的端口仍然 `curl` 得通 |
//! | `-R` 的收尾含**撤销远端监听** | 关掉之后服务端那个端口不再接受连接 |
//! | 关闭是幂等的 | 对同一个句柄再 `tunnel_stop` 一次：`Ok`，两个计数仍是 0 |
//! | **在途的尝试也被中止** | 规则指向一台"接了 TCP 就不再说话"的进程：点「重试」再点「停止」→ 那个 socket 在 3 秒内被对端读到 EOF（不中止的话要等握手超时） |
//!
//! ## 那两个数为什么不能从实体表来
//!
//! `tunnels` probe 数的是注册表里的实体，而关闭命令**自己**就会把实体摘掉 —— "表里没了"
//! 只是那条命令的效果，不是"连接断了"的证据。`residue` 数的是资源本身：
//! `SshConnection` 对象（归转发任务持有）与看护任务（归 runtime 持有），两者都与实体表无关。
//!
//! ## 这条用例与 plan 0605 那条的分工
//!
//! 0605 管"掉线之后连得回来"（`tunnel_reconnect`），0606 管"关掉之后什么都不剩"。
//! 两者都要用"服务端把连接切断 / 服务端消失"造刺激，所以前面那段脚手架与它同形。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akasha_ssh::testing::{Relay, Running, ServerOptions, start};
use akasha_store::pools::{forwards, hosts};
use serde_json::{Value, json};
use support::{
    USER, click, connect_and_prepare, connect_tunnel_through_prompts, connect_tunnel_until, forget,
    free_port, open_tunnel_panel, open_vault, text_of, tunnel_entries, unlock, wait_js,
    wait_text_contains, wait_tunnel_gone,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use victauri_test::VictauriClient;

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-teardown-password";
/// 池里那台主机的名字（两条隧道都连它）。
const HOST_NAME: &str = "e2e-teardown-host";
/// 池里那条本机转发（`-L`）规则的名字。
const LOCAL_NAME: &str = "e2e-teardown-local";
/// 池里那条远程转发（`-R`）规则的名字。
const REMOTE_NAME: &str = "e2e-teardown-remote";
/// 第三条规则用**另一台主机**：它的端口先空着（连接会被拒），之后才有人接听（握手会挂住）。
const HANG_HOST_NAME: &str = "e2e-teardown-hang-host";
/// 第三条规则的名字 —— "在途的尝试也能被停止"那条判据盯它。
const HANG_NAME: &str = "e2e-teardown-hang";
/// **只有对端认识**的名字（RFC 2606 保留域；`local` 那条的目标）。
const TARGET_NAME: &str = "akasha-teardown.invalid";
/// 本机 HTTP 服务会写进响应体的那串字节。
const BODY: &str = "akasha-teardown-e2e\n";
/// `curl` 的等待上限。
const CURL_TIMEOUT_SECS: &str = "20";
/// 等一个状态 / 一次收尾的上限。
const WAIT_MS: u64 = 60_000;
/// "在途的尝试被中止"那条判据的期限。
///
/// 不中止的话，那次握手要等 `connect_timeout`（D15 的 **10 s**）才结束 —— 3 秒足以把两者分开，
/// 又留够了慢机器的余量。
const ABORT_BUDGET: Duration = Duration::from_secs(3);

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

/// 一个**接了 TCP 就不再说话**的服务端（plan 0606 用来把"连接中"拉长）。
///
/// 它接受连接、把对方的字节读完、然后什么都不回 —— 客户端的 SSH 握手因此停在那儿等对端的
/// banner。它记下两件事：**接到过几条**（证明那次尝试真的开始了）与**第一条什么时候读到 EOF**
/// （证明那次尝试的 socket 被关掉了）。
struct Silent {
    port: u16,
    accepted: Arc<AtomicUsize>,
    closed_at: Arc<Mutex<Option<Instant>>>,
}

impl Silent {
    /// 在**指定端口**上开始接听（端口由调用方先取好：那条规则里写的就是它）。
    async fn start(port: u16) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("那个端口本该是空的");
        let accepted = Arc::new(AtomicUsize::new(0));
        let closed_at = Arc::new(Mutex::new(None));
        let counter = Arc::clone(&accepted);
        let closed = Arc::clone(&closed_at);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let closed = Arc::clone(&closed);
                tokio::spawn(async move {
                    // 什么都不回：只管读，读到 EOF 就是"对端把这条连接关了"。
                    let mut sink = [0u8; 256];
                    while let Ok(read) = socket.read(&mut sink).await {
                        if read == 0 {
                            break;
                        }
                    }
                    let mut slot = closed.lock().unwrap();
                    if slot.is_none() {
                        *slot = Some(Instant::now());
                    }
                });
            }
        });
        Self {
            port,
            accepted,
            closed_at,
        }
    }

    /// 接到过几条连接。
    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// 第一条连接**什么时候**结束的（`None` = 还开着）。
    fn closed_at(&self) -> Option<Instant> {
        *self.closed_at.lock().unwrap()
    }
}

impl Drop for Silent {
    fn drop(&mut self) {
        // 留下一条日志，好让失败时的诊断知道那个端口是哪个。
        eprintln!("静默服务端 {} 接到过 {} 条", self.port, self.accepted());
    }
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

/// 这次要用的东西：一台主机 + 一条 `-L` + 一条 `-R`；另有一台"暂时没人听"的主机 + 一条规则。
///
/// 返回 `(local 规则 id, local 端口, remote 规则 id, remote 端口, hang 规则 id, hang 端口)`。
fn seed(path: &Path, ssh_port: u16, http_port: u16) -> Seeded {
    let conn = open_vault(path);

    // 先清干净（重跑）：主机行与规则行都按名字清。
    forget(&conn, HOST_NAME, "127.0.0.1", ssh_port);
    let names = [LOCAL_NAME, REMOTE_NAME, HANG_NAME];
    for row in forwards::forwards(&conn).unwrap() {
        if names.contains(&row.name.as_str()) {
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

    // 第三条：另一台主机，端口**先空着**（第一次连接会被立刻拒绝 —— 于是这条隧道落到
    // `失败`，而界面上"重试"这一颗按钮因此是可点的）。之后我们才在那个端口上开始接听但不说话，
    // 好让"重试"卡在握手里，从而验"在途的尝试也能被停止"。
    let hang_port = free_port();
    let hang_host = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: HANG_HOST_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port: hang_port,
            user: USER.to_owned(),
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();
    let hang_bind = free_port();
    let hang_rule = forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: HANG_NAME.to_owned(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".to_owned(),
            bind_port: hang_bind,
            target_host: Some(TARGET_NAME.to_owned()),
            target_port: Some(http_port),
            host_id: hang_host,
            autostart: false,
        },
    )
    .unwrap();

    Seeded {
        local_rule,
        local_port,
        remote_rule,
        remote_port,
        hang_rule,
        hang_port,
    }
}

/// [`seed`] 摆好的那些行。
struct Seeded {
    local_rule: i64,
    local_port: u16,
    remote_rule: i64,
    remote_port: u16,
    hang_rule: i64,
    hang_port: u16,
}

/// `residue` 探针的两个数：`(连接数, 看护任务数)`。
async fn residue(client: &mut VictauriClient) -> (u64, u64) {
    let value = client
        .call_tool("app_state", json!({ "probe": "residue" }))
        .await
        .expect("读不到 residue probe —— 它注册进 lib.rs 了吗？");
    (
        value
            .pointer("/sshConnections")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX),
        value
            .pointer("/watchTasks")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX),
    )
}

/// 等两个计数变成期望的那一组。
async fn wait_residue(client: &mut VictauriClient, want: (u64, u64), what: &str) {
    let deadline = Instant::now() + Duration::from_millis(WAIT_MS);
    loop {
        let now = residue(client).await;
        if now == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}：等不到 residue = {want:?}（现在是 {now:?}）"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 等**对端**看到指定条数的活连接。
async fn wait_server_connections(server: &Running, want: usize, what: &str) {
    let deadline = Instant::now() + Duration::from_millis(WAIT_MS);
    loop {
        let now = server.live_connections();
        if now == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}：服务端应当看到 {want} 条活连接（现在是 {now}）"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 等某个端口**连不上**（本机监听或服务端监听已经还回去了）。
async fn wait_port_released(port: u16, what: &str) {
    let deadline = Instant::now() + Duration::from_millis(WAIT_MS);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}：127.0.0.1:{port} 还连得上，那个监听没被释放"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 某个句柄在 `tunnels` probe 里还在不在。
async fn is_listed(client: &mut VictauriClient, handle: u64) -> bool {
    tunnel_entries(client)
        .await
        .iter()
        .any(|entry| entry.pointer("/handle").and_then(Value::as_u64) == Some(handle))
}

/// 打开某条规则的隧道，并把提示答完，返回 handle（等它**连上**）。
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

/// 打开某条规则的隧道，等它**落到某个状态**（用"打开就失败"那条路）。
async fn open_tunnel_until(
    client: &mut VictauriClient,
    rule_id: i64,
    fingerprint: &str,
    want: &str,
) -> u64 {
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
    let (handle, _) = connect_tunnel_until(client, rule_id, fingerprint, PASSWORD, want).await;
    assert!(handle > 0, "落到 {want} 的隧道也应当在册（拿得到 handle）");
    handle
}

/// 点某条规则的「停止」。
async fn stop(client: &mut VictauriClient, rule_id: i64, what: &str) {
    click(
        client,
        &format!(".tunnel-item[data-rule-id=\"{rule_id}\"] .tunnel-stop"),
        what,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closing_a_forward_session_leaves_nothing_behind() {
    if support::skip_unless_e2e() {
        return;
    }
    // `curl` 是硬前提（同 `tunnel_dynamic_forward` / `tunnel_remote_forward`）：
    // "那条隧道还能转发"这句话只有真实客户端能证。
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
    let seeded = seed(&path, server.addr.port(), http.port());
    unlock(&mut client).await;

    // ── 3. 起点：一条连接都没有 ──────────────────────────────────────────────
    assert_eq!(
        residue(&mut client).await,
        (0, 0),
        "还没开任何隧道，两个计数就该是空的（否则它们量的是别的东西）"
    );

    // ── 4. 两条隧道都打开：账上是两条连接、两条看护循环 ───────────────────────
    open_tunnel_panel(&mut client).await;
    let local = open_tunnel_for(&mut client, seeded.local_rule, &server.fingerprint).await;
    let remote = open_tunnel_for(&mut client, seeded.remote_rule, &server.fingerprint).await;
    assert_curl(seeded.local_port, "本机转发（关之前）");
    assert_curl(seeded.remote_port, "远端转发（关之前）");

    wait_residue(&mut client, (2, 2), "两条隧道都连上之后").await;
    wait_server_connections(&server, 2, "两条隧道都连上之后").await;
    eprintln!("两条隧道都连上：residue = (2, 2)，服务端也看到两条连接");

    // ── 5. 关掉 `-R`：只该少它那一条，另一条照常转发 ─────────────────────────
    stop(&mut client, seeded.remote_rule, "关掉远端转发那条").await;
    wait_tunnel_gone(&mut client, remote).await;
    wait_residue(&mut client, (1, 1), "关掉远端那条之后").await;
    wait_server_connections(&server, 1, "关掉远端那条之后").await;
    assert_curl(seeded.local_port, "关掉 -R 之后本机那条仍在转");

    // `-R` 的收尾比 `-L` 多一步：**撤销服务端那个监听**（D10）。那个端口是服务端的资源，
    // 不撤销的话"关掉了隧道、端口还开着"。
    wait_port_released(seeded.remote_port, "关掉 -R 之后，服务端那个端口该还回去").await;
    eprintln!("关掉 -R：连接数与看护任务数各少一，服务端那个端口也还回去了");

    // ── 6. 关掉 `-L`：两个计数归零，对端也看不到连接了 ───────────────────────
    stop(&mut client, seeded.local_rule, "关掉本机转发那条").await;
    wait_tunnel_gone(&mut client, local).await;
    wait_residue(&mut client, (0, 0), "两条都关掉之后").await;
    wait_server_connections(&server, 0, "两条都关掉之后").await;
    wait_port_released(seeded.local_port, "关掉 -L 之后，本机那个端口该还回去").await;
    eprintln!("两条都关掉：residue = (0, 0)，服务端一条连接都不剩");

    // ── 7. 关闭是幂等的，而且没有被重新拉起来 ────────────────────────────────
    // ⚠️ `invoke_command` 在命令返回 `Err` 时是**工具层错误**（`Err`），所以"没 panic"
    // 本身就是这条断言：重复关闭返回的是 `Ok`。
    client
        .invoke_command("tunnel_stop", Some(json!({ "handle": local })))
        .await
        .expect("再关一次不该报错（已经不在册就是'已经关了'）");
    assert!(!is_listed(&mut client, local).await, "它不该又回到册里");
    // 等一会儿再读：看护循环若还活着，它会在退避之后把这条隧道重新连起来（那会让计数抬头）。
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(
        residue(&mut client).await,
        (0, 0),
        "关掉之后 1.5 秒，两个计数仍该是 0 —— 没有东西在偷偷重连"
    );

    // ── 8. 在途的尝试也要能被停止 ────────────────────────────────────────────
    // 那条规则指向一台"暂时没人听"的主机：第一次连接被立刻拒绝，隧道落到 `失败`
    // （界面上"重试"因此可点，而"停止"也在）。之后我们才开始接听 —— **但不说话** ——
    // 于是重试卡在握手里。这时候停止它，socket 必须当场关掉，而不是等握手超时。
    let hang =
        open_tunnel_until(&mut client, seeded.hang_rule, &server.fingerprint, "failed").await;
    let silent = Silent::start(seeded.hang_port).await;

    click(
        &mut client,
        &format!(
            ".tunnel-item[data-rule-id=\"{}\"] .tunnel-retry",
            seeded.hang_rule
        ),
        "重试（这一次会卡在握手里）",
    )
    .await;
    wait_text_contains(
        &mut client,
        &format!(
            ".tunnel-item[data-rule-id=\"{}\"] .tunnel-state",
            seeded.hang_rule
        ),
        "连接中",
    )
    .await;
    let accepted = Instant::now();
    while silent.accepted() == 0 {
        assert!(
            Instant::now() < accepted + Duration::from_millis(WAIT_MS),
            "重试没有连到那台静默服务端上（它一条都没接到）"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    eprintln!("重试已经连上静默服务端（握手中），现在关掉这条隧道");

    stop(&mut client, seeded.hang_rule, "握手中停止这条隧道").await;
    wait_tunnel_gone(&mut client, hang).await;

    let deadline = Instant::now() + ABORT_BUDGET;
    while silent.closed_at().is_none() {
        assert!(
            Instant::now() < deadline,
            "停止之后 {:?} 内那个 socket 还没关：在途的尝试没有被中止（要等握手超时）",
            ABORT_BUDGET
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    eprintln!(
        "在途的尝试被中止：对端在 {:?} 内读到 EOF",
        silent.closed_at().map(|at| at.duration_since(accepted))
    );
    assert_eq!(
        residue(&mut client).await,
        (0, 0),
        "握手中被停止之后，两个计数也该是 0"
    );

    // ── 9. 收尾 ──────────────────────────────────────────────────────────────
    client
        .invoke_command("tunnel_stop", Some(json!({ "handle": hang })))
        .await
        .expect("收尾这条隧道也应当幂等");
    assert!(
        hits.load(Ordering::SeqCst) >= 2,
        "两个端口各该真的取到过一次（实际 {}）",
        hits.load(Ordering::SeqCst)
    );
    let _ = client.invoke_command("vault_lock", None).await;
    // 界面上的最后一眼：那几条都退回了"未打开"（停止按钮不再是它）。
    let shown = text_of(
        &mut client,
        &format!(
            ".tunnel-item[data-rule-id=\"{}\"] .tunnel-actions",
            seeded.local_rule
        ),
    )
    .await;
    assert!(
        shown.contains("打开"),
        "停掉之后那一行该回到「打开」：{shown:?}"
    );
}
