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
//! ## 为什么服务端在**测试进程**里
//!
//! app 是另一个进程，它连过来 —— 于是"服务端看到了什么"与"界面上看到了什么"是同一件事的
//! 两种观察，可以在一条用例里对账。服务端本体是 `akasha_ssh::testing`（生产代码别用它，
//! 理由写在那个模块的文档里）。
//!
//! ## 数据从哪来
//!
//! app 现在**没有**写主机池的 IPC 命令（那是后面的 plan），所以种子数据直接调
//! `akasha-store` 的函数落到 `vault_status` 报出来的那个路径上 —— 与 E2E 写 `config.json`
//! 是同一种做法（都是"这台机器上的外部状态"）。⚠️ 动手之前先确认那**不是用户的真库**：
//! 是的话**显式跳过**，绝不拿测试口令去动它。
//!
//! ## 收尾
//!
//! 自己造的库自己删（`Fixture` 的 `Drop`，断言失败也走得到）；测试用的口令**只**用于
//! "这个库是不是我们自己造的"这个判断。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use akasha_ssh::testing::{Running, ServerOptions, start};
use akasha_store::pools::{hosts, known_hosts};
use akasha_store::{Passphrase, VaultState, vault_state};
use serde_json::{Value, json};
use victauri_test::VictauriClient;

/// 这条用例自己用的口令（库）与登录口令（SSH）。**不是**用户的。
const PASSPHRASE: &str = "e2e-ssh-session-passphrase";
const PASSWORD: &str = "e2e-ssh-login-password";
/// 池里那一行的名字（也是标签页标题）。
const HOST_NAME: &str = "e2e-ssh-target";
const USER: &str = "e2e";
/// 关标签页之后要等的时间（SSH 的收尾有期限：`SHUTDOWN_DEADLINE` 是 5 s，留够余量）。
const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);
/// 连上、开标签页这类动作的等待上限。
const CONNECT_TIMEOUT_MS: u64 = 30_000;

fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("Skipping: set VICTAURI_E2E=1 with your Tauri dev server running");
        return true;
    }
    false
}

/// 自己造的库：**析构时删掉**（断言失败也走得到 —— 测试里 panic 是 unwind）。
struct Fixture {
    path: PathBuf,
    remove_on_drop: bool,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// 这个库能不能用**我们自己的口令**打开 —— 也就是"它是不是我们造的那个"。
fn is_ours(path: &Path) -> bool {
    let mut passphrase = Passphrase::new(PASSPHRASE.as_bytes().to_vec()).unwrap();
    akasha_store::open(path, &mut passphrase).is_ok()
}

/// 建（或打开）库，并在里面把这次要连的那一行摆好。
///
/// 可重复运行：同名主机行与它的 known_hosts 记录先清掉 —— 于是这条用例**不依赖**
/// "上一次跑干净了"（重跑一次 `cargo test --test ssh_session` 不该因为脏数据而红）。
fn seed(path: &Path, port: u16) -> PathBuf {
    let mut passphrase = Passphrase::new(PASSPHRASE.as_bytes().to_vec()).unwrap();
    let conn = match vault_state(path).unwrap() {
        VaultState::Present => akasha_store::open(path, &mut passphrase).unwrap(),
        VaultState::Missing | VaultState::Empty => {
            akasha_store::create(path, &mut passphrase).unwrap()
        }
    };

    // 先清干净（重跑）：主机行按名字找，known_hosts 按 (host, port) 清。
    for row in hosts::hosts(&conn).unwrap() {
        if row.name == HOST_NAME {
            hosts::delete_host(&conn, row.id).unwrap();
        }
    }
    known_hosts::forget_host(&conn, "127.0.0.1", port).unwrap();

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

/// 库里记着的 known_hosts 行数（读的是**同一个文件**，所以它同时说明"库那一侧真的写了"）。
fn recorded_host_keys(path: &Path) -> Vec<known_hosts::KnownHost> {
    let mut passphrase = Passphrase::new(PASSPHRASE.as_bytes().to_vec()).unwrap();
    let conn = akasha_store::open(path, &mut passphrase).unwrap();
    known_hosts::known_hosts(&conn).unwrap()
}

/// `eval_js` 的返回可能把结果包在 `result` 里，也可能就是裸值 —— 两种都认。
fn payload(value: &Value) -> &Value {
    value.get("result").unwrap_or(value)
}

fn text(value: &Value) -> String {
    payload(value).as_str().unwrap_or("?").to_string()
}

/// 等一个 JS 表达式为真（有截止时间的轮询，**不是** sleep 猜）。
async fn wait_js(client: &mut VictauriClient, expression: &str, timeout_ms: u64, what: &str) {
    let waited = client
        .wait_for_expression(expression, None, Some(timeout_ms), None)
        .await
        .unwrap();
    assert_eq!(
        waited.get("ok").and_then(Value::as_bool),
        Some(true),
        "{what} 超时（{timeout_ms} ms）：{waited}"
    );
}

/// 点一下某个选择器选中的元素（点不到就断言失败 —— 那说明界面与用例对不上了）。
async fn click(client: &mut VictauriClient, selector: &str, what: &str) {
    let literal = serde_json::to_string(selector).unwrap();
    let js = format!(
        "(() => {{ const el = document.querySelector({literal}); if (!el) return false; el.click(); return true; }})()"
    );
    let clicked = client.eval_js(&js).await.unwrap();
    assert!(
        payload(&clicked).as_bool().unwrap_or(false),
        "{what}：点不到 {selector}（返回 {clicked}）"
    );
}

/// 把当前活动标签页里的一行敲进终端（与 `tab_close` 同一手法）。
async fn type_line(client: &mut VictauriClient, line: &str) {
    let literal = serde_json::to_string(line).unwrap();
    let js = format!(
        r#"(() => {{
  const textarea = document.querySelector('.tab-pane.is-active .xterm-helper-textarea');
  if (!textarea) return false;
  textarea.focus();
  textarea.dispatchEvent(new InputEvent('input', {{ data: {literal}, inputType: 'insertText' }}));
  return true;
}})()"#
    );
    let sent = client.eval_js(&js).await.unwrap();
    assert!(
        payload(&sent).as_bool().unwrap_or(false),
        "按键没能送进 xterm：{sent}"
    );
}

