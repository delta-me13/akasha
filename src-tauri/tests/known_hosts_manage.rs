//! plan 0507 的**端到端**验收：主机指纹在界面上**看得到、删得掉**，且删除之后
//! 下一次连接**重新询问**。
//!
//! 判据（ROADMAP 原文）=「面板里看得到、删得掉，删除之后下一次连接重新询问」。
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 看到 | 连一台没见过的机器并确认 → 面板里一行，名字来自主机池、指纹逐字等于服务端的 |
//! | 删掉 | 点那一行的"删除" → 面板里 0 行，且**库那一侧**也没有了（读同一个文件） |
//! | 重新询问 | 再连一次同一台 → 主机密钥的询问**再次出现**（这就是"删除 = 遗忘"） |
//! | 没有第二条路 | 面板里除了刷新 / 关闭 / 删除，**没有**任何输入控件（ADR-0003 D11：添加与修改只在连接过程中） |
//!
//! ## 服务端在**测试进程**里
//!
//! 与 `ssh_session` 同一条口径：app 连过来，于是"服务端看到了什么"与"界面上看到了什么"
//! 是同一件事的两种观察。服务端本体是 `akasha_lib::ssh::testing`。
//!
//! ## 脚手架
//!
//! 造库 / 种数据 / 驱动界面的那一套在 [`support`]。
//!
//! 本文件是测试，`unwrap` 在这里就是断言手段。

#![allow(clippy::unwrap_used)]

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use akasha_lib::ssh::testing::{ServerOptions, start};
use akasha_lib::store::pools::hosts;
use support::{
    USER, click, connect_and_prepare, fill_secret, forget, open_vault, recorded_host_keys, text_of,
    unlock, wait_js,
};
use victauri_test::VictauriClient;

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-known-hosts-password";
/// 池里那一行的名字（也是面板里"名字"那一栏）。
const HOST_NAME: &str = "e2e-known-hosts-target";

/// 面板里现在有几行。
const COUNT_ROWS: &str = "document.querySelectorAll('.known-hosts-item').length";

/// 两种提示里**任意一种**在不在（与 `support` 的问答循环同一手法）。
const ANY_PROMPT: &str = "!!document.querySelector('.ssh-prompt[data-prompt-kind=\"hostKey\"]') \
                          || !!document.querySelector('.ssh-prompt[data-prompt-kind=\"credential\"]')";

/// 在库里把这次要连的那一行摆好（并把 `(host, port)` 的指纹记录清掉）。
fn seed(path: &Path, port: u16) {
    let conn = open_vault(path);
    forget(&conn, HOST_NAME, "127.0.0.1", port);
    hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: HOST_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port,
            user: USER.to_owned(),
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();
}

/// 从主机选择器里选池里那一行（面板 / 选择器都是挂载时读一次）。
async fn pick_host(client: &mut VictauriClient, host_id: u64) {
    click(client, ".tab-new-ssh", "打开主机选择器").await;
    wait_js(
        client,
        &format!("!!document.querySelector('.host-picker-item[data-host-id=\"{host_id}\"]')"),
        10_000,
        "主机选择器列出了池里那一行",
    )
    .await;
    click(
        client,
        &format!(".host-picker-item[data-host-id=\"{host_id}\"]"),
        "选这台主机",
    )
    .await;
}

