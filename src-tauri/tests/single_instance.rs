//! plan 0304 的**端到端**验收：第二个实例**唤起已有窗口**，而不是各跑一套
//! （`scope.md` §5.5：两条隧道指向同一目标、两套托盘图标、"退出一个还剩一个"）。
//!
//! 判据五层，缺一不可：
//!
//! 1. **第二个实例自己退了**（退出码 0）—— 退出的进程不可能再跑一套；
//! 2. **话真的带到了这个进程**：app 自己上报的 `activations` 加一（probe 读，不 grep 日志）；
//! 3. **窗口回到屏幕上** —— 而且是在它**先被藏起来**的前提下：plan 0302 之后
//!    "窗口不在屏幕上"是常态，只 `set_focus()` 是叫不回一个隐藏窗口的（plan 0304 步骤 3）；
//! 4. **还是同一个窗口 / 同一个会话**：藏起来之前屏幕上的内容，唤起之后照样读得到
//!    （重建窗口会换掉终端与回滚缓冲，"窗口可见"就不再代表"原来那个终端回来了"）；
//! 5. **只有一个 app 进程**（Linux：数 `/proc` 里同一个可执行文件、且不是看门狗的进程）。
//!
//! ⚠️ **前提是"这台机器注册上了单实例机制"**：Linux 上它是**会话总线上的一个名字**，
//! 容器 / CI 的 xvfb 里没有会话总线 —— 那时 app 降级为"可以多开"（`AGENTS.md` §3.3 的
//! 降级口径），本用例**显式跳过**并打印 app 上报的 probe。判据来自 app 自己的状态，
//! 不是"猜它可能不满足"，也不是"试着起一个第二实例看看会不会退"。
//!
//! 平台差异照实说：第 5 层靠 `/proc`，别处**显式跳过**并打印原因；1–4 层三平台都跑。
//!
//! 本文件是测试，unwrap 在这里就是断言手段。

#![allow(clippy::unwrap_used)]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use victauri_test::VictauriClient;

/// 看门狗模式的 argv 标志（`akasha_lib::pty::watchdog::FLAG` 的线上表示）。
///
/// 数"有几个 app 实例"时必须把它排除掉：看门狗**就是同一个可执行文件**再跑一次
/// （plan 0205），按可执行文件数会把它一起数进去。
///
/// 唯一的用处是 `app_instances()`，而那个函数读 `/proc`、只在 Linux 上存在 ——
/// 常量跟着它门控，否则 macOS 上会多一条"未使用的常量"，而 `just clippy` 带 `-D warnings`。
#[cfg(target_os = "linux")]
const WATCHDOG_FLAG: &str = "--akasha-session-watchdog";

/// `eval_js` 的返回可能把结果包在 `result` 里，也可能就是裸值 —— 两种都认。
fn payload(value: &serde_json::Value) -> &serde_json::Value {
    value.get("result").unwrap_or(value)
}

fn ok(value: &serde_json::Value) -> bool {
    payload(value).as_bool().unwrap_or(false)
}

/// `eval_js` 的字符串结果（拿不到就给 `?`）。
fn text(value: &serde_json::Value) -> String {
    payload(value).as_str().unwrap_or("?").to_string()
}

/// 让 `{head}-{arg}` 出现在屏幕上，而**命令行里看不出它**。
///
/// 判据是"shell 真的执行了这条命令"，不是"按键被回显了"：命令行里若已经写着结果，
/// 光靠 PTY 的回显就能命中。POSIX 用 `printf` 的格式串把命令行与结果拆开（命令行里是
/// `%s`）；Windows 的默认 shell（cmd.exe）没有 `printf`，改用内建的 `type` 倒一份文件
/// —— 命令行里只有路径，屏幕上的字只可能来自文件内容。
#[cfg(unix)]
fn echo_marker(head: &str, arg: &str) -> String {
    format!("printf '{head}-%s\\n' {arg}")
}

