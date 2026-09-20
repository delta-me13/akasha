//! plan 0506 的**端到端**验收：真 app 上从一份 `~/.ssh/config` 导入，再用**导入进来的那些行**
//! 开一个经跳板的会话。
//!
//! 判据（ROADMAP 原文）=「含 `Match` 的配置产生**明确报错**，不是静默误解析」。它有两半，
//! 各一份 fixture：
//!
//! | fixture | 期望 |
//! |---|---|
//! | 正常的（跳板 + 目标 + `Host *` 里的局部指令） | 条目进池、链挂上、**能真连** |
//! | 含 `Match` 的 | 界面逐条报错带行号，而**池里一行都没多** |
//!
//! 为什么"能真连"必须在这条用例里：phase 5 把 0506 排在 0505 之后的理由就是"导入进来的跳板
//! 要能真用"。所以这里不满足于"报告里有 2 条"，而是**拿那两行去连** ——
//! `akasha-e2e-inner.invalid` 只对跳板可见的老办法（与 `ssh_jump.rs` 同一套构造）。
//!
//! ## 这台机器上跑它需要什么
//!
//! * app 起得来（`just test-e2e` 自己会起）；
//! * `akasha-e2e-import.invalid` 在本机解析**失败**（RFC 2606，用例自己断言）；
//! * **ssh-agent 里没有一大把钥匙**：导入的条目按 `publickey` 落库、钥匙在 agent 里，
//!   所以认证会先试 agent —— 钥匙多到撞上假服务端的 `max_auth_attempts`（russh 默认 10）
//!   就轮不到口令那一档了。用例因此把 `SSH_AUTH_SOCK` 打出来备查。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use akasha_lib::ssh::testing::{Relay, Running, ServerOptions, start};
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, PromptScript, USER, answer_prompts, click, connect_and_prepare, fill_input,
    forget, observed, open_vault, text, text_of, type_line, unlock, wait_connected, wait_js,
};
use victauri_test::VictauriClient;

/// 池里那两行的名字 —— 就是 fixture 里的 `Host` 模式（导入不另起名字）。
const JUMP_NAME: &str = "e2e-config-jump";
const TARGET_NAME: &str = "e2e-config-target";

/// **只有跳板认识**的名字（与 `ssh_jump.rs` 同一个构造）。
///
/// ⚠️ **名字必须与别的 E2E 目标不一样**（这里是 `akasha-e2e-**import**.invalid`）：全部目标跑在
/// **同一个 app 进程**里，而内存凭据缓存的键是 `(host, port, user, 认证方式)`（ADR-0003 D8）——
/// 名字撞上就意味着撞上**上一个目标留下的口令**。症状很隐蔽：服务端拒绝缓存里那句之后，
/// 整条认证**失败**而不是重新问一次（`password_step` 只在被拒时 `forget`，而这一条连接已经走完了），
/// 表现是"提示问答没走完就连不上"。
/// 端口撞不会有事（跳板的端口每次随机），**名字会**。
const INNER_NAME: &str = "akasha-e2e-import.invalid";
const INNER_PORT: u16 = 22;

const JUMP_PASSWORD: &str = "e2e-config-jump-password";
const TARGET_PASSWORD: &str = "e2e-config-target-password";

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

