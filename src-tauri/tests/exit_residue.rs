//! plan 0204 的**端到端**验收：真正退出之后零残留。
//!
//! ⚠️ 这条用例**会把被测 app 关掉**，所以只在"app 是 `just test-e2e` 自己起的"时候跑
//! （配方会设 `AKASHA_E2E_OWNS_APP=1`）；复用一个 `just dev` 的 app 时它**显式跳过**并
//! 写明原因 —— 跑一次 E2E 就把别人开着的开发 app 关掉，是比"少一个用例"糟得多的事。
//! 配方还保证它跑在**第二段**（`E2E_TARGETS_EXIT`：先写 `close_behavior=exit` 再起 app），
//! 因为关窗的默认语义是**隐藏**（plan 0302）——那是第一段的对象。
//!
//! ⚠️ **这条用例同时是"配置真的被读到"的证据**（plan 0303）：配置文件没生效的话，关窗只会
//! 把窗口藏起来、进程照旧活着，下面的断言会在 30s 后红。也就是说它一箭双雕，而这是**顺带**
//! 得到的：刺激（关窗）没变，变的只是"关窗之后该发生什么"由配置说了算。
//!
//! 探针取的是**最坏情况**：一个明确忽略 SIGHUP 的进程（`trap "" HUP`）。内核在 PTY
//! 挂断时发的那轮 SIGHUP 对它无效，`Child::kill()` 也够不着它 —— 实测（plan 0204 的
//! 实施记录）它在「关窗口 / SIGKILL / SIGTERM」三条路径上**都会活下来并逐次累积**。
//! 也就是说：这条断言在改之前必红，改之后才绿（不是"怎么都能过"的那种）。
//!
//! ⚠️ 探针命令必须**同时**在 fish / bash / sh 下成立：它是被直接敲进用户登录 shell 的。
//! 一开始写成 POSIX 的 `(cmd) &`，在 **fish** 里 `(...)` 是**命令替换** —— 于是
//! 那一行会把 shell 挂住直到 `sleep 600` 结束，后面所有"敲命令"的用例全部跟着超时。
//! `sh -c '…' &` 是三种 shell 下语义相同的写法。
//!
//! 平台差异照实说：会话级回收目前只有 Linux 实现（Windows 要 Job Object，macOS 要
//! `proc_listpids` + `getsid`，见 `pty` 的 `teardown` 模块）。非 Linux 上不起探针、
//! 只断言"关窗口 = app 真的退出"，并打印原因 —— 不把弱判据说成强判据。
//!
//! 本文件是测试，unwrap 在这里就是断言手段。

#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::time::{Duration, Instant};

use victauri_test::VictauriClient;

/// 启动这条用例的配方有没有把 app 一起起起来（见文件头）。
const OWNS_APP_ENV: &str = "AKASHA_E2E_OWNS_APP";

fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("跳过: 未设置 VICTAURI_E2E=1（该变量由 just test-e2e 设置）");
        return true;
    }
    if std::env::var_os(OWNS_APP_ENV).is_none() {
        eprintln!("跳过: 复用了别的 app，这条用例会关闭它，需先停止 just dev");
        return true;
    }
    false
}

/// `eval_js` 的返回可能把结果包在 `result` 里，也可能就是裸值 —— 两种都认。
fn payload(value: &serde_json::Value) -> &serde_json::Value {
    value.get("result").unwrap_or(value)
}

fn text(value: &serde_json::Value) -> String {
    payload(value).as_str().unwrap_or("?").to_string()
}

