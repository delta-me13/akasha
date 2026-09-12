//! plan 0305 的**端到端**验收：关闭终端标签页 = **立刻丢弃**该 `Session`，且**只丢它自己**。
//!
//! 走的是**真实 UI 路径**：点 `+` 开标签页、点标签页标题切换、点 `×` 关闭 ——
//! 不直接调 `close_session`（那样等于把被测的那一段从测试里删掉）。
//!
//! 判据两层，缺一不可：
//!   1. **立刻丢弃**：点完 `×` **不需要第二次点击**（没有确认弹窗），该会话里的进程在
//!      截止时间内消失。探针取最坏情况：一个明确忽略 SIGHUP 的 `sleep` ——
//!      普通 `sleep` 靠内核 session 级 SIGHUP 本来就不残留，拿它当判据等于没测。
//!   2. **只丢它自己**：另一个标签页的探针**仍然活着**，而且它的屏幕内容还在
//!      （内容还在 = 那个面没有被卸载重建，会话还是原来那个）。
//!   3. 关掉**最后一个**标签页只是空状态：会话照样被丢弃，但**进程留着** ——
//!      「关标签页 ≠ 关窗口 ≠ 退出应用」。
//!   4. **反方向**（plan 0306）：在终端里敲 `exit` → 会话自己结束 → **标签页跟着关掉**。
//!      标签页与会话**同生命期**，两个方向都要成立。
//!
//! 三条前提也顺手被这条用例守住：开新标签页、切换标签页**都不得**动到已有会话。
//!
//! ⚠️ 被关的那个标签页**故意**处在最恶劣的状态下：WebGL 上下文已经丢了、退到了 canvas
//! （见 `LOSE_WEBGL`）。这个状态实测会让 xterm 的 `term.dispose()` 抛异常，而 `dispose()`
//! 跑在 React 的 effect 清理函数里 —— 天真的写法会**同时**坏掉两件事：关一个标签页把整个
//! 界面卸载成空白，以及会话回收那一句再也执行不到（进程留在用户机器上）。
//!
//! ⚠️ 本用例不关 app，但会把界面留在"只剩一个标签页"的状态 —— 排在它后面的
//! `exit_residue` 依赖"活动终端能敲命令"，所以不能留下一个空界面（顺序见 `E2E_TARGETS`）。
//!
//! 探针命令必须 fish / bash / sh 都成立（坑 #41）：`(cmd) &` 在 fish 里是**命令替换**，
//! 会把 shell 挂住；`sh -c '…' &` 三边语义相同。
//!
//! 本文件是测试，unwrap 在这里就是断言手段。

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use victauri_test::VictauriClient;

/// 忽略 SIGHUP 的后台探针（与 `exit_residue` 取同一个最坏情况）。
const PROBE: &str = "sh -c 'trap \"\" HUP; echo AKPROBE=$$; exec sleep 600' &\n";

/// 当前标签页的标题。`aria-selected` 落在标签按钮上，所以"活动"这个类名是唯一的判据。
const ACTIVE_TITLE: &str = "document.querySelector('.tab.is-active .tab-label')?.textContent ?? ''";

const COUNT_TABS: &str = "document.querySelectorAll('.tab').length";

/// "现在正好有 `n` 个标签页"的 JS 条件。
fn tabs_eq(n: usize) -> String {
    format!("{COUNT_TABS} === {n}")
}

/// "当前标签页的标题是 `title`"的 JS 条件。
fn active_title_eq(title: &str) -> String {
    let literal = serde_json::to_string(title).expect("标题转不成 JS 字面量");
    format!("{ACTIVE_TITLE} === {literal}")
}

const NEW_TAB: &str = "(() => { const b = document.querySelector('.tab-new'); if (!b) return false; b.click(); return true; })()";

/// 强制丢掉 WebGL 上下文 —— 真实世界里驱动重启 / 显存不足就是这么发生的。
///
/// 这条用例**特意**先制造这个状态：实测（plan 0305）一个丢过上下文、已经退到 canvas 的
/// 终端，在 `term.dispose()` 时会在 xterm 内部抛 `TypeError`。而 `dispose()` 跑在 React 的
/// effect 清理函数里、又排在"关会话"前面 —— 于是这个状态同时暴露两件事：
/// **关一个标签页把整个界面带崩**，以及**会话回收那一句永远执行不到**。
/// 不给这个扩展的驱动就跳过这一步（核心判据不依赖它）。
const LOSE_WEBGL: &str = r#"(() => {
  for (const canvas of document.querySelectorAll('.xterm-screen canvas')) {
    const gl = canvas.getContext('webgl2');
    if (!gl) continue;
    const ext = gl.getExtension('WEBGL_lose_context');
    if (!ext) continue;
    ext.loseContext();
    return true;
  }
  return false;
})()"#;