#[cfg(windows)]
fn echo_marker(head: &str, arg: &str) -> String {
    // ⚠️ 文件名里**不得**出现 `{head}-{arg}`：命令行会被 PTY 回显，回显里若已经有求值结果，
    // 光靠回显就能让断言命中 —— 这条用例就什么都没验（正是上面说的那条判据）。
    let marker = format!("{head}-{arg}");
    let path = std::env::temp_dir().join(format!("akasha-e2e-{head}.txt"));
    std::fs::write(&path, format!("{marker}\n")).expect("造探针文件失败");
    format!("type \"{}\"", path.display())
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
///
/// 超时的时候把**终端屏幕上的原文**一起报出来（理由与 `terminal_render::wait_for_screen` 相同）。
async fn wait_js(client: &mut VictauriClient, expression: &str, timeout_ms: u64, what: &str) {
    let waited = client
        .wait_for_expression(expression, None, Some(timeout_ms), None)
        .await
        .unwrap();
    if waited.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        return;
    }
    let screen = text(
        &client
            .eval_js("window.__akashaTerminal.screenText()")
            .await
            .unwrap_or(serde_json::Value::Null),
    );
    panic!("{what} 超时（{timeout_ms} ms）：{waited} —— 屏幕上是 {screen:?}");
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

/// 窗口的可见性（`window get_state`）—— "藏起来了没有"由机器判，不靠人看（问题 #64）。
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

/// 问 app 单实例机制的状态（`app_state { probe: "single_instance" }`）。
///
/// `registered` 是本用例"真跑还是跳过"的**唯一**依据；`activations` 是"第二个实例的
/// 话真的带到了这个进程"的证据。
async fn single_instance(client: &mut VictauriClient) -> serde_json::Value {
    client
        .call_tool(
            "app_state",
            serde_json::json!({ "probe": "single_instance" }),
        )
        .await
        .expect("app_state 调不通 —— probe 注册上了吗？")
}

/// 在截止时间内等 `activations` 到达 `want`。
async fn wait_activations(client: &mut VictauriClient, want: u64) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let seen = single_instance(client)
            .await
            .get("activations")
            .and_then(serde_json::Value::as_u64);
        if seen == Some(want) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "20s 内 activations 没有变成 {want}（实际 {seen:?}）—— \
             第二个实例没有把话带到这个进程（它是不是自己起了一套？）"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 等第二个实例**自己退出**；退不出来就杀掉，别把它留在这台机器上。
fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// 这个进程的可执行文件**是不是那个 app 二进制**。
///
/// ⚠️ **不能比 `canonicalize` 之后的完整路径**：cargo 重建时会拿一个**新的 hardlink**
/// 换掉 `target/debug/akasha`，于是**正在跑的那个进程**的 `/proc/<pid>/exe` 变成
/// `…/target/debug/akasha (deleted)` —— 那个路径已经不存在，`canonicalize` 直接报
/// NotFound。拿它做相等比较的结果是"一个 app 实例都找不到"（本轮就是这么红的）。
/// 于是比"父目录 + 文件名"，文件名先去掉内核加的 ` (deleted)`。
fn is_app_binary(target: &std::path::Path, want: &std::path::Path) -> bool {
    fn name_of(path: &std::path::Path) -> Option<String> {
        let name = path.file_name()?.to_string_lossy();
        Some(name.trim_end_matches(" (deleted)").to_string())
    }
    name_of(target) == name_of(want) && target.parent() == want.parent()
}

/// 同一个可执行文件、且**不是看门狗**的进程（= 有几个 app 实例）。
///
/// 比对 argv 而不是"命令行里含某段文本"：沙箱包装进程自己的命令行里带着整段脚本文本，
/// 子串匹配会误伤（问题 #49）。看门狗按 [`WATCHDOG_FLAG`] **整参数**排除。
#[cfg(target_os = "linux")]
fn app_instances() -> Vec<u32> {
    let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_akasha")).expect("app 二进制不在？");
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(target) = std::fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        if !is_app_binary(&target, &exe) {
            continue;
        }
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let is_watchdog = cmdline
            .split(|byte| *byte == 0)
            .any(|arg| arg == WATCHDOG_FLAG.as_bytes());
        if !is_watchdog {
            pids.push(pid);
        }
    }
    pids
}

/// 上面那个"重建过的二进制仍要认得出来"的判据本身也要有用例守着 ——
/// 它红了的表现是**静默变成"找不到实例"**（本轮踩的就是这个）。
#[test]
fn a_rebuilt_binary_is_still_recognised() {
    let dir = std::path::Path::new("/tmp/build/debug");
    let want = dir.join("akasha");
    assert!(is_app_binary(&want, &want));
    assert!(is_app_binary(&dir.join("akasha (deleted)"), &want));
    assert!(!is_app_binary(&dir.join("akasha-watchdog"), &want));
    assert!(!is_app_binary(&dir.join("other").join("akasha"), &want));
}

