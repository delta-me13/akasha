//! plan 0302 的**端到端**验收：**点叉 = 隐藏而非销毁** —— 进程、会话、终端缓冲都还在。
//!
//! 走的是**真实关窗路径**：Victauri 的 `window manage close` → `Window::close()` →
//! tauri 的 `CloseRequested`（`AGENTS.md` §7 的反面例子正是"绕过被测的那一段"）。
//!
//! ⚠️ **前提是"托盘真的建成了"**：配置要收托盘、但这台机器上建不起托盘时，本步的语义是
//! **降级为直接退出**（问题 #60）—— 那种情况下这条用例**显式跳过**，判据来自 app 自己上报的
//! probe（`app_state { probe: "lifecycle" }`），不是"猜它可能不满足"。
//! 也就是说：CI（xvfb，没有会话总线 / 没有 runtime dir）上它会跳过 —— **托盘宿主是它的前提**，
//! 这一点与托盘本身没有自动化门禁是同一件事（`STATUS.md` 的「待验证」）。
//!
//! 判据四层，缺一不可：
//!
//! 1. **进程还在** —— 关窗不是退出；
//! 2. **窗口不可见** —— `window get_state` 的 `visible == false`："藏起来了"要机器可查，
//!    不能靠人看（问题 #64）；
//! 3. **会话还在** —— 一个明确忽略 SIGHUP 的后台进程仍然活着。
//!    ⚠️ **这是预期行为，不是泄漏**（`AGENTS.md` §3.3：收托盘时窗口关闭**不是回收时机**）；
//! 4. **终端没被重建** —— 隐藏期间屏幕内容照旧读得到，显示回来之后还能继续敲命令。
//!    重建窗口会换掉会话与回滚缓冲，那时第 1、3 条判据就都不代表"原来那个终端"了。
//!
//! 本文件是测试，unwrap 在这里就是断言手段。

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use victauri_test::VictauriClient;

/// 忽略 SIGHUP 的后台探针（与 `tab_close` / `exit_residue` 取同一个最坏情况）：
/// 会话真被收掉时它必须消失，只是"窗口藏起来"时它必须活着。
const PROBE: &str = "sh -c 'trap \"\" HUP; echo AKPROBE=$$; exec sleep 600' &";

/// `eval_js` 的返回可能把结果包在 `result` 里，也可能就是裸值 —— 两种都认。
fn payload(value: &serde_json::Value) -> &serde_json::Value {
    value.get("result").unwrap_or(value)
}

fn ok(value: &serde_json::Value) -> bool {
    payload(value).as_bool().unwrap_or(false)
}

fn text(value: &serde_json::Value) -> String {
    payload(value).as_str().unwrap_or("?").to_string()
}

/// 把一行敲进**当前活动标签页**的终端（真实输入路径：见 `terminal_render.rs` 的说明）。
fn type_js(line: &str) -> String {
    // 行尾补 CR（0x0D）：终端线上的 Enter 就是这个字节 —— POSIX 的行规程用 `ICRNL` 把它
    // 折成 NL，而 Windows 的 ConPTY 只认 CR（送 LF 在那边既不提交命令行也不回显）。
    let literal =
        serde_json::to_string(&format!("{line}\r")).expect("文本无法转成 JS 字符串字面量");
    format!(
        r#"(() => {{
  const textarea = document.querySelector('.tab-pane.is-active .xterm-helper-textarea');
  if (!textarea) return false;
  textarea.focus();
  textarea.dispatchEvent(new InputEvent('input', {{ data: {literal}, inputType: 'insertText' }}));
  return true;
}})()"#
    )
}

async fn type_line(client: &mut VictauriClient, line: &str) {
    let sent = client.eval_js(&type_js(line)).await.unwrap();
    assert!(ok(&sent), "按键没能送进 xterm：{sent}");
}

/// 屏幕上出现 `needle` 的 JS 条件。
fn screen_has(needle: &str) -> String {
    format!(
        "window.__akashaTerminal.screenText(200).includes({})",
        serde_json::to_string(needle).expect("文本无法转成 JS 字面量")
    )
}

/// 等一个 JS 表达式为真（有截止时间的轮询，**不是** sleep 猜）。
async fn wait_js(client: &mut VictauriClient, expression: &str, timeout_ms: u64, what: &str) {
    let waited = client
        .wait_for_expression(expression, None, Some(timeout_ms), None)
        .await
        .unwrap();
    assert_eq!(
        waited.get("ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "{what} 超时（{timeout_ms} ms）：{waited}"
    );
}

/// 这个平台能不能看一个外部进程的存活。
///
/// ⚠️ **只有 Linux 能**：判据读 `/proc/<pid>/stat` 的 state（`/proc/<pid>` 存在 ≠ 活着：
/// 僵尸也有目录项，问题 #48）。macOS / Windows 没有 `/proc`，也没有等价的读数 ——
/// 那两处显式跳过进程级判据，改用 `sessions` probe 那条与平台无关的断言（见
/// `registered_sessions`）。
fn process_death_visible() -> bool {
    cfg!(target_os = "linux")
}

/// 进程是否**真的**活着。非 Linux 上没有这一档读数（见 `process_death_visible`）——
/// 一律返回 `true`，让"活着"那几条断言退化成恒真，调用方不必写两份。
fn alive(pid: u32) -> bool {
    if !process_death_visible() {
        return true;
    }
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return false;
    };
    !matches!(rest.split_whitespace().next(), None | Some("Z"))
}

