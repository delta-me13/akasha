//! SSH 那两条 E2E 的**共用脚手架**（plan 0504 起，plan 0505 提出来）。
//!
//! 为什么要有它：`ssh_session`（直连）与 `ssh_jump`（经跳板）是**同一个场景的两个版本** ——
//! 都自己造库、自己起进程内的 SSH 服务端、都靠界面驱动。抄第二份的下场是两份慢慢分叉，
//! 而分叉的方向总是"其中一条以为验了"。
//!
//! ⚠️ `tests/` 下的模块每个测试目标各编译一份（同 `akasha-ssh/tests/support/`），
//! 所以"共享"只到这一层为止；跨 crate 的东西住在库里（那是那台服务端搬进
//! `akasha_ssh::testing` 的理由）。
//!
//! **不是测试目标**：`cargo` 只把 `tests/*.rs` 当目标，`tests/support/mod.rs` 是被各个目标
//! `mod support;` 引进来的普通模块 —— 也因此不会撞上 `justfile` 里那条"没接进 test-e2e 的目标"的
//! guard（它扫的是 `tests/*.rs`）。

#![allow(dead_code)] // 每个测试目标各取所需，用不到的辅助函数不该让 `-D warnings` 变红
#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use akasha_ssh::testing::{Observed, Running};
use akasha_store::pools::{hosts, known_hosts};
// `Connection` 从 store 那侧取而不是直接依赖 `rusqlite`：它是 store 的**公开类型**
// （`open` / `create` 的返回值就是它），而版本对齐由 store 一处负责。
use akasha_store::{Connection, Passphrase, VaultState, vault_state};
use serde_json::Value;
use victauri_test::VictauriClient;

/// 这些用例自己用的库口令。**不是**用户的。
pub const PASSPHRASE: &str = "e2e-ssh-session-passphrase";
/// 池里那些行共用的登录用户名。
pub const USER: &str = "e2e";
/// 关标签页之后要等的时间（SSH 的收尾有期限：`SHUTDOWN_DEADLINE` 是 5 s，留够余量）。
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);
/// 连上、开标签页这类动作的等待上限。
pub const CONNECT_TIMEOUT_MS: u64 = 30_000;

pub fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("Skipping: set VICTAURI_E2E=1 with your Tauri dev server running");
        return true;
    }
    false
}

/// 自己造的库：**析构时删掉**（断言失败也走得到 —— 测试里 panic 是 unwind）。
pub struct Fixture {
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

/// 连上 app、看清库在哪 / 是什么状态、必要时先锁上。
///
/// 返回 `None` = **这条用例必须跳过**：那儿已经有一个库，而且**不是**用这些用例的口令建的
/// —— 那是用户自己的数据，测试不许碰。
pub async fn connect_and_prepare() -> Option<(VictauriClient, Fixture, PathBuf)> {
    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— 用 `just test-e2e`（它会自己起 app）");

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
            return None;
        }
    };

    // ⚠️ 先锁定再动它：如果上一次运行留下了解锁状态，种子会与 app 抢同一个文件。
    let _ = client.invoke_command("vault_lock", None).await;

    Some((
        client,
        Fixture {
            path: path.clone(),
            remove_on_drop: ours_to_remove,
        },
        path,
    ))
}

/// 打开（必要时新建）库。
///
/// app 现在**没有**写主机池的 IPC 命令（那是后面的 plan），所以种子数据直接落到
/// `vault_status` 报出来的那个路径上 —— 与 E2E 写 `config.json` 是同一种做法
/// （都是"这台机器上的外部状态"）。调用之前库必须是**锁着**的
/// （[`connect_and_prepare`] 保证了这一点）。
pub fn open_vault(path: &Path) -> Connection {
    let mut passphrase = Passphrase::new(PASSPHRASE.as_bytes().to_vec()).unwrap();
    match vault_state(path).unwrap() {
        VaultState::Present => akasha_store::open(path, &mut passphrase).unwrap(),
        VaultState::Missing | VaultState::Empty => {
            akasha_store::create(path, &mut passphrase).unwrap()
        }
    }
}

