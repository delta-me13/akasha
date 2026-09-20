//! plan 0203 的**端到端**验收：真 app、真 xterm、真按键走一个来回。
//!
//! 运行：`VICTAURI_E2E=1 cargo test --test terminal_render -- --test-threads=1`
//! （app 得先跑着：`just dev`）
//!
//! 三条判据，都是"看屏幕"而不是"看日志"：
//!   1. 终端由**画布**渲染 —— `renderer` 是 webgl/canvas，且 DOM 渲染器的行容器
//!      已经消失（`AGENTS.md` §4.1：不作为默认的那条路必须真的没在用）；
//!   2. 一次按键真的走完 `输入 → IPC → PTY → 输出 → 屏幕`：断言的是**命令求值后的
//!      输出**，不是被回显的命令行本身（回显来自 PTY 的 ECHO，证明力弱得多）；
//!   3. 灌 8 MB 输出后**没有卡死**：队列排空、屏幕出现哨兵、还能继续敲命令 ——
//!      这正是 `STATUS.md` 里"消费慢会不会把 PTY 反压死"要的答案。
//!
//! 为什么读 `window.__akashaTerminal` 而不是 DOM 文本：canvas / WebGL 渲染器
//! **不把文本放进 DOM**，屏幕内容只活在 xterm 的缓冲里。探针由
//! `src/terminal/surface.ts` 在 dev 构建里注册（生产构建里整段被摇掉）。
//!
//! 本文件是测试，unwrap 在这里就是断言手段。

#![allow(clippy::unwrap_used)]

use victauri_test::VictauriClient;

/// 终端挂上了、而且用的是画布渲染器。
const RENDERED: &str = r#"(() => {
  const probe = window.__akashaTerminal;
  if (!probe || probe.renderer === 'none') return false;
  return document.querySelectorAll('.xterm-screen canvas').length >= 1;
})()"#;

/// 灌一大坨输出，末尾挂一个"排空哨兵"。
///
/// 哨兵走的是**同一条流**，所以它出现在屏幕上就意味着它前面的 8 MB 全被消费过了 ——
/// 这比"等一会儿再看字节数"强得多。
const FLOOD: &str = "yes akasha | head -c 8000000; printf 'akasha-drained-%s\\n' ok\n";

/// 强制丢掉 WebGL 上下文 —— 真实世界里驱动重启 / 显存不足就是这么发生的。
///
/// 找不到 WebGL 上下文、或这张驱动不提供 `WEBGL_lose_context` 扩展时返回 `false`
/// （调用方跳过，不把"驱动没给这个扩展"报成失败）。两张 canvas 都要试：WebGL 渲染器
/// 另有一张纹理图集 canvas，谁在文档里靠前不保证。
const LOSE_WEBGL_CONTEXT: &str = r#"(() => {
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

fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("跳过: 未设置 VICTAURI_E2E=1（该变量由 just test-e2e 设置）");
        return true;
    }
    false
}

/// `eval_js` 的返回可能把结果包在 `result` 里，也可能就是裸值 —— 两种都认。
fn payload(value: &serde_json::Value) -> &serde_json::Value {
    value.get("result").unwrap_or(value)
}

fn ok(value: &serde_json::Value) -> bool {
    payload(value).as_bool().unwrap_or(false)
}

fn number(value: &serde_json::Value) -> u64 {
    payload(value).as_u64().unwrap_or(0)
}

fn text(value: &serde_json::Value) -> String {
    payload(value).as_str().unwrap_or("?").to_string()
}

/// 把按键送进 xterm 的隐藏 textarea —— 这是**真实**输入路径的入口。
///
/// 为什么不直接调命令：`onData` 之后的每一段（编码、IPC、PTY）都是被测对象，
/// 绕过 `onData` 等于把要验的那一段从测试里删掉。xterm 用 `input` 事件取文本
/// （可打印字符走这条路，功能键才走 keydown），所以这里造一个 `InputEvent`。
///
/// ⚠️ 取的是**当前活动标签页**里的那个 textarea（plan 0305）：多标签之后"第一个"
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

async fn type_line(client: &mut VictauriClient, line: &str) {
    let sent = client.eval_js(&type_js(line)).await.unwrap();
    assert!(ok(&sent), "按键没能送进 xterm：{sent}");
}