/// fixture 落在 `target/` 下（与库那一侧的 fixture 同一个地方，不会进仓库）。
fn fixture_path(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/e2e-config");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// 一份**正常**的配置：全局段里的两条局部指令（认得、但不生效）+ 跳板 + 目标。
///
/// ⚠️ `Host *` 写在**最前面**，这是真配置里最常见的形状 —— 而按"首次取到的值生效"，
/// 它里面的 `Port` / `HostName` 会压住后面所有具体条目。所以它只放保活与 `IdentityFile`
/// （它们本来就不是我们要导入的东西），**不放**端口。
fn write_good(jump_port: u16) -> PathBuf {
    let path = fixture_path("good.conf");
    std::fs::write(
        &path,
        format!(
            "# plan 0506 的 E2E fixture：一份正常配置\n\
             Host *\n\
             \x20   ServerAliveInterval 30\n\
             \x20   IdentityFile ~/.ssh/id_e2e\n\
             Host {JUMP_NAME}\n\
             \x20   HostName 127.0.0.1\n\
             \x20   Port {jump_port}\n\
             \x20   User {USER}\n\
             Host {TARGET_NAME}\n\
             \x20   HostName {INNER_NAME}\n\
             \x20   Port {INNER_PORT}\n\
             \x20   User {USER}\n\
             \x20   ProxyJump {JUMP_NAME}\n"
        ),
    )
    .unwrap();
    path
}

/// 含 `Match` 的那一份：**整份不导入**（判据本身）。
fn write_with_match() -> PathBuf {
    let path = fixture_path("with-match.conf");
    std::fs::write(
        &path,
        format!(
            "# plan 0506 的 E2E fixture：这一份必须让整个导入失败\n\
             Host {TARGET_NAME}\n\
             \x20   HostName {INNER_NAME}\n\
             Match host {TARGET_NAME}\n\
             \x20   User matched\n"
        ),
    )
    .unwrap();
    path
}

/// 池里的行（app 说的，不是我们直接读库说的）。
async fn pool(client: &mut VictauriClient) -> Vec<Value> {
    client
        .invoke_command("vault_hosts", None)
        .await
        .expect("vault_hosts 调不通")
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn row<'a>(rows: &'a [Value], name: &str) -> &'a Value {
    rows.iter()
        .find(|row| row.pointer("/name").and_then(Value::as_str) == Some(name))
        .unwrap_or_else(|| panic!("池里没有 {name} 这一行：{rows:#?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_imported_config_reaches_a_host_only_the_bastion_can_see() {
    if support::skip_unless_e2e() {
        return;
    }

    // ── 1. 两台服务端（都在**本进程**里）────────────────────────────────────
    let (jump, target) = two_hosts().await;
    eprintln!(
        "构造: 跳板=127.0.0.1:{} 跳板指纹={} 跳板口令={JUMP_PASSWORD} \
         目标={INNER_NAME}:{INNER_PORT} 目标地址={} 目标指纹={} 目标口令={TARGET_PASSWORD} \
         ssh_agent={}",
        jump.addr.port(),
        jump.fingerprint,
        target.addr,
        target.fingerprint,
        std::env::var_os("SSH_AUTH_SOCK")
            .map_or_else(|| "无".to_owned(), |v| v.to_string_lossy().into_owned()),
    );

    // ── 2. "只对跳板机可见"的那一半（与 `ssh_jump.rs` 同一条构造前提）──────
    let resolved = (INNER_NAME, INNER_PORT).to_socket_addrs();
    assert!(
        resolved.is_err(),
        "那个目标名在本机解析得出来（{:?}）—— 这条用例的构造前提就不成立了",
        resolved.map(|mut addrs| addrs.next())
    );
    eprintln!("构造: 目标名={INNER_NAME} 本机解析=失败");

    // ── 3. 库 + 清干净 + 解锁 ───────────────────────────────────────────────
    let Some((mut client, _fixture, path)) = connect_and_prepare().await else {
        return;
    };
    let good = write_good(jump.addr.port());
    let bad = write_with_match();
    let missing = fixture_path("does-not-exist.conf");
    let _ = std::fs::remove_file(&missing);

    {
        // 重跑不依赖"上次跑干净了"：把这两行删掉（目标引用着跳板，顺序不能反）。
        let conn = open_vault(&path);
        forget(&conn, TARGET_NAME, INNER_NAME, INNER_PORT);
        forget(&conn, JUMP_NAME, "127.0.0.1", jump.addr.port());
    }
    unlock(&mut client).await;

    // ── 4. 界面：打开选择器 → 展开导入面板 → 填路径 → 点导入 ────────────────
    click(&mut client, ".tab-new-ssh", "打开主机选择器").await;
    wait_js(
        &mut client,
        "!!document.querySelector('[data-import-panel]')",
        10_000,
        "选择器里有导入面板",
    )
    .await;

    // 支持集的字样要在**点之前**就看得见（`scope.md` §8 风险 4 第 2 条：UI 上列出边界）。
    let supported = text(
        &client
            .eval_js(
                "document.querySelector('[data-import-supported]')?.dataset.importSupported ?? ''",
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        supported, "Host,HostName,User,Port,IdentityFile,ProxyJump",
        "界面要把支持的六条列出来（后端 `sshconfig::supported()` 就是这六条）"
    );

    click(&mut client, ".config-import-summary", "展开导入面板").await;
    fill_input(&mut client, "[data-import-path]", &good.to_string_lossy()).await;
    click(&mut client, "[data-import-run]", "导入").await;
    wait_js(
        &mut client,
        "!!document.querySelector('[data-import-report]')",
        15_000,
        "导入报告出现了",
    )
    .await;

    let counts = text_of(&mut client, "[data-import-counts]").await;
    eprintln!("报告: 计数={counts}");
    assert!(
        counts.contains("新增 2")
            && counts.contains(&good.file_name().unwrap().to_string_lossy().to_string()),
        "报告要说清读了哪个文件、新增了几条（拿到 {counts:?}）"
    );

    // "没生效"的东西要**逐条**露面 —— 受限子集不静默的那一半。
    let ignored = text_of(&mut client, "[data-import-ignored]").await;
    assert!(
        ignored.contains("serveraliveinterval") && ignored.contains("identityfile"),
        "两条局部指令都要被列出来（拿到 {ignored:?}）"
    );
    eprintln!("报告: 未生效={ignored}");

    let rows = pool(&mut client).await;
    eprintln!("池: 行数={}", rows.len());
    let jump_row = row(&rows, JUMP_NAME);
    let jump_id = jump_row.pointer("/id").and_then(Value::as_u64).unwrap();
    let target_row = row(&rows, TARGET_NAME);
    assert_eq!(
        target_row.pointer("/jumpId").and_then(Value::as_u64),
        Some(jump_id),
        "目标那一行的跳板该指向导入进来的跳板那一行"
    );
    assert_eq!(
        target_row.pointer("/host").and_then(Value::as_str),
        Some(INNER_NAME)
    );
    assert_eq!(
        target_row.pointer("/user").and_then(Value::as_str),
        Some(USER)
    );
    assert_eq!(
        target_row.pointer("/auth").and_then(Value::as_str),
        Some("publicKey"),
        "`IdentityFile` 让它落成公钥认证（私钥本身不导入，钥匙在 agent 里）"
    );
    // ⚠️ 不写 `rows.len() == 2`：库里还留着别的 E2E 目标建的行（每个目标只管自己那几个名字），
    // 所以这里记住"导入后的总数"，用它去对账下一步"一行都没多"。
    let after_good = rows.len();
    assert!(after_good >= 2, "池里该有刚导入的那两行：{rows:#?}");

    // ── 5. 含 `Match` 的那一份：**逐条报错 + 池里一行都没多**（判据本身）────
    fill_input(&mut client, "[data-import-path]", &bad.to_string_lossy()).await;
    click(&mut client, "[data-import-run]", "导入含 Match 的配置").await;
    wait_js(
        &mut client,
        "!!document.querySelector('[data-import-problems]')",
        15_000,
        "整份被拒的那几处列了出来",
    )
    .await;
    let problems = text_of(&mut client, "[data-import-problems]").await;
    eprintln!("报告: 拒绝原因={problems}");
    assert!(
        problems.contains("match"),
        "报错要点出是哪条指令（拿到 {problems:?}）"
    );
    let line = text(
        &client
            .eval_js(
                "document.querySelector('[data-import-problems] li')?.dataset.importProblemLine ?? ''",
            )
            .await
            .unwrap(),
    );
    assert_eq!(line, "4", "还要说得清是**第几行**（拿到 {line:?}）");
    assert!(
        !client
            .eval_js("!!document.querySelector('[data-import-report]')")
            .await
            .map(|value| support::payload(&value).as_bool().unwrap_or(false))
            .unwrap_or(true),
        "整份被拒时不该有'导入报告'—— 一个都没导进去"
    );
    assert_eq!(
        pool(&mut client).await.len(),
        after_good,
        "整份被拒时池里一行都不该多"
    );

    // 文件读不到 ≠ 导入了 0 台：这两件事在界面上要是两句话。
    fill_input(
        &mut client,
        "[data-import-path]",
        &missing.to_string_lossy(),
    )
    .await;
    click(&mut client, "[data-import-run]", "导入一个不存在的文件").await;
    wait_js(
        &mut client,
        "!!document.querySelector('[data-import-problem]')",
        15_000,
        "读不到文件时给了明确说法",
    )
    .await;
    let unreadable = text_of(&mut client, "[data-import-problem]").await;
    eprintln!("报告: 读取失败={unreadable}");
    assert!(
        unreadable.contains("读不到") && unreadable.contains("does-not-exist.conf"),
        "要说清读不到哪个文件（拿到 {unreadable:?}）"
    );

    // ── 6. 拿**导入进来的那一行**开一个经跳板的会话 ─────────────────────────
    wait_js(
        &mut client,
        &format!(
            "!!document.querySelector('.host-picker-item[data-host-id=\"{}\"]')",
            target_row.pointer("/id").and_then(Value::as_u64).unwrap()
        ),
        10_000,
        "导入之后选择器里有目标那一行",
    )
    .await;
    let badge = text_of(
        &mut client,
        &format!(".host-picker-item[data-host-id=\"{jump_id}\"] .host-picker-target"),
    )
    .await;
    eprintln!("界面: 跳板={badge}");
    click(
        &mut client,
        &format!(
            ".host-picker-item[data-host-id=\"{}\"]",
            target_row.pointer("/id").and_then(Value::as_u64).unwrap()
        ),
        "选那台经跳板的主机",
    )
    .await;

    let asked = answer_prompts(
        &mut client,
        PromptScript {
            tabs: 2,
            jump_port: jump.addr.port(),
            jump_password: JUMP_PASSWORD,
            target_password: TARGET_PASSWORD,
            jump_fingerprint: &jump.fingerprint,
            target_fingerprint: &target.fingerprint,
        },
    )
    .await;
    eprintln!("界面: 提示数={}", asked.len());
    wait_connected(&mut client, 2, "由导入的配置开出来的 SSH 会话").await;

    // ── 7. 跳板那一半：它被要求连的正是那个只有它认识的名字 ─────────────────
    let jump_seen = observed(&jump);
    assert_eq!(
        jump_seen.direct_tcpip.len(),
        1,
        "跳板应当恰好被要求转发一次，实际：{:?}",
        jump_seen.direct_tcpip
    );
    assert_eq!(jump_seen.direct_tcpip[0].host, INNER_NAME);

    // ── 8. 字节能双向流：到了**目标** ───────────────────────────────────────
    type_line(&mut client, "echo via-imported-config\n").await;
    wait_js(
        &mut client,
        "window.__akashaTerminal.screenText(200).includes('via-imported-config')",
        support::CONNECT_TIMEOUT_MS,
        "目标的回声出现在了终端上",
    )
    .await;
    let seen = String::from_utf8_lossy(&observed(&target).shell_data).to_string();
    assert!(
        seen.contains("echo via-imported-config"),
        "目标服务端没收到那行字节（收到 {seen:?}）"
    );

    // ── 9. 关标签页零残留（这条会话与别的会话同一个收尾路径）────────────────
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

    // ── 10. 收尾 ────────────────────────────────────────────────────────────
    let _ = client.invoke_command("vault_lock", None).await;
}
