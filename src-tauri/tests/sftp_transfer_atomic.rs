//! plan 0702 的**端到端**验收：本机 ↔ 主机的双向传输，落盘一律"临时名 + 原子重命名"。
//!
//! 判据（ROADMAP 原文）=「中断传输后目标目录里**没有**看似完整的文件」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 一栏可以是**本机**（不需要主机、不需要终端） | 左栏选「本机」+ 前往一个目录，列得出来 |
//! | **取消**不留半成品 | 传一个大文件 → 有进度 → 取消 → 目标目录**一个条目都没有** |
//! | **成功**留下的是最终名，且字节相同 | 上传小文件 → 对端**真盘**上字节与源相同、没有临时名 |
//! | **关闭 `Session`** 也要清干净 | 再传一次 → 有进度 → 结束会话 → 目标目录仍只有上一次的文件 |
//! | 进度是真的在动 | `data-transfer-done` 由后端给（探针同源），取消前它已经 > 0 |
//!
//! ## 为什么服务端要放慢
//!
//! 本机回环上 4 MiB 会在一瞬间搬完，用例来不及点"取消" —— 那是一条随机器快慢而红的用例。
//! `ServerOptions::sftp_delay` 让每一次读都等 20 ms（4 MiB ÷ 32 KiB × 20 ms ≈ 2.5 s），
//! 于是"取消"发生在**确定的位置**：传输确实在跑，且还剩很多没搬。
//!
//! ## 两半证据都在
//!
//! 目标目录由**测试进程**直接读（那是 app 之外的真盘），传输的状态由 `sftp` 探针读 ——
//! 前者答"文件到底在哪"，后者答"后端自己怎么记这笔账"。两者必须同时成立。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use akasha_lib::ssh::testing::{ServerOptions, SftpItem, start};
use akasha_store::pools::hosts;
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, USER, click, connect_and_prepare, fill_input, forget, open_vault, text, text_of,
    unlock, wait_js,
};
use victauri_test::VictauriClient;

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-sftp-transfer-password";
/// 池里那台主机的名字。
const HOST_NAME: &str = "e2e-sftp-transfer";
/// 下载的源文件多大。配合服务端的 20 ms/块 = 128 块 ≈ 2.5 s，够"点得到取消"。
const BIG_BYTES: usize = 4 * 1024 * 1024;
/// 上传的源文件多大（够分几块，又不必等）。
const SMALL_BYTES: usize = 300 * 1024;