/// 等屏幕上出现 `needle`（`wait_for` 轮询，**不是** sleep 猜）。
async fn wait_for_screen(client: &mut VictauriClient, needle: &str, timeout_ms: u64, what: &str) {
    let needle = serde_json::to_string(needle).unwrap();
    let waited = client
        .wait_for_expression(
            &format!("window.__akashaTerminal.screenText().includes({needle})"),
            None,
            Some(timeout_ms),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        waited.get("ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "{what} 超时（{timeout_ms} ms）：{waited}"
    );
}

#[tokio::test]
async fn terminal_renders_on_canvas_and_survives_a_flood() {
    if skip_unless_e2e() {
        return;
    }
    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— `just dev` 起了吗？");

    // ── 1. 渲染：画布渲染器在岗，DOM 渲染器不在岗 ─────────────────────────────
    let rendered = client
        .wait_for_expression(RENDERED, None, Some(30_000), None)
        .await
        .unwrap();
    assert_eq!(
        rendered.get("ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "终端没渲染出来：{rendered}"
    );

    let renderer = text(
        &client
            .eval_js("window.__akashaTerminal.renderer")
            .await
            .unwrap(),
    );
    let canvases = number(
        &client
            .eval_js("document.querySelectorAll('.xterm-screen canvas').length")
            .await
            .unwrap(),
    );
    let dom_rows = number(
        &client
            .eval_js("document.querySelectorAll('.xterm-rows').length")
            .await
            .unwrap(),
    );
    eprintln!("界面: 渲染器={renderer} 画布={canvases}");
    assert!(
        renderer == "webgl" || renderer == "canvas",
        "渲染器必须是 webgl 或 canvas，实际是 {renderer} —— 静默落到 DOM 是禁止的"
    );
    assert!(canvases >= 1, "没有 xterm 的 canvas");
    assert_eq!(dom_rows, 0, "DOM 渲染器的行容器还在 —— 它不该在渲染路径上");

    // ── 2. 一个完整的来回：按键 → PTY → 回显到屏幕 ───────────────────────────
    // 断言的是 `akasha-probe-42` 这个**求值结果**：命令行里只有 `%s`，
    // 所以屏幕上出现它只可能来自 shell 真的执行了这条命令。
    type_line(&mut client, "printf 'akasha-probe-%s\\n' 42\n").await;
    wait_for_screen(&mut client, "akasha-probe-42", 30_000, "命令求值后的输出").await;

    // ── 3. 大流量：不卡死 = 排空 + 哨兵 + 之后还能用 ─────────────────────────
    let before = number(
        &client
            .eval_js("window.__akashaTerminal.batches()")
            .await
            .unwrap(),
    );
    type_line(&mut client, FLOOD).await;

    wait_for_screen(
        &mut client,
        "akasha-drained-ok",
        180_000,
        "8 MB 之后的排空哨兵",
    )
    .await;

    let after = number(
        &client
            .eval_js("window.__akashaTerminal.batches()")
            .await
            .unwrap(),
    );
    let pending = client
        .wait_for_expression(
            "window.__akashaTerminal.pendingBytes() === 0",
            None,
            Some(60_000),
            None,
        )
        .await
        .unwrap();
    eprintln!("界面: 批次增量={}", after - before);
    assert_eq!(
        pending.get("ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "输出排空了但写入队列没归零（xterm 还在消化）：{pending}"
    );

    // 灌完之后还能再敲一条命令 —— 这才是"输出暂停"而不是"卡死"的判据。
    type_line(&mut client, "printf 'akasha-alive-%s\\n' yes\n").await;
    wait_for_screen(
        &mut client,
        "akasha-alive-yes",
        30_000,
        "大流量之后的新命令",
    )
    .await;
}

/// WebGL 上下文丢失 → **必须**退到 canvas，而且**不重建终端**（屏幕内容还在）。
///
/// 这条用例存在的理由很实在：`AGENTS.md` §4.1 把 canvas 定为唯一回退路径，
/// 而回退代码如果从没跑过，它就是一段"看起来对"的死代码。
#[tokio::test]
async fn webgl_context_loss_falls_back_to_canvas() {
    if skip_unless_e2e() {
        return;
    }
    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— `just dev` 起了吗？");

    let rendered = client
        .wait_for_expression(RENDERED, None, Some(30_000), None)
        .await
        .unwrap();
    assert_eq!(
        rendered.get("ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "终端没渲染出来：{rendered}"
    );

    // 先在屏幕上留个"降级前后应当还在"的痕迹。
    type_line(&mut client, "printf 'akasha-pre-loss-%s\\n' ok\n").await;
    wait_for_screen(
        &mut client,
        "akasha-pre-loss-ok",
        30_000,
        "降级前的屏幕内容",
    )
    .await;

    let before = text(
        &client
            .eval_js("window.__akashaTerminal.renderer")
            .await
            .unwrap(),
    );
    let lost = client.eval_js(LOSE_WEBGL_CONTEXT).await.unwrap();
    if !ok(&lost) {
        eprintln!("跳过: 当前渲染器是 {before}，或该驱动不提供 WEBGL_lose_context");
        return;
    }

    let fell = client
        .wait_for_expression(
            "window.__akashaTerminal.renderer === 'canvas'",
            None,
            Some(30_000),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        fell.get("ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "{before} 丢了上下文却没退到 canvas：{fell}"
    );

    // 缓冲区没被清掉 —— 说明只是换了渲染器，不是重开了一个终端。
    let screen = text(
        &client
            .eval_js("window.__akashaTerminal.screenText()")
            .await
            .unwrap(),
    );
    assert!(
        screen.contains("akasha-pre-loss-ok"),
        "降级把屏幕内容弄丢了 —— 那说明终端被重建过，不是换渲染器"
    );

    // 降级之后还能用。
    type_line(&mut client, "printf 'akasha-fallback-%s\\n' ok\n").await;
    wait_for_screen(
        &mut client,
        "akasha-fallback-ok",
        30_000,
        "降级之后的新命令",
    )
    .await;

    let canvases = number(
        &client
            .eval_js("document.querySelectorAll('.xterm-screen canvas').length")
            .await
            .unwrap(),
    );
    eprintln!("界面: 画布={canvases}");
}