/// 在凭据提示里填一句口令。
///
/// ⚠️ **必须分两次 `eval_js`**（填一次、提交一次）：React 的受控输入要等它把 `input` 事件
/// 之后的那次重渲染提交完，`onSubmit` 闭包里的 `secret` 才是新值 —— 同一个 JS 任务里
/// 紧接着 `.click()` 会提交**上一次渲染**的空串。
async fn fill_secret(client: &mut VictauriClient, secret: &str) {
    let literal = serde_json::to_string(secret).unwrap();
    let js = format!(
        r#"(() => {{
  const input = document.querySelector('.ssh-prompt[data-prompt-kind="credential"] .ssh-prompt-secret');
  if (!input) return false;
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, {literal});
  input.dispatchEvent(new Event('input', {{ bubbles: true }}));
  return true;
}})()"#
    );
    let filled = client.eval_js(&js).await.unwrap();
    assert!(
        payload(&filled).as_bool().unwrap_or(false),
        "口令框没有出现在预算里：{filled}"
    );
}

/// 等"+一个 SSH 标签页出现、且它说已连接"。
///
/// 失败时把**界面当时说的话**打出来（状态栏 + 报错行）—— 否则下一次只能猜是"没连上"
/// 还是"界面没更新"（实测这两件事长得一模一样）。
async fn wait_connected(client: &mut VictauriClient, tabs: usize, what: &str) {
    let expression = format!(
        "document.querySelectorAll('.tab').length === {tabs} && \
         !!document.querySelector('.tab-pane.is-active .terminal-status')?.textContent?.includes('已连接')"
    );
    let waited = client
        .wait_for_expression(&expression, None, Some(CONNECT_TIMEOUT_MS), None)
        .await
        .unwrap();
    if waited.get("ok").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let status = text(
        &client
            .eval_js(
                "document.querySelector('.tab-pane.is-active .terminal-status')?.textContent ?? ''",
            )
            .await
            .unwrap(),
    );
    let error = text(
        &client
            .eval_js(
                "document.querySelector('.tab-pane.is-active .terminal-error')?.textContent ?? ''",
            )
            .await
            .unwrap(),
    );
    let titles = text(
        &client
            .eval_js("Array.from(document.querySelectorAll('.tab-label'), (t) => t.textContent).join('|')")
            .await
            .unwrap(),
    );
    panic!(
        "{what} 超时（{CONNECT_TIMEOUT_MS} ms）：状态栏={status:?} 报错={error:?} 标签页={titles:?}"
    );
}