/// 清掉一条同名主机行与某个 `(host, port)` 的 known_hosts 记录。
///
/// 为什么要清：这些用例**不依赖**"上一次跑干净了"（重跑一次 `cargo test --test …`
/// 不该因为脏数据而红）。
pub fn forget(conn: &Connection, name: &str, host: &str, port: u16) {
    for row in hosts::hosts(conn).unwrap() {
        if row.name == name {
            hosts::delete_host(conn, row.id).unwrap();
        }
    }
    known_hosts::forget_host(conn, host, port).unwrap();
}

/// 用这些用例自己的口令解锁。调用点刚锁过库，所以不必处理 `AlreadyUnlocked`。
pub async fn unlock(client: &mut VictauriClient) {
    client
        .invoke_command(
            "vault_unlock",
            Some(serde_json::json!({ "passphrase": PASSPHRASE })),
        )
        .await
        .expect("vault_unlock 调不通（口令对、库也在）");
}

/// 库里记着的 known_hosts 行（读的是**同一个文件**，所以它同时说明"库那一侧真的写了"）。
pub fn recorded_host_keys(path: &Path) -> Vec<known_hosts::KnownHost> {
    let mut passphrase = Passphrase::new(PASSPHRASE.as_bytes().to_vec()).unwrap();
    let conn = akasha_store::open(path, &mut passphrase).unwrap();
    known_hosts::known_hosts(&conn).unwrap()
}

/// `eval_js` 的返回可能把结果包在 `result` 里，也可能就是裸值 —— 两种都认。
pub fn payload(value: &Value) -> &Value {
    value.get("result").unwrap_or(value)
}

pub fn text(value: &Value) -> String {
    payload(value).as_str().unwrap_or("?").to_string()
}

/// 等一个 JS 表达式为真（有截止时间的轮询，**不是** sleep 猜）。
pub async fn wait_js(client: &mut VictauriClient, expression: &str, timeout_ms: u64, what: &str) {
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
pub async fn click(client: &mut VictauriClient, selector: &str, what: &str) {
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
pub async fn type_line(client: &mut VictauriClient, line: &str) {
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

/// 往一个受控输入框里填值。
///
/// ⚠️ React 的受控输入认的是**原生 setter + `input` 事件**（直接改 `.value` 它看不见），
/// 而且**必须与提交分成两次 `eval_js`**：要等它把这一次事件之后的重渲染提交完，
/// `onSubmit` 闭包里的值才是新的 —— 同一个 JS 任务里紧接着 `.click()` 会提交**上一次渲染**
/// 的旧值（空串）。这一点由 `ssh_session` 实测得出。
pub async fn fill_input(client: &mut VictauriClient, selector: &str, value: &str) {
    let selector = serde_json::to_string(selector).unwrap();
    let value = serde_json::to_string(value).unwrap();
    let js = format!(
        r#"(() => {{
  const input = document.querySelector({selector});
  if (!input) return false;
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, {value});
  input.dispatchEvent(new Event('input', {{ bubbles: true }}));
  return true;
}})()"#
    );
    let filled = client.eval_js(&js).await.unwrap();
    assert!(
        payload(&filled).as_bool().unwrap_or(false),
        "填不进 {selector}：{filled}"
    );
}

/// 在凭据提示里填一句口令（见 [`fill_input`] 的两条理由）。
pub async fn fill_secret(client: &mut VictauriClient, secret: &str) {
    fill_input(
        client,
        ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-secret",
        secret,
    )
    .await;
}

/// 回答提示的**剧本**：链上有哪两台、口令各是什么、各自的主机密钥指纹。
pub struct PromptScript<'a> {
    /// 这一轮问答结束时会话数（用来判"已经连上了，不用再答"）。
    pub tabs: usize,
    /// 跳板监听的端口 —— 用它认"这一问问的是哪一台"（口令按此分流）。
    pub jump_port: u16,
    pub jump_password: &'a str,
    pub target_password: &'a str,
    pub jump_fingerprint: &'a str,
    pub target_fingerprint: &'a str,
}