/// 点第 `index` 个标签页的标题（切换）。
fn select_tab_js(index: usize) -> String {
    format!(
        "(() => {{ const t = document.querySelectorAll('.tab-label')[{index}]; \
         if (!t) return false; t.click(); return true; }})()"
    )
}

/// 点第 `index` 个标签页的关闭按钮 —— 这就是被测的那个动作。
fn close_tab_js(index: usize) -> String {
    format!(
        "(() => {{ const b = document.querySelectorAll('.tab-close')[{index}]; \
         if (!b) return false; b.click(); return true; }})()"
    )
}

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

/// 把一行敲进**当前活动标签页**的终端。
///
/// 多标签之后"第一个 / 最后一个 textarea"都不再唯一 —— 只有活动面里的那个是确定的
/// （`window.__akashaTerminal` 也跟着活动面走，两者必须指同一个终端）。
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

async fn type_line(client: &mut VictauriClient, line: &str) {
    let sent = client.eval_js(&type_js(line)).await.unwrap();
    assert!(ok(&sent), "按键没能送进 xterm：{sent}");
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

/// 进程是否**真的**活着。
///
/// ⚠️ `/proc/<pid>` 存在 ≠ 活着：僵尸（`Z`）也有目录项（坑 #48）。而 SIGKILL 的投递
/// 又是异步的（坑 #45）—— 所以判据一律是"**在截止时间内消失**"，不是"信号发过了"。
fn alive(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return false;
    };
    !matches!(rest.split_whitespace().next(), None | Some("Z"))
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

/// `AKPROBE=<pid>` 里的 pid。命令行回显里是 `$$`（不带数字），
/// 所以"屏幕上有带数字的 `AKPROBE=`"只可能来自 shell 求值后的输出。
fn probe_pid(screen: &str) -> Option<u32> {
    let (_, rest) = screen.rsplit_once("AKPROBE=")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// 在**当前活动标签页**里起一个忽略 SIGHUP 的后台进程，返回它的 pid。
async fn start_probe(client: &mut VictauriClient) -> u32 {
    type_line(client, PROBE).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let screen = text(
            &client
                .eval_js("window.__akashaTerminal.screenText(200)")
                .await
                .unwrap_or(serde_json::Value::Null),
        );
        if let Some(pid) = probe_pid(&screen) {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("活动标签页的屏幕上始终没出现 AKPROBE=<pid>");
}

/// 屏幕上出现 `needle` 的 JS 条件（用于 `wait_js`）。
fn screen_has(needle: &str) -> String {
    format!(
        "window.__akashaTerminal.screenText(200).includes({})",
        serde_json::to_string(needle).expect("文本无法转成 JS 字面量")
    )
}

#[tokio::test]
async fn closing_a_terminal_tab_discards_only_its_own_session() {
    if !victauri_test::is_e2e() {
        eprintln!("Skipping: set VICTAURI_E2E=1 with your Tauri dev server running");
        return;
    }
    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— 配方起了吗？");

    // ── 0. 起点：一个标签页，终端渲染出来 ───────────────────────────────────
    wait_js(&mut client, &tabs_eq(1), 30_000, "初始只有一个标签页").await;
    wait_js(
        &mut client,
        "!!window.__akashaTerminal && window.__akashaTerminal.renderer !== 'none'",
        30_000,
        "第一个终端渲染出来",
    )
    .await;

    // 先把第一个标签页的渲染器**弄坏**（丢 WebGL 上下文 → 退 canvas）：见 `LOSE_WEBGL`
    // 的注释 —— 这正是"关闭时把会话一起丢掉"最容易被漏掉的那个状态。
    if ok(&client.eval_js(LOSE_WEBGL).await.unwrap()) {
        wait_js(
            &mut client,
            "window.__akashaTerminal.renderer === 'canvas'",
            30_000,
            "丢上下文后退到 canvas",
        )
        .await;
        eprintln!("已把标签页 1 的 WebGL 上下文丢掉（退到 canvas）");
    } else {
        eprintln!(
            "跳过丢上下文：当前不是 WebGL 渲染器（可能已经被前面的目标丢过了 —— 那也满足本用例的前提），\
             或驱动不提供 WEBGL_lose_context"
        );
    }

    // ── 1. 标签页 1 里的探针 A ─────────────────────────────────────────────
    let probe_a = start_probe(&mut client).await;
    assert!(
        alive(probe_a),
        "探针 A({probe_a}) 没起来，这条用例就什么都没验"
    );

    // ── 2. 点 + 开第二个标签页：**开新的不得动到旧的** ─────────────────────
    assert!(
        ok(&client.eval_js(NEW_TAB).await.unwrap()),
        "找不到新建标签页的按钮"
    );
    wait_js(&mut client, &tabs_eq(2), 30_000, "第二个标签页出现").await;
    wait_js(
        &mut client,
        "window.__akashaTerminal.renderer !== 'none'",
        30_000,
        "第二个终端渲染出来",
    )
    .await;
    assert!(
        alive(probe_a),
        "开第二个标签页就把第一个的进程弄没了：{probe_a}"
    );

    // ── 3. 标签页 2 里的探针 B ─────────────────────────────────────────────
    let probe_b = start_probe(&mut client).await;
    assert!(
        alive(probe_b) && probe_a != probe_b,
        "两个标签页必须是两个会话（探针 {probe_a} / {probe_b}）"
    );
    eprintln!("标签页 1：探针 A = {probe_a}；标签页 2：探针 B = {probe_b}");

    // ── 4. 切回标签页 1：**切换不是关闭** ──────────────────────────────────
    assert!(
        ok(&client.eval_js(&select_tab_js(0)).await.unwrap()),
        "找不到标签页 1 的标题"
    );
    wait_js(
        &mut client,
        &active_title_eq("终端 1"),
        10_000,
        "切回标签页 1",
    )
    .await;
    assert!(
        alive(probe_a) && alive(probe_b),
        "切换标签页不该收掉任何会话（A {probe_a} / B {probe_b}）"
    );
    eprintln!(
        "标签页数：1 → + → 2 → 切换（当前「{}」）→ 两个探针都还在",
        text(&client.eval_js(ACTIVE_TITLE).await.unwrap())
    );

    // ── 5. 点 × 关闭标签页 1：**立刻丢弃** ─────────────────────────────────
    let clicked = Instant::now();
    assert!(
        ok(&client.eval_js(&close_tab_js(0)).await.unwrap()),
        "找不到标签页 1 的关闭按钮"
    );
    // 不需要第二次点击（没有确认弹窗）：界面直接少一个标签页，剩下那个接管。
    //
    // ⚠️ 这一步同时守着**界面不能被带崩**：标签页 1 的渲染器是坏过的（见 `LOSE_WEBGL`），
    // 而 `term.dispose()` 正是在这种状态下会抛。抛在 React 的 effect 清理函数里 = React
    // 卸载整棵树 —— 那时这里读到的是 **0** 而不是 1（下面的超时消息会这么说）。
    wait_js(
        &mut client,
        &tabs_eq(1),
        10_000,
        "标签页只剩一个（0 = 整个界面被卸载了：关闭把 UI 带崩）",
    )
    .await;
    wait_js(
        &mut client,
        &active_title_eq("终端 2"),
        10_000,
        "剩下的标签页接管",
    )
    .await;
    assert!(
        waits_until(Duration::from_secs(15), || !alive(probe_a)),
        "关闭标签页之后探针 A({probe_a}) 还活着 —— 这个会话没被丢弃"
    );
    eprintln!(
        "探针 A({probe_a}) 已随标签页消失（{} ms）",
        clicked.elapsed().as_millis()
    );

    // ── 6. **只丢它自己**：另一个标签页的进程与屏幕内容都要在 ───────────────
    assert!(
        alive(probe_b),
        "关掉标签页 1 把标签页 2 的进程一起收了：{probe_b}"
    );
    // 屏幕内容还在 = 那个面没有被卸载重建（重建会换掉会话，探针 B 的命运也就不可信了）。
    wait_js(
        &mut client,
        &screen_has(&format!("AKPROBE={probe_b}")),
        10_000,
        "剩下的标签页接管探针且屏幕内容仍在",
    )
    .await;
    eprintln!("探针 B({probe_b}) 仍在，且它的屏幕内容还在");

    // 剩下的那个标签页**还能用** —— 关掉一个标签页不该把界面带坏。
    type_line(&mut client, "printf 'akasha-tab-alive-%s\\n' ok\n").await;
    wait_js(
        &mut client,
        &screen_has("akasha-tab-alive-ok"),
        30_000,
        "关闭之后剩下的标签页仍可交互",
    )
    .await;

    // ── 7. 关掉**最后一个**标签页：空状态，**不是退出应用** ─────────────────
    // 「关标签页 ≠ 关窗口 ≠ 退出应用」（`docs/scope.md` §5.6）：最后一个标签页关掉时
    // 它的会话照样被丢弃，但**进程要留着**（收托盘是窗口的事，不是标签页的事）。
    assert!(
        ok(&client.eval_js(&close_tab_js(0)).await.unwrap()),
        "找不到最后一个标签页的关闭按钮"
    );
    wait_js(&mut client, &tabs_eq(0), 10_000, "标签页全部关闭（空状态）").await;
    assert!(
        waits_until(Duration::from_secs(15), || !alive(probe_b)),
        "关掉最后一个标签页之后探针 B({probe_b}) 还活着 —— 这个会话没被丢弃"
    );
    let empty = text(
        &client
            .eval_js("document.querySelector('.tab-empty')?.textContent ?? ''")
            .await
            .unwrap(),
    );
    assert!(
        empty.contains("没有打开的标签页"),
        "关掉最后一个标签页之后没有空状态（app 是不是被带崩了？）：{empty:?}"
    );
    eprintln!("关掉最后一个标签页：空状态（app 仍在），探针 B({probe_b}) 也随会话被丢弃");

    // 空状态下还能再开一个 —— 界面是空的，进程不是。
    assert!(
        ok(&client.eval_js(NEW_TAB).await.unwrap()),
        "空状态下找不到新建标签页的按钮"
    );
    wait_js(&mut client, &tabs_eq(1), 30_000, "空状态下再开一个标签页").await;
    wait_js(
        &mut client,
        "window.__akashaTerminal.renderer !== 'none'",
        30_000,
        "重开的终端渲染出来",
    )
    .await;
    type_line(&mut client, "printf 'akasha-reopened-%s\\n' ok\n").await;
    wait_js(
        &mut client,
        &screen_has("akasha-reopened-ok"),
        30_000,
        "重开的标签页可交互",
    )
    .await;

    // ── 8. 在终端里敲 `exit`：会话**自己**结束 → 标签页跟着关（plan 0306）──────
    // 反方向：0305 验的是"关标签页 → 丢弃会话"，这一段验"会话自己走 → 标签页跟着走"。
    // 标签页与会话**同生命期**，两个方向都要成立。
    //
    // ⚠️ 必须在**干净**的标签页里敲：会话里若还有别的进程握着 PTY（例如忽略 SIGHUP 的
    // 后台作业），主端就读不到 EOF —— 那时真实终端的行为也是"标签页留着"，不是 bug。
    type_line(&mut client, "exit\n").await;
    wait_js(
        &mut client,
        &tabs_eq(0),
        15_000,
        "敲 exit 之后标签页自己关掉（会话结束 = 标签页关闭）",
    )
    .await;
    eprintln!("在终端里敲 exit：标签页自己关掉（app 仍在）");

    // 会话结束了 ≠ app 退出：还能再开一个（也给后面的 `exit_residue` 留个能用的终端）。
    assert!(
        ok(&client.eval_js(NEW_TAB).await.unwrap()),
        "敲 exit 之后找不到新建标签页的按钮"
    );
    wait_js(&mut client, &tabs_eq(1), 30_000, "exit 之后再开一个标签页").await;
    wait_js(
        &mut client,
        "window.__akashaTerminal.renderer !== 'none'",
        30_000,
        "exit 之后新开的终端渲染出来",
    )
    .await;
    type_line(&mut client, "printf 'akasha-after-exit-%s\\n' ok\n").await;
    wait_js(
        &mut client,
        &screen_has("akasha-after-exit-ok"),
        30_000,
        "exit 之后新开的标签页可交互",
    )
    .await;

    eprintln!("✅ 关闭终端标签页 = 立刻丢弃该 Session，且只丢它自己（关掉最后一个也不退出应用）");
    eprintln!("✅ 在终端里敲 exit：会话自己结束，标签页跟着关掉（app 仍在）");
}