/// 服务端记下来的事实（观察点的另一半）。
fn observed(server: &Running) -> akasha_ssh::testing::Observed {
    server.shared.observed()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ssh_session_flows_bytes_and_asks_for_a_credential_once() {
    if skip_unless_e2e() {
        return;
    }

    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— 用 `just test-e2e`（它会自己起 app）");

    // ── 1. 服务端（在**本进程**里）：随机端口 + 只认口令 ─────────────────────
    let server = start(ServerOptions::password(PASSWORD)).await;
    eprintln!(
        "服务端：127.0.0.1:{} 指纹 {}",
        server.addr.port(),
        server.fingerprint
    );

    // ── 2. 库在哪、现在是什么状态 ────────────────────────────────────────────
    let status = client
        .invoke_command("vault_status", None)
        .await
        .expect("vault_status 调不通 —— 它登记进 bindings.rs 了吗？");
    let path = PathBuf::from(
        status
            .pointer("/path")
            .and_then(Value::as_str)
            .expect("返回值里必须有 path"),
    );
    let state = status
        .pointer("/state")
        .and_then(Value::as_str)
        .expect("返回值里必须有 state");
    let ours_to_remove = match state {
        "missing" | "empty" => true,
        "present" if is_ours(&path) => true, // 上一次跑留下的
        other => {
            eprintln!(
                "跳过：{} 上已经有一个库（state={other}），而且它**不是**用这条用例的口令建的 —— \
                 那是用户自己的数据，测试不许碰它",
                path.display()
            );
            return;
        }
    };
    let _fixture = Fixture {
        path: path.clone(),
        remove_on_drop: ours_to_remove,
    };
    // ⚠️ 先锁定再动它：如果上一次运行留下了解锁状态，种子会与 app 抢同一个文件。
    let _ = client.invoke_command("vault_lock", None).await;

    // ── 3. 种子数据 + 解锁 ─────────────────────────────────────────────────
    seed(&path, server.addr.port());

    // 上面刚锁过，所以这里必然是"从锁着到解开"—— 不需要 `AlreadyUnlocked` 那条分支。
    client
        .invoke_command("vault_unlock", Some(json!({ "passphrase": PASSPHRASE })))
        .await
        .expect("vault_unlock 调不通（口令对、库也在）");

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
        .expect("HostView 里必须有 id");
    eprintln!("主机池：{listed}");

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
    eprintln!("界面：已从主机选择器里选中 id={host_id}（选之前 {before_tabs} 个标签页）");

    // ── 5. 未知主机密钥：**问**，而且给出的指纹要能核对 ──────────────────────
    wait_js(
        &mut client,
        "!!document.querySelector('.ssh-prompt[data-prompt-kind=\"hostKey\"]')",
        CONNECT_TIMEOUT_MS,
        "没见过的主机密钥要弹出来问用户",
    )
    .await;
    let shown = text(
        &client
            .eval_js("document.querySelector('.ssh-prompt-fingerprint')?.textContent ?? ''")
            .await
            .unwrap(),
    );
    assert_eq!(
        shown, server.fingerprint,
        "提示里那串指纹必须就是服务端的（用户拿它去核对，给错等于没给）"
    );
    eprintln!("SSH 提示：hostKey 指纹与上面一致（{shown}）");

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
    eprintln!("SSH 提示：credential（password）");
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
    eprintln!("终端回声：ssh-hello（服务端也收到了同一串）");

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
        "库里 known_hosts：{} 行（{}:{} {}）",
        recorded.len(),
        row.host,
        row.port,
        row.key_type
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
    eprintln!("第二个会话：0 次提示，服务端第 2 次收到同一句口令");

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
            payload(&closed).as_bool().unwrap_or(false),
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
    eprintln!("关标签页：sessions probe = {sessions}");

    // 对端：服务端看到两条连接都断了（这是"连接真的没了"的唯一外部证据）。
    let deadline = std::time::Instant::now() + CLOSE_TIMEOUT;
    loop {
        let closed = observed(&server).sessions_closed;
        if closed >= 2 {
            eprintln!("关标签页：服务端看到 {closed} 条连接断开");
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