#[tokio::test]
async fn a_second_instance_activates_the_hidden_window_of_the_first() {
    if !victauri_test::is_e2e() {
        eprintln!("跳过: 未设置 VICTAURI_E2E=1（该变量由 just test-e2e 设置）");
        return;
    }

    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— 配方起了吗？");
    let port = client.port();
    let app_pid = app_pid_for_port(port).expect("找不到这个 app 的 discovery 目录（pid）");

    // ── 0. 前提：这台机器上真的注册上了单实例机制 ───────────────────────────
    let state = single_instance(&mut client).await;

    if state.get("registered").and_then(serde_json::Value::as_bool) != Some(true) {
        eprintln!("跳过: 本机没有注册单实例机制，Linux 上它需要 D-Bus 会话总线");
        return;
    }
    let before = state
        .get("activations")
        .and_then(serde_json::Value::as_u64)
        .expect("activations 字段不见了");

    // ── 1. 起点：终端有内容，然后**把窗口藏起来** ───────────────────────────
    wait_js(
        &mut client,
        "!!window.__akashaTerminal && window.__akashaTerminal.renderer !== 'none'",
        30_000,
        "终端渲染出来",
    )
    .await;
    type_line(&mut client, &echo_marker("akasha-single", "marker")).await;
    wait_js(
        &mut client,
        &screen_has("akasha-single-marker"),
        30_000,
        "命令求值后的输出出现在屏幕上",
    )
    .await;

    client
        .call_tool(
            "window",
            serde_json::json!({ "action": "manage", "manage_action": "hide" }),
        )
        .await
        .expect("隐藏窗口失败");
    wait_visible(&mut client, false, "起第二个实例之前窗口是藏着的").await;
    eprintln!("进程: app={app_pid} activations={before}");

    // ── 2. 起第二个实例（就是**同一个可执行文件**再跑一次）──────────────────
    let log_path = std::env::temp_dir().join("akasha-e2e-second-instance.log");
    let log = std::fs::File::create(&log_path).expect("建不了第二个实例的日志文件");
    let log_err = log.try_clone().expect("复制不了文件句柄");
    let mut second = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()
        .expect("起不了第二个实例");

    // ── 3. 它必须**自己退掉**：退出的进程不可能再跑一套 ─────────────────────
    let started = Instant::now();
    let status = wait_for_exit(&mut second, Duration::from_secs(30));
    let elapsed = started.elapsed();
    match status {
        Some(status) => assert!(
            status.success(),
            "第二个实例退了，但退出码是 {status}（期望 0）：\n{}",
            std::fs::read_to_string(&log_path).unwrap_or_default()
        ),
        None => panic!(
            "第二个实例 30s 内没有退出 —— 它成了**第二套 app**，而不是「唤起已有窗口」。\n\
             它的输出：\n{}",
            std::fs::read_to_string(&log_path).unwrap_or_default()
        ),
    }
    eprintln!("进程: 第二个实例退出耗时={elapsed:?}");

    // ── 4. 话带到了 + 窗口回到屏幕上 ────────────────────────────────────────
    wait_activations(&mut client, before + 1).await;
    wait_visible(&mut client, true, "第二个实例唤起之后窗口可见").await;

    // ── 5. 还是同一个窗口 / 同一个会话：藏之前的内容照旧读得到 ──────────────
    wait_js(
        &mut client,
        &screen_has("akasha-single-marker"),
        10_000,
        "唤起之后屏幕内容仍在（是原来那个窗口，不是重建的）",
    )
    .await;

    // ── 6. 只有一个 app 进程（Linux：数 /proc）──────────────────────────────
    #[cfg(target_os = "linux")]
    {
        let instances = app_instances();
        assert_eq!(
            instances,
            vec![app_pid],
            "跑着的 app 实例不止一个：{instances:?}（期望只有 {app_pid}）—— \
             看门狗按 argv 排除，所以这里多出来的那个就是**真的第二套**"
        );
    }
    #[cfg(not(target_os = "linux"))]
    eprintln!("跳过: 非 Linux 平台，进程实例计数依赖 /proc");
}