/// 答完这一台主机的全部提问（主机密钥 + 口令，**一个都不问**也可能）直到连上。
///
/// 为什么是循环而不是写死两步：口令有内存缓存（问题 #124），第二次连接可能只问密钥；
/// 写死步数会在缓存命中时**空等**，而表现是"连不上"而不是"用例写错了"。
/// `asked` 记下问过什么 —— "重新询问"那条判据就是它。
async fn answer_until_connected(
    client: &mut VictauriClient,
    tabs: usize,
    fingerprint: &str,
    asked: &mut Vec<String>,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if support::is_connected(client, tabs).await {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "提示问答没走完就连不上（问到过的：{asked:?}）"
        );
        let _ = client
            .wait_for_expression(ANY_PROMPT, None, Some(3_000), None)
            .await;

        let shown = text_of(
            client,
            ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-fingerprint",
        )
        .await;
        if !shown.is_empty() {
            // 指纹必须能核对：界面给错等于没给（与 `ssh_session` 同一条断言）。
            assert_eq!(shown, fingerprint, "询问里那串指纹必须就是服务端的");
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
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_panel_shows_and_forgets_a_recorded_host_key() {
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
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };
    seed(&path, server.addr.port());
    unlock(&mut client).await;

    let listed = client
        .invoke_command("vault_hosts", None)
        .await
        .expect("vault_hosts 调不通");
    let host_id = listed
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row.pointer("/id"))
        .and_then(serde_json::Value::as_u64)
        .expect("池里该有刚种下的那一行");

    // ── 3. 第一次连接：没见过的主机密钥要**问**，接受之后进库 ────────────────
    pick_host(&mut client, host_id).await;
    let mut first = Vec::new();
    answer_until_connected(&mut client, 2, &server.fingerprint, &mut first).await;
    assert!(
        first.iter().any(|asked| asked.starts_with("hostKey:")),
        "没见过的主机密钥必须问一次（问到过的：{first:?}）"
    );
    eprintln!("提示: 第一次连接={first:?}");

    // ── 4. 面板：看得到那一行，名字来自池、指纹逐字等于服务端的 ──────────────
    click(&mut client, ".tab-new-known-hosts", "打开主机指纹面板").await;
    wait_js(
        &mut client,
        "!!document.querySelector('.known-hosts-panel')",
        10_000,
        "主机指纹面板打开了",
    )
    .await;
    wait_js(
        &mut client,
        &format!("{COUNT_ROWS} === 1"),
        10_000,
        "面板里有一行",
    )
    .await;

    let name = text_of(&mut client, ".known-hosts-item .known-hosts-names").await;
    assert_eq!(name, HOST_NAME, "名字该来自主机池里那一行");
    let shown = text_of(&mut client, ".known-hosts-item .known-hosts-fingerprint").await;
    assert_eq!(shown, server.fingerprint, "面板里的指纹必须是服务端那把");
    let route = text_of(&mut client, ".known-hosts-item .known-hosts-route").await;
    assert!(
        route.contains(&format!("127.0.0.1:{}", server.addr.port())),
        "面板里该说清是哪一台的哪个端口：{route:?}"
    );
    eprintln!("面板: 行数=1 名字={name} 指纹={shown}");

    // ── 5. 负例：面板里**没有**"信任新密钥"这类入口（D11 非目标那两条的界面表现）──
    let only_explicit_actions = client
        .eval_js(
            "Array.from(document.querySelectorAll('.known-hosts-panel button, .known-hosts-panel input, .known-hosts-panel select, .known-hosts-panel textarea')).every((el) => el.classList.contains('known-hosts-forget') || el.classList.contains('known-hosts-refresh') || el.classList.contains('known-hosts-close'))",
        )
        .await
        .unwrap();
    assert!(
        support::payload(&only_explicit_actions)
            .as_bool()
            .unwrap_or(false),
        "面板里出现了删除 / 刷新 / 关闭之外的控件 —— 添加与修改只许发生在连接过程中：{only_explicit_actions}"
    );

    // ── 6. 删掉那一行：界面与**库那一侧**都要没有 ────────────────────────────
    click(
        &mut client,
        ".known-hosts-item .known-hosts-forget",
        "删掉这一行",
    )
    .await;
    wait_js(
        &mut client,
        &format!("{COUNT_ROWS} === 0"),
        10_000,
        "那一行从面板里消失",
    )
    .await;
    let notice = text_of(&mut client, ".known-hosts-notice").await;
    assert!(
        notice.contains("重新询问"),
        "删除之后该说清下一步会发生什么（重新询问）：{notice:?}"
    );
    assert!(
        !recorded_host_keys(&path)
            .iter()
            .any(|row| row.host == "127.0.0.1" && row.port == server.addr.port()),
        "库里那一行必须真的没了（删的是缓存，不是界面上的一个影子）"
    );
    eprintln!("删除: 面板行数=0 库侧=无那条记录");

    // 关掉面板：它盖在标签栏下面那一层，下一步要点标签栏上的按钮。
    click(&mut client, ".known-hosts-close", "关上主机指纹面板").await;

    // ── 7. 再连一次：**重新询问**（这就是"删除 = 遗忘"）────────────────────
    pick_host(&mut client, host_id).await;
    let mut second = Vec::new();
    answer_until_connected(&mut client, 3, &server.fingerprint, &mut second).await;
    assert!(
        second.iter().any(|asked| asked.starts_with("hostKey:")),
        "删掉之后同一台必须重新询问（问到过的：{second:?}）"
    );
    eprintln!("提示: 第二次连接={second:?}");

    // ── 8. 收尾：把两条 SSH 标签页关掉，只留最开始那个本地终端 ──────────────
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
        support::CLOSE_TIMEOUT.as_millis() as u64,
        "两个 SSH 标签页都关掉了",
    )
    .await;

    // 后端：会话表回到只剩本地那一个（SSH 没有本地进程，所以这条判据只能看注册表）。
    let deadline = Instant::now() + support::CLOSE_TIMEOUT;
    loop {
        let sessions = client
            .app_state(Some("sessions"))
            .await
            .expect("读不到 sessions probe");
        if sessions
            .pointer("/live")
            .and_then(serde_json::Value::as_u64)
            == Some(1)
            && sessions
                .pointer("/registered")
                .and_then(serde_json::Value::as_u64)
                == Some(1)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "关掉标签页之后后端还登记着会话：{sessions}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
