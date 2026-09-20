//! plan 0504 的**端到端**验收：真 app 上开一个 SSH 会话。
//!
//! 判据（ROADMAP 原文）=「真 app 上开一个 SSH 会话 —— 字节能双向流、凭据只问一次、
//! 关标签页零残留」。这条用例逐条盯着它们：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 能从界面选主机并连上 | 点 `.tab-new-ssh` → 点池里那一行 → 标签页出现且状态"已连接" |
//! | 未知主机密钥要**问**（D11） | `.ssh-prompt[data-prompt-kind=hostKey]` 里那串指纹**等于服务端的** |
//! | 字节能双向流 | 终端里敲命令 → 屏幕出现回声，且**服务端**也收到了同一串字节 |
//! | 确认过的密钥进**我们的库** | 直连库文件读 `known_hosts`：一行，算法/端口对得上 |
//! | 凭据只问一次 | 第二个会话**一次都没问**就"已连接"，而服务端两次收到**同一句**口令 |
//! | 关标签页零残留 | 服务端看到连接关闭（`sessions_closed`），且 `sessions` probe 回到 1（只剩本地那个） |
//!
//! ## 服务端在**测试进程**里
//!
//! app 是另一个进程，它连过来 —— 于是"服务端看到了什么"与"界面上看到了什么"是同一件事的
//! 两种观察，可以在一条用例里对账。服务端本体是 `akasha_lib::ssh::testing`（生产代码别用它，
//! 理由写在那个模块的文档里）。
//!
//! ## 脚手架
//!
//! 造库 / 种数据 / 驱动界面的那一套在 [`support`]（plan 0505 提出来，因为经跳板那条用例
//! 要的是同一套）。**数据从哪来**的那个理由写在那里。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use akasha_lib::ssh::testing::{ServerOptions, start};
use akasha_lib::store::pools::hosts;
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, CONNECT_TIMEOUT_MS, USER, click, connect_and_prepare, fill_secret, forget,
    observed, open_vault, recorded_host_keys, text, type_line, unlock, wait_connected, wait_js,
};

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-ssh-login-password";
/// 池里那一行的名字（也是标签页标题）。
const HOST_NAME: &str = "e2e-ssh-target";