/// 测试进程自己的一个临时目录（判据的读数口：目标盘就在这里面）。
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "akasha-e2e-transfer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// **"这个目录里到底有什么"的唯一读数口** —— 判据的每一句都断言在它上面。
    fn names(&self) -> Vec<String> {
        names(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 一个目录里的条目名（排序）。**排序**：判据比的是"有什么"，不是"服务端以什么顺序列出来"。
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 在库里摆好这次要用的那行主机（主机池的界面仍未规划，所以直接写库 —— 同 plan 0701）。
fn seed(path: &Path, port: u16) -> u32 {
    let conn = open_vault(path);
    forget(&conn, HOST_NAME, "127.0.0.1", port);
    let id = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: HOST_NAME.to_owned(),
            host: "127.0.0.1".to_owned(),
            port,
            user: USER.to_owned(),
            auth: hosts::Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();
    u32::try_from(id).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_transfers_never_leave_a_half_written_file() {
    if support::skip_unless_e2e() {
        return;
    }

    // ── 1. 一台服务端（在**本进程**里）：根目录是**真盘**上的一棵临时目录 ─────────
    let mut options = ServerOptions::password(PASSWORD);
    options.sftp = Some(vec![
        SftpItem::file_with("big.bin", vec![b'B'; BIG_BYTES]),
        SftpItem::dir("sub"),
    ]);
    // 每一次读 / 写都等 20 ms —— 让"取消"有确定的落点（见文件头）。
    options.sftp_delay = Some(Duration::from_millis(20));
    let server = start(options).await;
    let root = server
        .sftp_root()
        .expect("这台服务端应当提供 SFTP")
        .to_path_buf();
    eprintln!(
        "服务端 127.0.0.1:{} · 根目录 {}",
        server.addr.port(),
        root.display()
    );

    let scratch = Scratch::new();
    eprintln!("本机目标目录 {}", scratch.path().display());

    let Some((mut client, _fixture, vault)) = connect_and_prepare().await else {
        return;
    };

    // ── 2. 种子数据 + 解锁 ─────────────────────────────────────────────────
    let host_id = seed(&vault, server.addr.port());
    unlock(&mut client).await;

    // ── 3. 界面：打开面板、新建会话，左栏选**本机**、右栏选那台主机 ──────────────
    open_sftp_panel(&mut client).await;
    start_session(&mut client).await;
    choose_origin(&mut client, "left", "local").await;
    choose_origin(&mut client, "right", &format!("host:{host_id}")).await;

    // 本机那一档没有认证，点一下就通。
    click(
        &mut client,
        ".sftp-pane[data-side=\"left\"] .sftp-connect",
        "连接左栏（本机）",
    )
    .await;
    wait_js(
        &mut client,
        "document.querySelector('.sftp-pane[data-side=\"left\"]')?.dataset.sftpState === 'connected'",
        10_000,
        "左栏连上本机",
    )
    .await;

    // 前往测试自己的临时目录（本机那一栏的起点是家目录，判据要的是一个确定的目录）。
    goto_dir(&mut client, "left", scratch.path()).await;

    // 右栏连那台主机（要答主机密钥与口令两轮 —— 步数由用例自己数，不写死）。
    let asked = connect_side(&mut client, "right", &server.fingerprint).await;
    eprintln!("右侧连上，问到过 {asked:?}");
    wait_js(
        &mut client,
        "!!document.querySelector('.sftp-pane[data-side=\"right\"] \
         .sftp-entry[data-entry-name=\"big.bin\"]')",
        10_000,
        "右栏列出了对端的 big.bin",
    )
    .await;

    // ── 4. 判据一：取消不留半成品 ───────────────────────────────────────────
    click(
        &mut client,
        ".sftp-pane[data-side=\"right\"] .sftp-send[data-entry-name=\"big.bin\"]",
        "把 big.bin 传到左栏（下载）",
    )
    .await;
    // 等**进度真的动了**：那说明字节已经在往本机盘上写，而写的是临时名。
    wait_js(
        &mut client,
        "(() => { const row = document.querySelector('.sftp-transfer'); \
         return !!row && Number(row.dataset.transferDone) > 0; })()",
        15_000,
        "传输有了进度",
    )
    .await;
    let (state, done, total) = transfer_row(&mut client).await;
    assert_eq!(state, "running", "还在搬的时候状态应当是运行中");
    assert_eq!(total, BIG_BYTES as f64, "分母应当是源文件的大小");
    assert!(done < total, "此刻它还没搬完（否则这条用例什么都没验到）");
    // 传输中：目标目录里**只有临时名**，没有最终名 —— 落盘不变量的前半。
    assert_eq!(
        scratch.names(),
        vec![".big.bin.part".to_owned()],
        "传输中目标目录里只该有临时名"
    );
    eprintln!(
        "取消之前：搬了 {done} / {total} 字节，目标目录 {:?}",
        scratch.names()
    );

    click(&mut client, ".sftp-transfer-cancel", "取消这次传输").await;
    // ⚠️ 等的是**状态不再是 running**（后端那条任务收工之后才写这个值），
    // 而不是等一段时间 —— 清理有没有落地由它答（`AGENTS.md` §7）。
    wait_js(
        &mut client,
        "(() => { const row = document.querySelector('.sftp-transfer'); \
         return !!row && row.dataset.transferState !== 'running'; })()",
        20_000,
        "取消已经落地",
    )
    .await;
    let (state, done, _) = transfer_row(&mut client).await;
    assert_eq!(state, "cancelled", "取消之后状态应当是已取消");
    assert!(done > 0.0, "它确实搬过一部分（否则取消的时机没对上）");

    assert!(
        scratch.names().is_empty(),
        "取消之后目标目录必须是空的（既没有 big.bin，也没有临时名）：{:?}",
        scratch.names()
    );
    eprintln!(
        "取消之后：目标目录里 {} 个条目（既没有 big.bin，也没有临时名）",
        scratch.names().len()
    );

    // 探针那一半：后端自己记的也是"已取消"。
    let entries = sftp_entries(&mut client).await;
    let transfers = entries[0]
        .pointer("/transfers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        transfers[0].pointer("/state").and_then(Value::as_str),
        Some("cancelled"),
        "探针里那条传输也该是已取消：{transfers:?}"
    );

    // ── 5. 判据二：成功留下的是最终名，字节一模一样 ──────────────────────────
    let payload: Vec<u8> = (0u8..=255).cycle().take(SMALL_BYTES).collect();
    std::fs::write(scratch.path().join("small.bin"), &payload).unwrap();
    // 让左栏重新列一次（新文件才会出现）。走的是同一条"前往"命令。
    goto_dir(&mut client, "left", scratch.path()).await;
    wait_js(
        &mut client,
        "!!document.querySelector('.sftp-pane[data-side=\"left\"] \
         .sftp-entry[data-entry-name=\"small.bin\"]')",
        10_000,
        "左栏列出了刚写下的 small.bin",
    )
    .await;

    click(
        &mut client,
        ".sftp-pane[data-side=\"left\"] .sftp-send[data-entry-name=\"small.bin\"]",
        "把 small.bin 传到右栏（上传）",
    )
    .await;
    // 最新那条传输（列表新的在前）跑到 done 为止。
    wait_js(
        &mut client,
        "(() => { const row = document.querySelector('.sftp-transfer'); \
         return !!row && row.dataset.transferState === 'done'; })()",
        30_000,
        "上传完成",
    )
    .await;

    // 对端**真盘**上对账：字节相同，而且没有留下任何临时名。
    assert_eq!(
        std::fs::read(root.join("small.bin")).unwrap(),
        payload,
        "对端盘上的字节必须与源相同"
    );
    assert_eq!(
        names(&root),
        vec![
            "big.bin".to_owned(),
            "small.bin".to_owned(),
            "sub".to_owned()
        ],
        "对端目录里不该多出临时名"
    );
    eprintln!("上传之后：对端盘上 small.bin 的字节数与源一致");

    // ── 6. 判据三：关闭 Session 也清干净 ────────────────────────────────────
    click(
        &mut client,
        ".sftp-pane[data-side=\"right\"] .sftp-send[data-entry-name=\"big.bin\"]",
        "再传一次 big.bin（这次靠关闭会话收尾）",
    )
    .await;
    wait_js(
        &mut client,
        "(() => { const row = document.querySelector('.sftp-transfer'); \
         return !!row && row.dataset.transferState === 'running' \
         && Number(row.dataset.transferDone) > 0; })()",
        15_000,
        "第二次下载有了进度",
    )
    .await;
    assert_eq!(
        scratch.names(),
        vec![".big.bin.part".to_owned(), "small.bin".to_owned()],
        "第二次传输中：上一次的文件在，这一次的临时名在，最终名不该有"
    );

    click(&mut client, ".sftp-stop", "结束 SFTP 会话").await;
    wait_js(
        &mut client,
        "!document.querySelector('.sftp-pane[data-side=\"left\"]')",
        30_000,
        "会话结束后两栏消失（这一刻清理已经落地 —— `sftp_close` 会等）",
    )
    .await;
    assert!(
        scratch.names() == vec!["small.bin".to_owned()],
        "关会话之后目标目录里只该剩下上一次成功那个文件：{:?}",
        scratch.names()
    );

    // 探针：会话没了（两侧的连接也随之断开）。
    assert!(
        sftp_entries(&mut client).await.is_empty(),
        "关掉之后探针该是空的"
    );

    // 对端：那条连接确实断了（外部证据 —— 我们自己说"关了"不算）。
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    loop {
        if support::observed(&server).connections_closed >= 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "会话关掉了，服务端却还看到连接挂着"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let _ = client.invoke_command("vault_lock", None).await;
}

// ── 驱动界面与读后端的辅助 ───────────────────────────────────────────────────

/// 打开 SFTP 面板（并保证它重新读一遍池子与已有会话）。
async fn open_sftp_panel(client: &mut VictauriClient) {
    if !text_of(client, ".sftp-panel").await.is_empty() {
        click(
            client,
            ".sftp-close",
            "关闭 SFTP 面板（好让它重新读一次池子）",
        )
        .await;
    }
    click(client, ".tab-new-sftp", "打开 SFTP 面板").await;
    wait_js(
        client,
        "!!document.querySelector('.sftp-panel')",
        10_000,
        "SFTP 面板打开了",
    )
    .await;
}

/// 保证面板上是一个**新建的**会话（上一个 E2E 目标可能留了一个）。
async fn start_session(client: &mut VictauriClient) {
    if !text_of(client, ".sftp-stop").await.is_empty() {
        click(client, ".sftp-stop", "结束上一个目标留下的 SFTP 会话").await;
        wait_js(
            client,
            "!!document.querySelector('.sftp-start')",
            10_000,
            "回到新建状态",
        )
        .await;
    }
    click(client, ".sftp-start", "新建 SFTP 会话").await;
    wait_js(
        client,
        "!!document.querySelector('.sftp-pane[data-side=\"left\"]')",
        10_000,
        "两栏出现了",
    )
    .await;
}

/// 在某一栏里选来源（`local`，或者 `host:<id>`）。
///
/// 为什么要绕过 React 的 value tracker（用原型上的 setter 再手动派发 `change`）：
/// 直接写 `select.value = …` 时 React 记着上一次的值，`change` 会被它当成"没变"。
async fn choose_origin(client: &mut VictauriClient, side: &str, value: &str) {
    let script = format!(
        "(() => {{
            const select = document.querySelector('.sftp-pane[data-side=\"{side}\"] .sftp-origin');
            if (!select) return 'no-select';
            const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
            setter.call(select, '{value}');
            select.dispatchEvent(new Event('change', {{ bubbles: true }}));
            return select.value;
        }})()"
    );
    let chosen = text(&client.eval_js(&script).await.unwrap());
    assert_eq!(
        chosen, value,
        "{side} 那一栏的来源没选上（得到 {chosen:?}）"
    );
}

/// 在某一栏里前往一个目录（填路径 → 点「前往」）。
///
/// ⚠️ 两次 `eval_js`：React 的受控输入要等它把这一次事件的重渲染提交完，
/// 同一个 JS 任务里紧接着 `.click()` 提交的是**上一次渲染**的空值（见 `support::fill_input`）。
async fn goto_dir(client: &mut VictauriClient, side: &str, dir: &Path) {
    fill_input(
        client,
        &format!(".sftp-pane[data-side=\"{side}\"] .sftp-goto"),
        dir.to_string_lossy().as_ref(),
    )
    .await;
    click(
        client,
        &format!(".sftp-pane[data-side=\"{side}\"] .sftp-goto-go"),
        "前往那个目录",
    )
    .await;
    wait_js(
        client,
        &format!(
            "document.querySelector('.sftp-pane[data-side=\"{side}\"] .sftp-path')?.dataset.sftpPath !== ''"
        ),
        10_000,
        "那一栏列出了目标目录",
    )
    .await;
}

/// 连某一侧：点「连接」，按类型回答提示，直到那一侧变成 `connected`。
///
/// 不写死"先密钥后口令"两步：链上每一跳各来一轮（plan 0505 的教训），写死步数会在
/// 拓扑一变时**静默少答一轮**，表现是"连不上"而不是"用例写错了"。
async fn connect_side(client: &mut VictauriClient, side: &str, fingerprint: &str) -> Vec<String> {
    let mut asked = Vec::new();
    click(
        client,
        &format!(".sftp-pane[data-side=\"{side}\"] .sftp-connect"),
        "连接这一侧",
    )
    .await;

    let deadline = Instant::now() + Duration::from_secs(90);
    let any_prompt = "!!document.querySelector('.ssh-prompt[data-prompt-kind=\"hostKey\"]') \
                      || !!document.querySelector('.ssh-prompt[data-prompt-kind=\"credential\"]')";
    loop {
        match side_state(client, side).await.as_str() {
            "connected" => return asked,
            "failed" => {
                let failure = text_of(
                    client,
                    &format!(".sftp-pane[data-side=\"{side}\"] .sftp-failure"),
                )
                .await;
                panic!("{side} 这一侧连接失败：{failure}");
            }
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "{side} 这一侧没连上（问到过的：{asked:?}）"
        );

        let _ = client
            .wait_for_expression(any_prompt, None, Some(3_000), None)
            .await;

        let presented = text_of(
            client,
            ".ssh-prompt[data-prompt-kind=\"hostKey\"] .ssh-prompt-fingerprint",
        )
        .await;
        if !presented.is_empty() {
            assert_eq!(presented, fingerprint, "提示里那串指纹不属于这个服务端");
            asked.push(format!("hostKey:{presented}"));
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
            support::fill_secret(client, PASSWORD).await;
            click(
                client,
                ".ssh-prompt[data-prompt-kind=\"credential\"] .ssh-prompt-submit",
                "提交口令",
            )
            .await;
        }
    }
}

/// 某一侧在界面上的状态（`data-sftp-state` 就是后端的短名）。
async fn side_state(client: &mut VictauriClient, side: &str) -> String {
    let raw = client
        .eval_js(&format!(
            "document.querySelector('.sftp-pane[data-side=\"{side}\"]')?.dataset.sftpState ?? ''"
        ))
        .await
        .unwrap();
    text(&raw)
}

/// 最新那条传输的 `(状态, 已搬字节, 总字节)` —— 三个数都来自后端。
async fn transfer_row(client: &mut VictauriClient) -> (String, f64, f64) {
    let raw = client
        .eval_js(
            "(() => { const row = document.querySelector('.sftp-transfer'); if (!row) return null; \
             return { state: row.dataset.transferState, \
                      done: Number(row.dataset.transferDone), \
                      total: Number(row.dataset.transferTotal) }; })()",
        )
        .await
        .unwrap();
    let value = support::payload(&raw);
    assert!(!value.is_null(), "界面上还没有传输那一行：{value}");
    (
        value
            .pointer("/state")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        value
            .pointer("/done")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        value
            .pointer("/total")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    )
}

/// `app_state { probe: "sftp" }` 的原始列表。
async fn sftp_entries(client: &mut VictauriClient) -> Vec<Value> {
    let value = client
        .call_tool("app_state", json!({ "probe": "sftp" }))
        .await
        .expect("读不到 sftp probe —— 它注册进 lib.rs 了吗？");
    value.as_array().cloned().unwrap_or_default()
}
