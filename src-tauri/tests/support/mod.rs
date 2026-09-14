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
use std::time::{Duration, Instant};

use akasha_ssh::testing::{Observed, Running};
use akasha_store::pools::{hosts, known_hosts};
// `Connection` 从 store 那侧取而不是直接依赖 `rusqlite`：它是 store 的**公开类型**
// （`open` / `create` 的返回值就是它），而版本对齐由 store 一处负责。
use akasha_store::{Connection, Passphrase, VaultState, vault_state};
use serde_json::{Value, json};
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

/// 挑一个**当前空闲**的本地端口。
///
/// 为什么不能写死一个数字：转发规则要真的绑定端口（plan 0602 起），
/// 写死的端口一旦被开发机上别的东西占着，用例就会红在一条与本次改动无关的原因上。
/// 库的 `CHECK` 不允许 `bind_port = 0`（内核分配），所以只能用"先绑再放"。
/// ⚠️ 放掉与 app 绑上之间有极小的竞争窗口 —— 比"写死一个端口"小得多，且失败信息清楚
/// （绑定失败会带上地址）。
pub fn free_port() -> u16 {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("取空闲端口失败");
    held.local_addr().expect("取空闲端口地址失败").port()
}

// ── 隧道（plan 0601 起，两条 E2E 共用）────────────────────────────────────────

/// 打开隧道面板，并保证它**重新读一遍**规则池。
///
/// 两件事都在这一处解决，缺一条就是一条随执行顺序红的用例：
///
/// 1. `.tab-new-tunnel` 是**切换**（`src/App.tsx`），而各个 E2E 目标**共用一个 app** ——
///    上一个目标可能已经把面板留在打开状态，再点一下就把它关了；
/// 2. 面板的规则表是**挂载时读一次**的（验证壳层的行为，见 `TunnelPanel`），
///    所以上一个用例挂载的那一份里不会有本次种下的规则 —— 必须先卸下再挂上。
pub async fn open_tunnel_panel(client: &mut VictauriClient) {
    if !text_of(client, ".tunnel-panel").await.is_empty() {
        click(
            client,
            ".tunnel-close",
            "关闭隧道面板（好让它重新读一次池子）",
        )
        .await;
    }
    click(client, ".tab-new-tunnel", "打开隧道面板").await;
    wait_js(
        client,
        "!!document.querySelector('.tunnel-panel')",
        10_000,
        "隧道面板打开了",
    )
    .await;
}

/// `app_state { probe: "tunnels" }` 的原始列表。
pub async fn tunnel_entries(client: &mut VictauriClient) -> Vec<Value> {
    let value = client
        .call_tool("app_state", json!({ "probe": "tunnels" }))
        .await
        .expect("读不到 tunnels probe —— 它注册进 lib.rs 了吗？");
    value.as_array().cloned().unwrap_or_default()
}

/// 前端记下的 `tunnel_state` 事件（测试接口，见 `src/ipc/tunnels.ts`）。
pub async fn tunnel_events(client: &mut VictauriClient) -> Vec<Value> {
    let raw = client
        .eval_js("JSON.stringify(window.__akashaTunnels?.events ?? [])")
        .await
        .unwrap();
    let encoded = text(&raw);
    serde_json::from_str(&encoded).unwrap_or_default()
}

/// 等 probe 里某条隧道到达某个状态，返回它的 handle。
pub async fn wait_tunnel_state(client: &mut VictauriClient, rule_id: i64, want: &str) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        for entry in tunnel_entries(client).await {
            let matches_rule = entry.pointer("/ruleId").and_then(Value::as_i64) == Some(rule_id);
            let state = entry.pointer("/state").and_then(Value::as_str);
            if matches_rule && state == Some(want) {
                return entry
                    .pointer("/handle")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
            }
        }
        assert!(
            Instant::now() < deadline,
            "等不到规则 {rule_id} 变成 {want}：{:?}",
            tunnel_entries(client).await
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 等一条隧道从后端消失（停止之后）。
pub async fn wait_tunnel_gone(client: &mut VictauriClient, handle: u64) {
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        let left = tunnel_entries(client).await;
        if !left
            .iter()
            .any(|entry| entry.pointer("/handle").and_then(Value::as_u64) == Some(handle))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "停掉的隧道还在 probe 里：{left:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 边答提示边等某条**隧道**连上，返回 `(handle, 收到过哪些提示)`。
///
/// 与 [`answer_prompts`] 的区别只有"等到什么算完"：那一条等的是**终端标签页**说已连接，
/// 这一条等的是 probe 里那条隧道说 `connected`（隧道不是标签页，没有标签页可等）。
pub async fn connect_tunnel_through_prompts(
    client: &mut VictauriClient,
    rule_id: i64,
    fingerprint: &str,
    password: &str,
) -> (u64, Vec<String>) {
    connect_tunnel_until(client, rule_id, fingerprint, password, "connected").await
}

/// 同上，但由调用方说**等到哪个状态算完**。
///
/// 为什么需要这一条：远端监听那条路有一类失败**在连接之后**才发生（plan 0604 ——
/// 那个端口在服务端，请求失败时连接已经建起来了），于是"打开它然后等它失败"是一条
/// 正经的用例路径，而不是异常。把"等到什么"参数化，比在用例里再抄一份问答循环好。
///
/// 为什么不写死"先密钥后口令"两步：链上每一跳各来一轮（plan 0505 的教训），
/// 写死步数会在拓扑一变时**静默少答一轮**，表现是"连不上"而不是"用例写错了"。
pub async fn connect_tunnel_until(
    client: &mut VictauriClient,
    rule_id: i64,
    fingerprint: &str,
    password: &str,
    want: &str,
) -> (u64, Vec<String>) {
    let mut asked = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(90);
    let any_prompt = "!!document.querySelector('.ssh-prompt[data-prompt-kind=\"hostKey\"]') \
                      || !!document.querySelector('.ssh-prompt[data-prompt-kind=\"credential\"]')";
    loop {
        for entry in tunnel_entries(client).await {
            if entry.pointer("/ruleId").and_then(Value::as_i64) == Some(rule_id)
                && entry.pointer("/state").and_then(Value::as_str) == Some(want)
            {
                let handle = entry
                    .pointer("/handle")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                return (handle, asked);
            }
        }
        if Instant::now() >= deadline {
            let problem = text_of(client, ".tunnel-failure").await;
            panic!(
                "提示问答没走完就等不到 {want}（问到过的：{asked:?}；面板上的失败={problem:?}）"
            );
        }
        let _ = client
            .wait_for_expression(any_prompt, None, Some(3_000), None)
            .await;

        let shown = text_of(
            client,
            ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-fingerprint",
        )
        .await;
        if !shown.is_empty() {
            assert_eq!(shown, fingerprint, "提示里的指纹必须就是服务端的");
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
            fill_secret(client, password).await;
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

/// 等某个选择器的文本里出现某段话（失败时把当时那段文本打出来）。
///
/// 用于"失败必须在界面上看得见"这一类判据：读的是**界面渲染出来的**那句话，
/// 而不是后端错误对象 —— 用户看到的是前者。
pub async fn wait_text_contains(client: &mut VictauriClient, selector: &str, needle: &str) {
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        let shown = text_of(client, selector).await;
        if shown.contains(needle) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "等不到 {selector} 里出现 {needle:?}：{shown:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