/// 把一行送进 xterm 的隐藏 textarea（同 `terminal_render.rs`：这是**真实**输入路径）。
///
/// ⚠️ 取的是**当前活动标签页**里的那个 textarea（plan 0305）：多标签之后"最后一个"
/// 不再唯一，而活动面里那个才与 `window.__akashaTerminal` 指同一个终端。
fn type_js(line: &str) -> String {
    let literal = serde_json::to_string(line).expect("文本无法转成 JS 字符串字面量");
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

/// 在**截止时间**内轮询 `condition`（不用固定 sleep 猜）。
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

fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
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

/// `AKPROBE=<pid>` 里的 pid。屏幕上**只可能**有一处带数字的：命令行回显里是 `$$`。
fn probe_pid(screen: &str) -> Option<u32> {
    let (_, rest) = screen.rsplit_once("AKPROBE=")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// 在**截止时间**内轮询屏幕，直到读出探针 pid（`AGENTS.md` §7：等的是"真的到了"，
/// 不是 sleep 猜一个时长）。
async fn wait_for_probe_pid(client: &mut VictauriClient, timeout: Duration) -> Option<u32> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let screen = text(
            &client
                .eval_js("window.__akashaTerminal.screenText(200)")
                .await
                .unwrap_or(serde_json::Value::Null),
        );
        if let Some(pid) = probe_pid(&screen) {
            return Some(pid);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

/// 在 UI 的终端里起一个**忽略 SIGHUP** 的后台进程，返回它的 pid。
///
/// `printf` 的那一层没有必要了：`sh -c '… &=$$'` 的回显里是 `$$`（不带数字），
/// 所以"屏幕上有带数字的 AKPROBE="只可能来自 shell 求值后的输出。
async fn start_ignorant_probe(client: &mut VictauriClient) -> u32 {
    let rendered = client
        .wait_for_expression(
            "!!window.__akashaTerminal && window.__akashaTerminal.renderer !== 'none'",
            None,
            Some(30_000),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        rendered.get("ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "终端没渲染出来：{rendered}"
    );

    let typed = client
        .eval_js(&type_js(
            "sh -c 'trap \"\" HUP; echo AKPROBE=$$; exec sleep 600' &\n",
        ))
        .await
        .unwrap();
    assert!(
        payload(&typed).as_bool().unwrap_or(false),
        "按键没能送进 xterm：{typed}"
    );

    let probe = wait_for_probe_pid(client, Duration::from_secs(30))
        .await
        .expect("屏幕上始终没出现 AKPROBE=<pid>");
    assert!(alive(probe), "探针 {probe} 没起来，这条用例就什么都没验");
    probe
}

#[tokio::test]
async fn closing_the_window_leaves_no_child_behind() {
    if skip_unless_e2e() {
        return;
    }

    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— 配方起了吗？");
    let port = client.port();
    let app_pid = app_pid_for_port(port).expect("找不到这个 app 的 discovery 目录（pid）");

    // 探针只在 Linux 上起：会话级回收只有 Linux 有实现，而且 Windows 上没有 `sh`。
    let probe = if cfg!(target_os = "linux") {
        Some(start_ignorant_probe(&mut client).await)
    } else {
        eprintln!("跳过: 会话级回收只有 Linux 实现，非 Linux 平台不启动探针");
        None
    };
    eprintln!("进程: pid={app_pid} 端口={port} 探针={probe:?}");

    // ── 走**真实退出路径**：关窗口 ──────────────────────────────────────────
    // 本用例的退出刺激**没变**（还是关窗），变的是"关窗之后该发生什么"现在由配置说了算
    // （plan 0303 的 `close_behavior=exit`，配方在第二段写进数据目录）。
    let closed = client
        .call_tool(
            "window",
            serde_json::json!({ "action": "manage", "manage_action": "close" }),
        )
        .await;
    assert!(closed.is_ok(), "关窗口失败：{closed:?}");

    assert!(
        waits_until(Duration::from_secs(30), || !alive(app_pid)),
        "app {app_pid} 在关窗口之后 30s 还活着 —— 要么这次运行的配置不是 close_behavior=exit\
         （关窗的默认语义是隐藏，plan 0302，那种情况配方会跳过本用例），\
         要么数据目录里的配置文件没被读到"
    );

    // ── 子进程一个都不剩 ───────────────────────────────────────────────────
    if let Some(probe) = probe {
        // SIGKILL 的投递是**异步**的（`pty` 的 teardown 用例里写过这件事），
        // 所以这里等的是"消失"，而不是"信号发过了"。
        assert!(
            waits_until(Duration::from_secs(15), || !alive(probe)),
            "退出后仍有残留：忽略 SIGHUP 的 {probe} 还活着 —— \
             会话级回收没生效（只 kill 那个 shell 是收不走它的）"
        );
    }
}
