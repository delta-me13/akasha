//! 二进制通道的**端到端**验收：真 app、真 PTY、raw 字节（plan 0202）。
//!
//! 运行：`VICTAURI_E2E=1 cargo test --test session_channel -- --test-threads=1`
//! （app 得先跑着：`just dev`）
//!
//! 为什么这条用例不走 `src/ipc/` 的 TS 包装：这里要验的是**线上格式**本身 ——
//! 频道句柄就是 `__CHANNEL__:<id>` 这个字符串，后端发出来的是 raw（JS 侧 ArrayBuffer）。
//! 包装层将来会变（0203 接 xterm），线上格式不会 —— 所以这里手工构造它，
//! 顺便把"格式假设"钉成断言。
//!
//! 本文件是测试，unwrap 在这里就是断言手段。

#![allow(clippy::unwrap_used)]

use victauri_test::VictauriClient;

fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("Skipping: set VICTAURI_E2E=1 with your Tauri dev server running");
        return true;
    }
    false
}

/// 打开会话的探针脚本：**手工**按线上格式建频道，统计收到的字节与批次数。
///
/// 统计放在 `window.__akashaProbe` 上，后续用 `wait_for` 轮询 —— 这是"等异步真正完成"
/// 的做法（`AGENTS.md` §7），不是 sleep。
const OPEN_AND_WATCH: &str = r#"
(() => {
  const internals = window.__TAURI_INTERNALS__;
  const probe = {
    bytes: 0, batches: 0, text: "", handle: null, error: null, closed: false,
    // 帧的**类型**也要盯：raw 通道应当收到二进制。若退化成 `number[]`
    // （tauri#13138 那种回归），字节数依然"对"，但整条路已经变成 JSON 序列化 ——
    // 只数字节数是抓不到这件事的。
    jsonFrames: 0, frameType: null, endFrames: 0,
  };
  window.__akashaProbe = probe;

  const id = internals.transformCallback((raw) => {
    // ⚠️ 频道的收尾帧是 `{index, end:true}`，**没有 `message`** —— 官方
    // `Channel` 正是先判 `'end' in raw` 再取 `raw.message`（@tauri-apps/api/core.js）。
    // 少了这一跳，收尾帧会在 `buf.byteLength` 上抛 `TypeError`。
    // 它不会让本用例红（本用例不看 console），但会把异常**留在 webview 的 console 里**，
    // 于是同一个 app 上后跑的 `smoke::ipc_integrity_passes`（no console errors）变红 ——
    // 用例之间的污染。源头在这里：探针不许往 console 里丢异常。
    if (raw === null || typeof raw !== "object" || "end" in raw) {
      probe.endFrames += 1;
      return;
    }
    const buf = raw.message;
    if (buf === null || buf === undefined) return;
    if (buf instanceof ArrayBuffer) probe.frameType = "ArrayBuffer";
    else if (ArrayBuffer.isView(buf)) probe.frameType = buf.constructor.name;
    else { probe.frameType = typeof buf; probe.jsonFrames += 1; }
    probe.bytes += buf.byteLength;
    probe.batches += 1;
    probe.text += new TextDecoder().decode(new Uint8Array(buf));
  });

  return internals
    .invoke("open_session", { channel: "__CHANNEL__:" + id })
    .then((handle) => { probe.handle = handle; return handle; })
    .catch((e) => { probe.error = String(e); return null; });
})()
"#;