/// 会话注册表里现在登记着几个会话（`sessions` probe 的 `registered`）。
///
/// 这是"关窗默认只是把窗口藏起来、**不碰会话**"那条判据与平台无关的一半 ——
/// 会话被多收一次的话，它在这里就少一个；多收两次（关窗 + 退出）也不会多出来。
async fn registered_sessions(client: &mut VictauriClient) -> u64 {
    client
        .app_state(Some("sessions"))
        .await
        .expect("app_state { probe: \"sessions\" } 调不通 —— probe 注册上了吗？")
        .pointer("/registered")
        .and_then(serde_json::Value::as_u64)
        .expect("sessions probe 的返回里必须有 registered")
}

fn waits_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    condition()
}

/// app 自己的 pid：discovery 目录的名字就是它（问题 #40）。
fn app_pid_for_port(port: u16) -> Option<u32> {
    let base = std::env::temp_dir().join("victauri");
    for entry in std::fs::read_dir(base).ok()?.flatten() {
        let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
        let advertised = std::fs::read_to_string(entry.path().join("port")).ok()?;
        if advertised.trim() == port.to_string() {
            return Some(pid);
        }
    }
    None
}

/// `AKPROBE=<pid>` 里的 pid（回显里是 `$$`，所以带数字的那个只可能来自 shell 求值）。
fn probe_pid(screen: &str) -> Option<u32> {
    let (_, rest) = screen.rsplit_once("AKPROBE=")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

async fn screen_text(client: &mut VictauriClient) -> String {
    text(
        &client
            .eval_js("window.__akashaTerminal.screenText(200)")
            .await
            .unwrap_or(serde_json::Value::Null),
    )
}

/// 在**当前活动标签页**里起一个忽略 SIGHUP 的后台进程，返回它的 pid。
async fn start_probe(client: &mut VictauriClient) -> u32 {
    type_line(client, PROBE).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(pid) = probe_pid(&screen_text(client).await) {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("活动标签页的屏幕上始终没出现 AKPROBE=<pid>");
}

/// 窗口的可见性（`window get_state`）—— "藏起来了没有"由机器判，不靠人看。
async fn visible(client: &mut VictauriClient) -> Option<bool> {
    let state = client
        .call_tool("window", serde_json::json!({ "action": "get_state" }))
        .await
        .ok()?;
    state.as_array()?.first()?.get("visible")?.as_bool()
}

/// 在截止时间内等"窗口可见性变成 `want`"。
async fn wait_visible(client: &mut VictauriClient, want: bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if visible(client).await == Some(want) {
            return;
        }
        if Instant::now() >= deadline {
            panic!(
                "{what}：15s 内可见性没有变成 {want}（实际 {:?}）",
                visible(client).await
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 问 app 要关窗语义（`app_state { probe: "lifecycle" }`）。
///
/// 启动期还没走到登记那一步时，probe 回的是 `{"initialized": false}` —— 那就再等等。
/// 这个字段是本用例"跳过还是真跑"的**唯一**依据（`AGENTS.md` §7）。
async fn lifecycle(client: &mut VictauriClient) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = client
            .call_tool("app_state", serde_json::json!({ "probe": "lifecycle" }))
            .await
            .expect("app_state 调不通 —— probe 注册上了吗？");
        if snapshot
            .get("initialized")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
        {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "30s 内 lifecycle probe 一直是「未就绪」"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn closing_the_window_hides_it_and_keeps_the_session() {
    if !victauri_test::is_e2e() {
        eprintln!("跳过: 未设置 VICTAURI_E2E=1（该变量由 just test-e2e 设置）");
        return;
    }

    // Windows 上跳过：这条用例的探针是一个 **POSIX shell 程序**
    // （`sh -c 'trap "" HUP; echo AKPROBE=$$; exec sleep 600' &`）—— 那边的默认 shell 是
    // cmd.exe，写不出"忽略 SIGHUP 的后台作业"；而"把它一起收走"这件事在 Windows 上依赖
    // 会话级回收（Job Object），尚未实现（见 `docs/STATUS.md` 的 Windows 会话回收缺口）。
    // 平台无法运行的用例显式跳过并写明原因（`AGENTS.md` §7）——留在这里的其余断言也
    // 一并跳过，不为它们另立一份弱判据。
    if cfg!(windows) {
        eprintln!("跳过: 探针是 POSIX shell 程序，且 Windows 的会话级回收（Job Object）尚未实现");
        return;
    }

    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— 配方起了吗？");
    let port = client.port();
    let app_pid = app_pid_for_port(port).expect("找不到这个 app 的 discovery 目录（pid）");

    // ── 0. 前提：这个 app 的关窗语义确实是"隐藏" ─────────────────────────────
    let state = lifecycle(&mut client).await;

    if state
        .get("close_action")
        .and_then(serde_json::Value::as_str)
        != Some("hide")
    {
        eprintln!("跳过: 关窗语义不是隐藏，托盘不可用或配置为 close_behavior=exit");
        return;
    }

    // ── 1. 起点：终端渲染出来、屏幕上有内容、会话里有个忽略 SIGHUP 的进程 ────
    wait_js(
        &mut client,
        "!!window.__akashaTerminal && window.__akashaTerminal.renderer !== 'none'",
        30_000,
        "终端渲染出来",
    )
    .await;
    type_line(&mut client, "printf 'akasha-hidden-%s\\n' marker").await;
    wait_js(
        &mut client,
        &screen_has("akasha-hidden-marker"),
        30_000,
        "命令求值后的输出出现在屏幕上",
    )
    .await;
    let probe = start_probe(&mut client).await;
    assert!(alive(probe), "探针 {probe} 没起来，这条用例就什么都没验");
    wait_visible(&mut client, true, "关窗之前窗口是可见的").await;
    let sessions_before = registered_sessions(&mut client).await;
    assert!(
        sessions_before > 0,
        "关窗之前注册表里一个会话都没有 —— 那这条用例什么都没验"
    );
    eprintln!("进程: app={app_pid} 探针={probe} 会话={sessions_before}");

    // ── 2. 关窗：**真实路径**（`Window::close()` → `CloseRequested`）──────────
    client
        .call_tool(
            "window",
            serde_json::json!({ "action": "manage", "manage_action": "close" }),
        )
        .await
        .expect("关窗失败");

    // ── 3. 进程还在 + 窗口不可见 ───────────────────────────────────────────
    assert!(
        waits_until(Duration::from_secs(10), || alive(app_pid)),
        "关窗之后 app（pid {app_pid}）退出了 —— 这不是「隐藏」。\
         若本机的托盘建不起来，第一步就该跳过；走到这里说明**隐藏那条路没有生效**"
    );
    wait_visible(&mut client, false, "关窗之后窗口不可见").await;

    // ── 4. 会话还在：探针进程活着 + 注册表里的会话数不变（**预期行为**，不是泄漏 —— AGENTS.md §3.3）
    assert!(
        alive(probe),
        "关窗把会话收了：忽略 SIGHUP 的探针 {probe} 不在了 —— \
         窗口关闭**不是**回收时机（收托盘时终端与隧道必须存活）"
    );
    // 与平台无关的那一半（进程级判据只有 Linux 看得到，见 `process_death_visible`）：
    let sessions_after_hide = registered_sessions(&mut client).await;
    assert_eq!(
        sessions_after_hide, sessions_before,
        "关窗把一个会话收掉了：{sessions_before} 到 {sessions_after_hide} —— \
         收托盘（默认）不回收会话，只有真的退出才回收"
    );

    // ── 5. 终端没被重建：隐藏期间屏幕内容照旧读得到 ──────────────────────────
    wait_js(
        &mut client,
        &screen_has("akasha-hidden-marker"),
        10_000,
        "隐藏期间屏幕内容仍在（窗口是藏起来的，不是重建的）",
    )
    .await;
    wait_js(
        &mut client,
        &screen_has(&format!("AKPROBE={probe}")),
        10_000,
        "隐藏期间探针的输出仍在",
    )
    .await;

    // ── 6. 显示回来：还是同一个终端，还能继续用 ─────────────────────────────
    client
        .call_tool(
            "window",
            serde_json::json!({ "action": "manage", "manage_action": "show" }),
        )
        .await
        .expect("显示窗口失败");
    wait_visible(&mut client, true, "显示之后窗口可见").await;

    type_line(&mut client, "printf 'akasha-after-hide-%s\\n' ok").await;
    wait_js(
        &mut client,
        &screen_has("akasha-after-hide-ok"),
        30_000,
        "显示回来之后还能继续敲命令（会话原样）",
    )
    .await;

    // 收掉自己起的后台进程：别把 `sleep 600` 留在（可能是别人的）会话里。
    type_line(&mut client, &format!("kill {probe}")).await;
    // ⚠️ 同 `tab_close`：非 Linux 上 `alive()` 恒为 true，这一条整段跳过（`||` 短路）。
    assert!(
        !process_death_visible() || waits_until(Duration::from_secs(15), || !alive(probe)),
        "探针 {probe} 没被收掉 —— 隐藏过的会话不该失去作业控制"
    );
}