/// 在库里把这次要连的那一行摆好。
fn seed(path: &Path, port: u16) -> PathBuf {
    let conn = open_vault(path);

    // 先清干净（重跑）：主机行按名字找，known_hosts 按 (host, port) 清。
    forget(&conn, HOST_NAME, "127.0.0.1", port);

    hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: HOST_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port,
            user: USER.to_owned(),
            // 口令认证：这样这条用例不依赖 agent，也不依赖任何密钥材料。
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();

    path.to_path_buf()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ssh_session_flows_bytes_and_asks_for_a_credential_once() {
    if support::skip_unless_e2e() {
        return;
    }

    // ── 1. 服务端（在**本进程**里）：随机端口 + 只认口令 ─────────────────────
    let server = start(ServerOptions::password(PASSWORD)).await;
    eprintln!(
        "构造: 服务端=127.0.0.1:{} 指纹={}",
        server.addr.port(),
        server.fingerprint
    );

    // ── 2. 库在哪、现在是什么状态、必要时先锁上 ──────────────────────────────
    // `_fixture` 是个**有名**的绑定（不是 `_`）：它要活到用例结束才删库。
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };

    // ── 3. 种子数据 + 解锁 ─────────────────────────────────────────────────
    seed(&path, server.addr.port());
    unlock(&mut client).await;

    // 界面凭什么选主机：这一条是 plan 0504 新增的只读命令。
    let listed = client
        .invoke_command("vault_hosts", None)
        .await
        .expect("vault_hosts 调不通 —— 它登记进 bindings.rs 了吗？");
    let rows = listed.as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 1, "池里应当只有我们种下的那一行：{listed}");
    let host_id = rows[0]
        .pointer("/id")
        .and_then(Value::as_u64)
        .expect("HostEntry 里必须有 id");
    assert_eq!(
        rows[0].pointer("/jumpId").and_then(Value::as_u64),
        None,
        "这一行没有跳板：jumpId 该是 null（有值的话说明池里那列被写进了别的东西）"
    );

    // ── 4. 界面：点 SSH → 选主机 ─────────────────────────────────────────────
    let before_tabs = text(
        &client
            .eval_js("document.querySelectorAll('.tab').length.toString()")
            .await
            .unwrap(),
    );
    click(&mut client, ".tab-new-ssh", "打开主机选择器").await;
    wait_js(
        &mut client,
        &format!(
            "!!document.querySelector('.host-picker-item[data-host-id=\"{}\"]')",
            host_id
        ),
        10_000,
        "主机选择器列出了池里那一行",
    )
    .await;
    click(
        &mut client,
        &format!(".host-picker-item[data-host-id=\"{host_id}\"]"),
        "选这台主机",
    )
    .await;
    eprintln!("界面: 主机id={host_id} 标签页数={before_tabs}");

    // ── 5. 未知主机密钥：**问**，而且给出的指纹要能核对 ──────────────────────
    wait_js(
        &mut client,
        "!!document.querySelector('.ssh-prompt[data-prompt-kind=\"hostKey\"]')",
        CONNECT_TIMEOUT_MS,
        "没见过的主机密钥要弹出来问用户",
    )
    .await;
    let shown = support::text_of(&mut client, ".ssh-prompt-fingerprint").await;
    assert_eq!(
        shown, server.fingerprint,
        "提示里那串指纹必须就是服务端的（用户拿它去核对，给错等于没给）"
    );

    // ── 6. 凭据：问到口令，填进去提交 ───────────────────────────────────────
    click(
        &mut client,
        ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-accept",
        "接受这把主机密钥",
    )
    .await;
    wait_js(
        &mut client,
        "!!document.querySelector('.ssh-prompt[data-prompt-kind=\"credential\"]')",
        CONNECT_TIMEOUT_MS,
        "主机密钥之后该问登录口令",
    )
    .await;
    fill_secret(&mut client, PASSWORD).await;
    click(
        &mut client,
        ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-submit",
        "提交口令",
    )
    .await;

    // ── 7. 会话开了：标签页出现、终端说"已连接" ──────────────────────────────
    wait_connected(&mut client, 2, "第一个 SSH 会话").await;
    let title = text(
        &client
            .eval_js("document.querySelector('.tab.is-active .tab-label')?.textContent ?? ''")
            .await
            .unwrap(),
    );
    assert_eq!(title, HOST_NAME, "标签页标题该是池里那台主机的名字");

    // ── 8. 字节能双向流 ────────────────────────────────────────────────────
    type_line(&mut client, "echo ssh-hello\n").await;
    wait_js(
        &mut client,
        "window.__akashaTerminal.screenText(200).includes('ssh-hello')",
        CONNECT_TIMEOUT_MS,
        "服务端的回声出现在了终端上",
    )
    .await;
    let seen = String::from_utf8_lossy(&observed(&server).shell_data).to_string();
    assert!(
        seen.contains("echo ssh-hello"),
        "服务端没收到那行字节（收到 {seen:?}）—— \"能发出去\"与\"对端收到了\"是两件事"
    );

    // ── 9. 确认过的主机密钥真的进了**我们的库** ──────────────────────────────
    let recorded = recorded_host_keys(&path);
    let row = recorded
        .iter()
        .find(|row| row.host == "127.0.0.1" && row.port == server.addr.port())
        .expect("确认过的密钥必须落进库（否则下次连接又要问一遍）");
    assert_eq!(row.key_type, "ssh-ed25519", "记的算法该是服务端那把");
    assert_eq!(
        row.fingerprint, server.fingerprint,
        "记的指纹该是服务端那把"
    );
    eprintln!(
        "池: known_hosts行数={} 主机={}:{}",
        recorded.len(),
        row.host,
        row.port
    );

    // ── 10. 凭据只问一次：第二个会话**一次都不问** ───────────────────────────
    let before = observed(&server).passwords.len();
    click(&mut client, ".tab-new-ssh", "再打开一次主机选择器").await;
    wait_js(
        &mut client,
        &format!("!!document.querySelector('.host-picker-item[data-host-id=\"{host_id}\"]')"),
        10_000,
        "第二次也列出了池里那一行",
    )
    .await;
    click(
        &mut client,
        &format!(".host-picker-item[data-host-id=\"{host_id}\"]"),
        "再选一次同一台主机",
    )
    .await;

    // ⚠️ 这一等就是判据本身：**谁都没有回答任何提问**，而连接必须自己走完 ——
    // 只要它需要一个口令，它就会停在那儿等答案，于是这条等待必然超时。
    wait_connected(&mut client, 3, "第二个会话（一次都没问就连上）").await;
    let after = observed(&server);
    assert_eq!(
        after.passwords.len(),
        before + 1,
        "服务端该看到第二次认证（凭据缓存复用的是同一句口令，不是跳过了认证）"
    );
    assert_eq!(
        after.passwords.last().map(String::as_str),
        Some(PASSWORD),
        "第二次认证用的必须还是同一句口令：{:?}",
        after.passwords
    );

    // ── 11. 关标签页零残留 ────────────────────────────────────────────────
    // 关掉两个 SSH 标签页（它们排在本地终端后面），只留最开始那个。
    for _ in 0..2 {
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
    }
    wait_js(
        &mut client,
        "document.querySelectorAll('.tab').length === 1",
        CLOSE_TIMEOUT.as_millis() as u64,
        "两个 SSH 标签页都关掉了",
    )
    .await;

    // 后端：会话表回到只剩本地那一个（SSH **没有本地进程**，所以这条判据只能看注册表）。
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

    // 对端：服务端看到两条连接都断了（这是"连接真的没了"的唯一外部证据）。
    let deadline = std::time::Instant::now() + CLOSE_TIMEOUT;
    loop {
        let closed = observed(&server).sessions_closed;
        if closed >= 2 {
            eprintln!("服务端: 连接关闭={closed}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "标签页关掉了，服务端却还看到连接挂着（只有 {closed} 条收到 EOF）"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── 12. 收尾：锁上并删掉自己造的库（`_fixture` 的析构负责删）─────────────
    let _ = client.invoke_command("vault_lock", None).await;
}
