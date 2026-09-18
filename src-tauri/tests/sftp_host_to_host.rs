//! plan 0703 的**端到端**验收：两栏都是主机时的两档与回退。
//!
//! 判据（ROADMAP 原文）=「A 无法直连 B 时**自动走 A 档**；两档**均不落盘**」。逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | **B 档**：目标只在源那一栏的网络里 | 池里目标那行的 `host` 是 `.invalid` 的名字，而它**在本机解析不出来**（用例自己解析一次） |
//! | B 档真的直通 | 跳板服务端记到**恰好一条** `direct-tcpip`（host/port 与池里那行一致），中继搬过字节 |
//! | 界面上说得出走的是哪一档 | 目标那一栏显示"经 … 直通"，传输那一行的 `data-transfer-via` 是那台的 id |
//! | 字节落在目标**真盘**上 | 目标那棵真目录里是最终名、字节与源相同、**没有**临时名 |
//! | **回退可观测** | 换一台本机能直达的主机 → 直通被拒 → 那一栏记下 `throughFailure`、`through` 为空，而连接照样成功 |
//! | **A 档**：回退之后传输仍然走通 | 同一个引擎、同一条落盘判据；传输记录里 `via` 为空（本机内存中转） |
//! | 源那一台**只被读** | 两段之后源目录的条目**一个都没多** |
//! | 关会话收干净 | 探针里会话归零，两台服务端都看到连接断开 |
//!
//! ## "本机够不着目标"在这个用例里怎么成立
//!
//! 与 plan 0505 / 0703 的库内用例同一条做法：无特权环境做不出真正的网络隔离，所以这条性质
//! 靠**名字**造 —— 目标是 `akasha-e2e-sftp.invalid:22`（RFC 2606 保留域），它只在跳板服务端的
//! 中继表里指向真服务端的监听地址。于是"B 档的字节到了目标盘上"只可能经过跳板。
//!
//! 第二段要验的是**回退**，所以那一栏换成本机能直达的地址（`127.0.0.1:<目标端口>`）：
//! 跳板对这条地址没有映射 —— 它拒绝转发，与本机 `AllowTcpForwarding no` 的表现同形 ——
//! 于是后端改走本机直连，传输变成内存中转。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::net::ToSocketAddrs;
use std::path::Path;
use std::time::{Duration, Instant};

use akasha_lib::ssh::testing::{Relay, Running, ServerOptions, SftpItem, start};
use akasha_lib::store::pools::hosts;
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, USER, click, connect_and_prepare, forget, open_vault, text, text_of, unlock,
    wait_js,
};
use victauri_test::VictauriClient;

/// 两台各用各的登录口令：B 档那条链上两跳**各认证各的**（凭据不是从另一台借的）。
const BASTION_PASSWORD: &str = "e2e-sftp-bastion-password";
const FAR_PASSWORD: &str = "e2e-sftp-far-password";

const BASTION_NAME: &str = "e2e-sftp-bastion";
const FAR_NAME: &str = "e2e-sftp-far";
const NEAR_NAME: &str = "e2e-sftp-near";

/// **只有跳板认识**的名字与端口（理由见文件头）。
const ONLY_NAME: &str = "akasha-e2e-sftp.invalid";
const ONLY_PORT: u16 = 22;

/// 源那一台的两个文件。48 KiB 越过一块（`CHUNK_BYTES` = 32 KiB），于是判据看得到"分块"。
const ALPHA_BYTES: usize = 48 * 1024;
const GAMMA_BYTES: usize = 40 * 1024;

fn alpha() -> Vec<u8> {
    (0u8..=255).cycle().take(ALPHA_BYTES).collect()
}

fn gamma() -> Vec<u8> {
    vec![b'g'; GAMMA_BYTES]
}