/// 把一段文本写进会话（`write_session` 走普通 JSON 参数：一次按键几个字节）。
fn write_js(text: &str) -> String {
    let bytes: Vec<String> = text.bytes().map(|b| b.to_string()).collect();
    format!(
        r#"window.__TAURI_INTERNALS__.invoke("write_session", {{ handle: window.__akashaProbe.handle, data: [{}] }}).then(() => true).catch((e) => {{ window.__akashaProbe.error = String(e); return false; }})"#,
        bytes.join(",")
    )
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

#[tokio::test]
async fn raw_channel_carries_ten_megabytes() {
    if skip_unless_e2e() {
        return;
    }
    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— `just dev` 起了吗？");

    // 探针脚本本身返回的是句柄（不是布尔），所以这里只要求"没报工具错" ——
    // 真正的判据是下面那两条对 `window.__akashaProbe` 的轮询。
    let opened_raw = client.eval_js(OPEN_AND_WATCH).await.unwrap();
    eprintln!("open_session → {opened_raw}");

    // 1. 会话真的开起来了（handle 由后端分配）。
    let opened = client
        .wait_for_expression(
            "window.__akashaProbe.handle !== null || window.__akashaProbe.error !== null",
            None,
            Some(20_000),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        opened.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "open_session 超时：{opened}"
    );
    assert!(
        client
            .eval_js("window.__akashaProbe.error === null")
            .await
            .map(|v| ok(&v))
            .unwrap_or(false),
        "open_session 报错了"
    );

    // 2. 写一条命令，并等**回显真的出现**（不是 sleep 猜）。
    client
        .eval_js(&write_js("echo akasha-raw-probe\n"))
        .await
        .unwrap();
    let echoed = client
        .wait_for_expression(
            "window.__akashaProbe.text.includes('akasha-raw-probe')",
            None,
            Some(20_000),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        echoed.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "没等到命令回显：{echoed}"
    );

    // 3. 大输出：raw 通道必须扛得住（这一条才是本 plan 的判据）。
    let before = number(&client.eval_js("window.__akashaProbe.bytes").await.unwrap());
    let wrote = client
        .eval_js(&write_js("yes akasha | head -c 10000000\n"))
        .await
        .unwrap();
    eprintln!("write_session → {wrote}");

    let flooded = client
        .wait_for_expression(
            "window.__akashaProbe.bytes >= 10000000",
            None,
            Some(120_000),
            None,
        )
        .await
        .unwrap();
    let after = number(&client.eval_js("window.__akashaProbe.bytes").await.unwrap());
    let batches = number(
        &client
            .eval_js("window.__akashaProbe.batches")
            .await
            .unwrap(),
    );
    assert_eq!(
        flooded.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "10 MB 没有全部到达（{before} → {after} 字节）：{flooded}"
    );
    eprintln!(
        "✅ raw 通道送达 {after} 字节，分 {batches} 批（等待耗时 {} ms）",
        flooded
            .get("elapsed_ms")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into())
    );

    // 4. 帧是**二进制**而不是 JSON 数组（tauri#13138 那类回归的哨兵）。
    let json_frames = number(
        &client
            .eval_js("window.__akashaProbe.jsonFrames")
            .await
            .unwrap(),
    );
    let frame_type = client
        .eval_js("window.__akashaProbe.frameType")
        .await
        .map(|v| payload(&v).as_str().unwrap_or("?").to_string())
        .unwrap_or_else(|_| "?".into());
    eprintln!("帧类型：{frame_type}（JSON 帧 {json_frames} 个）");
    assert_eq!(
        json_frames, 0,
        "raw 通道退化成 JSON 数组了（帧类型 {frame_type}）—— 这正是 no-string-pty-channel 守的东西"
    );

    // 5. 合批确实在工作：批次数量必须**远小于**字节数 —— 逐字节 emit 会在这里露馅。
    assert!(
        batches > 0 && batches < after / 1024,
        "{after} 字节只分了 {batches} 批，合批没生效？"
    );

    // 6. 收尾：显式关闭（后端 kill + wait 收尸）。
    client
        .eval_js(
            r#"window.__TAURI_INTERNALS__.invoke("close_session", { handle: window.__akashaProbe.handle }).then(() => { window.__akashaProbe.closed = true; return true; }).catch((e) => { window.__akashaProbe.error = String(e); return false; })"#,
        )
        .await
        .unwrap();
    let closed = client
        .wait_for_expression(
            "window.__akashaProbe.closed === true || window.__akashaProbe.error !== null",
            None,
            Some(20_000),
            None,
        )
        .await
        .unwrap();
    assert!(
        client
            .eval_js("window.__akashaProbe.closed === true")
            .await
            .map(|v| ok(&v))
            .unwrap_or(false),
        "close_session 没成功：{closed}"
    );
    assert!(
        client
            .eval_js("window.__akashaProbe.error")
            .await
            .map(|v| v.is_null())
            .unwrap_or(false),
        "整条路径上不该有任何错误"
    );

    // 7. 结束帧必须到达，而且探针**认得它**。
    //    这不是内部细节：官方 `Channel` 就是靠这个 `{end:true}` 帧把回调注销掉的
    //    （`cleanupCallback`），所以它是线上格式的一部分。同时它也是坑 #39 的哨兵 ——
    //    收尾帧被当成数据帧解，就会在 console 里留下一个 TypeError。
    let ended = client
        .wait_for_expression(
            "window.__akashaProbe.endFrames >= 1",
            None,
            Some(20_000),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        ended.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "关闭会话后没有收到频道的结束帧：{ended}"
    );
    eprintln!(
        "✅ 收尾帧 {} 个；console 里没有异常留下",
        number(
            &client
                .eval_js("window.__akashaProbe.endFrames")
                .await
                .unwrap()
        )
    );
}