/// 回答提示**直到会话连上**（按类型答），并把"问过谁"记下来。
///
/// 为什么是循环而不是写死四步：链上每一跳各来一轮（密钥 + 口令），轮数取决于链上有几台、
/// 以及哪些已经记在库里 —— 写死数字会在链一变长时**静默少答一轮**，表现是"连不上"
/// 而不是"用例写错了"。
///
/// 两个用例（`ssh_jump` 与 `ssh_config_import`）共用它：它们问的是同一个问题
/// （"每一跳各问各的凭据"），抄第二份的下场是两份慢慢分叉。
pub async fn answer_prompts(client: &mut VictauriClient, script: PromptScript<'_>) -> Vec<String> {
    let mut asked = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    // 等"弹出任意一种提示"（超时也照常往下走：下一圈先看是不是已经连上了）。
    let any_prompt = "!!document.querySelector('.ssh-prompt[data-prompt-kind=\"hostKey\"]') \
                      || !!document.querySelector('.ssh-prompt[data-prompt-kind=\"credential\"]')";
    loop {
        if is_connected(client, script.tabs).await {
            return asked;
        }
        if std::time::Instant::now() >= deadline {
            // 超时要带上**界面当时说的话**（状态栏 + 报错行 + 此刻有没有提示），
            // 否则下一次只能猜是"没弹提示"还是"连不上"——实测这两件事长得一模一样。
            let status = text_of(client, ".tab-pane.is-active .terminal-status").await;
            let error = text_of(client, ".tab-pane.is-active .terminal-error").await;
            let hint = text_of(
                client,
                ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-hint",
            )
            .await;
            panic!(
                "提示问答没走完就连不上（问到过的：{asked:?}；状态栏={status:?} 报错={error:?} \
                 当前提示={hint:?}）"
            );
        }
        let _ = client
            .wait_for_expression(any_prompt, None, Some(5_000), None)
            .await;

        let fingerprint = text_of(
            client,
            ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-fingerprint",
        )
        .await;
        if !fingerprint.is_empty() {
            // 主机密钥：指纹必须与**某一台**服务端对得上（"问的是谁"要说得清）。
            assert!(
                fingerprint == script.jump_fingerprint || fingerprint == script.target_fingerprint,
                "提示里那串指纹不属于任何一台服务端：{fingerprint}"
            );
            asked.push(format!("hostKey:{fingerprint}"));
            click(
                client,
                ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-accept",
                "接受这把主机密钥",
            )
            .await;
            continue;
        }

        let target_text = text_of(
            client,
            ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-hint",
        )
        .await;
        if !target_text.is_empty() {
            // 口令按**问的是哪台**给：两跳的登录口令不一样。
            let secret = if target_text.contains(&format!(":{}", script.jump_port)) {
                script.jump_password
            } else {
                script.target_password
            };
            asked.push(format!("credential:{target_text}"));
            fill_secret(client, secret).await;
            click(
                client,
                ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-submit",
                "提交口令",
            )
            .await;
            continue;
        }
    }
}

/// 读某个选择器的文本（取不到就返回空串）。
pub async fn text_of(client: &mut VictauriClient, selector: &str) -> String {
    let literal = serde_json::to_string(selector).unwrap();
    text(
        &client
            .eval_js(&format!(
                "document.querySelector({literal})?.textContent ?? ''"
            ))
            .await
            .unwrap(),
    )
}

/// 活动标签页是不是"已连接"。
pub async fn is_connected(client: &mut VictauriClient, tabs: usize) -> bool {
    let expression = format!(
        "document.querySelectorAll('.tab').length === {tabs} && \
         !!document.querySelector('.tab-pane.is-active .terminal-status')?.textContent?.includes('已连接')"
    );
    client
        .wait_for_expression(&expression, None, Some(500), None)
        .await
        .map(|waited| waited.get("ok").and_then(Value::as_bool) == Some(true))
        .unwrap_or(false)
}

/// 等"+一个 SSH 标签页出现、且它说已连接"。
///
/// 失败时把**界面当时说的话**打出来（状态栏 + 报错行）—— 否则下一次只能猜是"没连上"
/// 还是"界面没更新"（实测这两件事长得一模一样）。
pub async fn wait_connected(client: &mut VictauriClient, tabs: usize, what: &str) {
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
    let status = text_of(client, ".tab-pane.is-active .terminal-status").await;
    let error = text_of(client, ".tab-pane.is-active .terminal-error").await;
    let titles = text(
        &client
            .eval_js(
                "Array.from(document.querySelectorAll('.tab-label'), (t) => t.textContent).join('|')",
            )
            .await
            .unwrap(),
    );
    panic!(
        "{what} 超时（{CONNECT_TIMEOUT_MS} ms）：状态栏={status:?} 报错={error:?} 标签页={titles:?}"
    );
}

/// 服务端记下来的事实（观察点的另一半）。
pub fn observed(server: &Running) -> Observed {
    server.shared.observed()
}