/// 起两台服务端：目标（只在跳板的中继表里）与跳板（自己也是可搬文件的一栏）。
async fn two_hosts() -> (Running, Running) {
    let far = start(ServerOptions {
        password: Some(FAR_PASSWORD.to_owned()),
        sftp: Some(vec![SftpItem::dir("inbox")]),
        ..ServerOptions::default()
    })
    .await;
    let bastion = start(ServerOptions {
        password: Some(BASTION_PASSWORD.to_owned()),
        relay: vec![Relay {
            host: ONLY_NAME.to_owned(),
            port: ONLY_PORT,
            to: far.addr,
        }],
        sftp: Some(vec![
            SftpItem::file_with("alpha.bin", alpha()),
            SftpItem::file_with("gamma.bin", gamma()),
        ]),
        ..ServerOptions::default()
    })
    .await;
    (bastion, far)
}

/// 一个目录里的条目名（排序）—— 判据比的是"有什么"，不是服务端以什么顺序列出来。
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 在库里摆好三行：跳板（本机直达）、目标（只在跳板后面）、目标的本机地址。
///
/// 第三行是第二段的刺激：同一台服务端，地址换成**本机能直达**的那个 —— 于是跳板对它没有映射，
/// 直通被拒，后端必须回退。
fn seed(path: &Path, bastion_port: u16, far_port: u16) -> (u32, u32, u32) {
    let conn = open_vault(path);
    forget(&conn, BASTION_NAME, "127.0.0.1", bastion_port);
    forget(&conn, FAR_NAME, ONLY_NAME, ONLY_PORT);
    forget(&conn, NEAR_NAME, "127.0.0.1", far_port);

    let insert = |name: &str, host: &str, port: u16| {
        let id = hosts::insert_host(
            &conn,
            &hosts::NewHost {
                name: name.to_owned(),
                host: host.to_owned(),
                port,
                user: USER.to_owned(),
                auth: hosts::Auth::Password,
                key_id: None,
                jump_id: None,
            },
        )
        .unwrap();
        u32::try_from(id).unwrap()
    };
    (
        insert(BASTION_NAME, "127.0.0.1", bastion_port),
        insert(FAR_NAME, ONLY_NAME, ONLY_PORT),
        insert(NEAR_NAME, "127.0.0.1", far_port),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_hosts_go_through_the_tunnel_and_fall_back_to_the_relay() {
    if support::skip_unless_e2e() {
        return;
    }

    let (bastion, far) = two_hosts().await;
    let bastion_root = bastion.sftp_root().unwrap().to_path_buf();
    let far_root = far.sftp_root().unwrap().to_path_buf();
    eprintln!(
        "跳板 127.0.0.1:{} · 目标 {ONLY_NAME}:{ONLY_PORT}（跳板表里指向 127.0.0.1:{}）",
        bastion.addr.port(),
        far.addr.port()
    );
    assert!(
        (ONLY_NAME, ONLY_PORT).to_socket_addrs().is_err(),
        "{ONLY_NAME} 在本机解析出来了 —— B 档这条判据的前提不成立"
    );

    let Some((mut client, _fixture, vault)) = connect_and_prepare().await else {
        return;
    };
    let (bastion_id, far_id, near_id) = seed(&vault, bastion.addr.port(), far.addr.port());
    unlock(&mut client).await;

    // ── 1. 界面：左栏连跳板，右栏连那台**只有跳板看得见**的主机 ─────────────────
    open_sftp_panel(&mut client).await;
    start_session(&mut client).await;
    choose_origin(&mut client, "left", &format!("host:{bastion_id}")).await;
    let asked = connect_side(
        &mut client,
        "left",
        bastion_id,
        &[&bastion.fingerprint],
        &[(bastion.addr.port(), BASTION_PASSWORD)],
    )
    .await;
    eprintln!("左栏连上跳板，问到过 {asked:?}");

    choose_origin(&mut client, "right", &format!("host:{far_id}")).await;
    let asked = connect_side(
        &mut client,
        "right",
        far_id,
        &[&far.fingerprint],
        &[(far.addr.port(), FAR_PASSWORD), (ONLY_PORT, FAR_PASSWORD)],
    )
    .await;
    eprintln!("右栏经跳板连上目标，问到过 {asked:?}");
    wait_js(
        &mut client,
        "!!document.querySelector('.sftp-pane[data-side=\"right\"] \
         .sftp-entry[data-entry-name=\"inbox\"]')",
        15_000,
        "右栏列出了目标那棵目录（SFTP 跑在隧道里）",
    )
    .await;

    // 判据：界面与探针都说得出"这一栏是经跳板直通到达的"。
    wait_js(
        &mut client,
        &format!(
            "document.querySelector('.sftp-pane[data-side=\"right\"] .sftp-route')?.dataset.sftpThrough === '{bastion_id}'"
        ),
        10_000,
        "右栏显示经跳板直通",
    )
    .await;
    let right = side_info(&mut client, "right").await;
    assert_eq!(
        right.pointer("/through").and_then(Value::as_u64),
        Some(u64::from(bastion_id)),
        "探针里右栏应当记着经哪台直通：{right}"
    );
    assert_eq!(
        right.pointer("/throughFailure"),
        Some(&Value::Null),
        "直通成功时不该有回退原因"
    );

    // 服务端那一半：跳板被要求**恰好一次**去连那个名字，而且中继真的搬了字节。
    let seen = support::observed(&bastion);
    assert_eq!(
        seen.direct_tcpip.len(),
        1,
        "跳板应当收到恰好一条 direct-tcpip：{:?}",
        seen.direct_tcpip
    );
    assert_eq!(seen.direct_tcpip[0].host, ONLY_NAME);
    assert_eq!(seen.direct_tcpip[0].port, u32::from(ONLY_PORT));

    // ── 2. 判据：B 档的传输把字节送到目标真盘上，源那一台一个条目都没多 ──────────
    send_file(
        &mut client,
        "left",
        "alpha.bin",
        "B 档：把 alpha.bin 传到目标",
    )
    .await;
    assert!(
        bastion.shared.relayed_bytes() > 0,
        "跳板的中继一个字节都没搬"
    );
    assert_eq!(
        std::fs::read(far_root.join("alpha.bin")).unwrap(),
        alpha(),
        "目标盘上的字节必须与源相同"
    );
    assert_eq!(
        names(&far_root),
        vec!["alpha.bin".to_owned(), "inbox".to_owned()],
        "目标目录里不该多出临时名"
    );
    assert_eq!(
        names(&bastion_root),
        vec!["alpha.bin".to_owned(), "gamma.bin".to_owned()],
        "源那一台的目录不该有任何变化"
    );
    let via = transfer_via(&mut client).await;
    assert_eq!(
        via.as_deref(),
        Some(bastion_id.to_string().as_str()),
        "这次传输的记录里应当是经跳板直通"
    );
    eprintln!(
        "B 档：跳板收到 1 条 direct-tcpip → {ONLY_NAME}:{ONLY_PORT}，中继搬了 {} 字节；目标盘上 alpha.bin 的字节数与源一致",
        bastion.shared.relayed_bytes()
    );

    // ── 3. 回退：右栏换成本机能直达的地址 —— 跳板对那条地址没有映射，于是拒转发 ──
    choose_origin(&mut client, "right", &format!("host:{near_id}")).await;
    let asked = connect_side(
        &mut client,
        "right",
        near_id,
        &[&far.fingerprint],
        &[(far.addr.port(), FAR_PASSWORD)],
    )
    .await;
    eprintln!("右栏回退到本机直连，问到过 {asked:?}");

    let right = side_info(&mut client, "right").await;
    assert_eq!(
        right.pointer("/through"),
        Some(&Value::Null),
        "回退之后不该还记着经谁直通：{right}"
    );
    let failure = right
        .pointer("/throughFailure")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    assert!(
        !failure.is_empty(),
        "回退必须留下原因（否则用户看不出原本想走直通）：{right}"
    );
    // 界面要跟上来再读：上面那句判据读的是**后端**（探针），而面板是在命令返回之后
    // 才刷新两侧状态的 —— 直接读会读到还没渲染的那一帧（这一条实测偶发红过一次）。
    wait_js(
        &mut client,
        "(() => { const el = document.querySelector('.sftp-pane[data-side=\"right\"] .sftp-detour'); \
         return !!el && el.dataset.sftpDetour !== ''; })()",
        10_000,
        "界面上出现回退原因",
    )
    .await;
    let shown = text_of(&mut client, ".sftp-pane[data-side=\"right\"] .sftp-detour").await;
    assert!(
        shown.contains(&failure),
        "界面上应当把这个原因说给用户：{shown:?}"
    );
    eprintln!("回退：经跳板直通失败（{failure}），已改为本机直连");

    // 服务端那一半：跳板确实又被问了一次，而这次它没有认。
    let seen = support::observed(&bastion);
    assert_eq!(
        seen.direct_tcpip.len(),
        2,
        "回退那一次也该到过跳板（它被要求连本机地址）：{:?}",
        seen.direct_tcpip
    );
    assert_eq!(seen.direct_tcpip[1].host, "127.0.0.1");
    assert_eq!(seen.direct_tcpip[1].port, u32::from(far.addr.port()));

    // ── 4. 判据：回退之后传输仍然走通，而且是**本机内存中转**那一档 ──────────────
    send_file(
        &mut client,
        "left",
        "gamma.bin",
        "A 档：把 gamma.bin 传到已经回退的那一栏",
    )
    .await;
    assert_eq!(
        std::fs::read(far_root.join("gamma.bin")).unwrap(),
        gamma(),
        "A 档的字节也必须与源相同"
    );
    assert_eq!(
        names(&far_root),
        vec![
            "alpha.bin".to_owned(),
            "gamma.bin".to_owned(),
            "inbox".to_owned()
        ],
        "目标目录里不该多出临时名"
    );
    assert_eq!(
        names(&bastion_root),
        vec!["alpha.bin".to_owned(), "gamma.bin".to_owned()],
        "源那一台在两段之后仍然一个条目都没多"
    );
    assert_eq!(
        transfer_via(&mut client).await,
        None,
        "A 档的传输记录里 `via` 应当为空（本机内存中转）"
    );
    eprintln!("A 档：回退之后 gamma.bin 同样落到了目标盘上，传输记录里没有直通那一项");

    // ── 5. 关会话：两侧都收干净（外部证据 —— 我们自己说"关了"不算） ─────────────
    click(&mut client, ".sftp-stop", "结束 SFTP 会话").await;
    wait_js(
        &mut client,
        "!document.querySelector('.sftp-pane[data-side=\"left\"]')",
        30_000,
        "会话结束后两栏消失",
    )
    .await;
    assert!(
        sftp_entries(&mut client).await.is_empty(),
        "关掉之后探针该是空的"
    );
    for (who, server) in [("跳板", &bastion), ("目标", &far)] {
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        while support::observed(server).connections_closed == 0 {
            assert!(
                Instant::now() < deadline,
                "{who}那边还看到连接挂着 —— 会话关了却没断开"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    let _ = client.invoke_command("vault_lock", None).await;
}

// ── 驱动界面与读后端的辅助 ───────────────────────────────────────────────────

/// 打开 SFTP 面板（并保证它重新读一遍池子与已有会话）。
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

/// 保证面板上是一个**新建的**会话（上一个 E2E 目标可能留了一个）。
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

/// 在某一栏里选来源（`local`，或者 `host:<id>`）。
///
/// 为什么要绕过 React 的 value tracker（用原型上的 setter 再手动派发 `change`）：
/// 直接写 `select.value = …` 时 React 记着上一次的值，`change` 会被它当成"没变"。
async fn choose_origin(client: &mut VictauriClient, side: &str, value: &str) {
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
        "{side} 那一栏的来源没选上（得到 {chosen:?}）"
    );
}

/// 连某一侧：点「连接」，按类型回答提示，直到那一侧变成 `connected`。
///
/// 指纹与口令都按**提示里给的东西**挑，不写死步数：B 档那条链上可能有两跳，
/// 而"链上某台的主机密钥与凭据缓存已经命中"这件事不该让用例少答一轮就失败
/// （plan 0505 的教训：写死步数会在拓扑一变时**静默少答一轮**）。
async fn connect_side(
    client: &mut VictauriClient,
    side: &str,
    host_id: u32,
    keys: &[&str],
    logins: &[(u16, &str)],
) -> Vec<String> {
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
        let info = side_info(client, side).await;
        let state = info
            .pointer("/state")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let origin = info.pointer("/origin/id").and_then(Value::as_u64);
        if state == "connected" && origin == Some(u64::from(host_id)) {
            return asked;
        }
        if state == "failed" {
            let failure = info
                .pointer("/failure")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            panic!("{side} 这一侧连接失败：{failure}");
        }
        assert!(
            Instant::now() < deadline,
            "{side} 这一侧没连上（问到过的：{asked:?}，探针：{info}）"
        );

        let _ = client
            .wait_for_expression(any_prompt, None, Some(3_000), None)
            .await;

        let presented = text_of(
            client,
            ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-fingerprint",
        )
        .await;
        if !presented.is_empty() {
            assert!(
                keys.contains(&presented.as_str()),
                "提示里那串指纹不属于这条链上的任何一台：{presented}"
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
            let password = logins
                .iter()
                .find(|(port, _)| hint.ends_with(&format!(":{port}")))
                .map(|(_, password)| *password)
                .unwrap_or_else(|| panic!("提示问的端口不在预期里：{hint}"));
            support::fill_secret(client, password).await;
            click(
                client,
                ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-submit",
                "提交口令",
            )
            .await;
        }
    }
}

/// 把某一栏的一个文件传到对侧，并等到后端说它**完成**。
async fn send_file(client: &mut VictauriClient, side: &str, name: &str, what: &str) {
    click(
        client,
        &format!(".sftp-pane[data-side=\"{side}\"] .sftp-send[data-entry-name=\"{name}\"]"),
        what,
    )
    .await;
    wait_js(
        client,
        "(() => { const row = document.querySelector('.sftp-transfer'); \
         return !!row && row.dataset.transferState === 'done'; })()",
        30_000,
        what,
    )
    .await;
}

/// 最新那条传输的 `via`（界面上那个属性就是后端的字段）。
async fn transfer_via(client: &mut VictauriClient) -> Option<String> {
    let raw = client
        .eval_js("document.querySelector('.sftp-transfer')?.dataset.transferVia ?? null")
        .await
        .unwrap();
    let value = text(&raw);
    if value.is_empty() || value == "null" {
        None
    } else {
        Some(value)
    }
}

/// 某一侧在 `sftp` 探针里的那条记录。
async fn side_info(client: &mut VictauriClient, side: &str) -> Value {
    let entries = sftp_entries(client).await;
    let sides = entries
        .first()
        .and_then(|entry| entry.pointer("/sides"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    sides
        .into_iter()
        .find(|info| info.pointer("/side").and_then(Value::as_str) == Some(side))
        .unwrap_or_else(|| panic!("探针里没有 {side} 这一侧"))
}

/// `app_state { probe: "sftp" }` 的原始列表。
async fn sftp_entries(client: &mut VictauriClient) -> Vec<Value> {
    let value = client
        .call_tool("app_state", json!({ "probe": "sftp" }))
        .await
        .expect("读不到 sftp probe —— 它注册进 lib.rs 了吗？");
    value.as_array().cloned().unwrap_or_default()
}
