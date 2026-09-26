//! plan 0704 的**端到端**验收：真 app 上"大量小文件"的并发搬运（ADR-0006 D6）。
//!
//! 判据（ROADMAP 原文）=「大量小文件的吞吐**显著优于**串行请求」。这条用例的两半都在真 app 上：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 上限可见 | `sftp` 探针与面板表头的三个数（`limit` / `live` / `peak`）一致 |
//! | 串行基线 | 12 个文件**逐个**发、逐个等结束 —— `peak == 1`，记墙钟 |
//! | 并发真的起来 | 12 个文件一口气发完 —— `1 < peak <= limit`，记墙钟，且 `并发 × 2 < 串行` |
//! | 落盘不变量没丢 | 对端**真盘**上 12 个文件的字节与源相同、目录里**没有临时名** |
//!
//! ## 为什么这一条要给自己接一条带时延的链路
//!
//! 本机回环的一次往返在微秒级，而并发 in-flight 的收益**全部**来自往返（`scope.md` §4.1）——
//! 不放大它，"串行 vs 并发"的差距会被系统噪声淹掉，用例只能变成一条随机红。所以主机行指向
//! [`slow_link`]（每个方向的每一段延后 10 ms），app 对此一无所知：它连的是"一台主机"。
//!
//! 断言取一个宽裕的比值（`并发 × 2 < 串行`），而**数字本身不是门禁**（`AGENTS.md` §7）：
//! 它们打印出来进基线，用途是改动前后对比。
//!
//! ## 两半证据都在
//!
//! "同时几个在搬"读 `sftp` 探针（后端自己怎么记这笔账），"文件到底在不在对端盘上"由**测试
//! 进程**直接读服务端那棵真目录（app 之外的事实）。判据的每一句都同时落在这两半上。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use akasha_lib::ssh::testing::{ServerOptions, SftpItem, slow_link, start};
use akasha_lib::store::pools::hosts;
use serde_json::{Value, json};
use support::{
    USER, click, connect_and_prepare, fill_input, forget, open_vault, text, text_of, unlock,
    wait_js,
};
use victauri_test::VictauriClient;

/// 这条用例自己用的**登录**口令（库口令在 `support` 里）。**不是**用户的。
const PASSWORD: &str = "e2e-sftp-pipelining-password";
/// 池里那台主机的名字。
const HOST_NAME: &str = "e2e-sftp-pipelining";
/// 一共搬多少个文件，以及每个多大。
///
/// 12 个 1 KiB：一个文件的 SFTP 往返是 4 次（`open` / `write` / `close` / `rename`），
/// 交替走的时延下串行一批 ≈ 1.2 s，而并发一批远小于它 —— 差距落在"看得见"的量级上。
const FILES: usize = 12;
const BYTES: usize = 1024;
/// 遍历上限分档里**生效的那一个**（`akasha_lib::config::Transfer::default` 的默认值）。
///
/// 写死在这里是刻意的：这条用例要断言"上限真的在起作用"，而在起作用的那一刻，探针报的
/// `limit` 就是后端读到的配置值 —— 拿它当分母，用例就不会与配置分叉。
const EXPECTED_LIMIT: u32 = 8;

/// 测试进程自己的一个临时目录（判据的读数口：源文件在这里）。
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "akasha-e2e-pipelining-{}-{}",
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

    fn write(&self, name: &str, content: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 一个目录里的条目名（排序）。
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
async fn many_small_files_are_faster_when_they_are_in_flight() {
    if support::skip_unless_e2e() {
        return;
    }

    // ── 1. 一台服务端（在**本进程**里）+ 一条带时延的链路 ──────────────────────
    let mut options = ServerOptions::password(PASSWORD);
    options.sftp = Some(vec![SftpItem::dir("inbox")]);
    let server = start(options).await;
    let root = server
        .sftp_root()
        .expect("这台服务端应当提供 SFTP")
        .to_path_buf();
    let link = slow_link(server.addr, Duration::from_millis(10)).await;
    eprintln!(
        "构造: 服务端=127.0.0.1:{} 链路=127.0.0.1:{} 每段延后={:?} 根目录={}",
        server.addr.port(),
        link.addr.port(),
        link.delay,
        root.display()
    );

    let scratch = Scratch::new();
    // 12 个小文件，内容各不相同 —— "字节对不对"因此能逐条对上。
    let mut payloads = Vec::new();
    for index in 0..FILES {
        let content: Vec<u8> = (0u8..=255).cycle().take(BYTES).collect();
        let content = if index == 0 {
            content
        } else {
            let mut shifted = content.clone();
            shifted.rotate_left(index * 7);
            shifted
        };
        payloads.push(content.clone());
        scratch.write(&file_name(index), &content);
    }
    eprintln!("构造: 本机源目录={}", scratch.path().display());

    let Some((mut client, _fixture, vault)) = connect_and_prepare().await else {
        return;
    };

    // ── 2. 种子数据 + 解锁 ─────────────────────────────────────────────────
    // ⚠️ 主机行的地址是**链路**而不是服务端：app 对它一无所知，它连的是"一台主机"。
    let host_id = seed(&vault, link.addr.port());
    unlock(&mut client).await;

    // ── 3. 界面：左栏本机、右栏那台主机（经链路） ────────────────────────────
    open_sftp_panel(&mut client).await;
    start_session(&mut client).await;
    choose_origin(&mut client, "left", "local").await;
    choose_origin(&mut client, "right", &format!("host:{host_id}")).await;

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
    goto_dir(&mut client, "left", scratch.path()).await;
    wait_js(
        &mut client,
        &format!(
            "document.querySelectorAll('.sftp-pane[data-side=\"left\"] .sftp-entry').length == {FILES}"
        ),
        10_000,
        "左栏列出了那 12 个源文件",
    )
    .await;

    let asked = connect_side(&mut client, "right", &server.fingerprint).await;
    eprintln!("会话: 侧=右 提示数={}", asked.len());
    wait_js(
        &mut client,
        "document.querySelector('.sftp-pane[data-side=\"right\"]')?.dataset.sftpState === 'connected'",
        20_000,
        "右栏连上了那台主机",
    )
    .await;

    // 上限是后端读到的那个数（配置里来的）—— 后面的断言拿它当分母。
    let limit = in_flight(&mut client).await.0;
    assert_eq!(
        limit, EXPECTED_LIMIT,
        "并发上限应当是配置模型里的默认值（改了就同步这条用例）"
    );
    // 面板上那三个数与探针同源（`data-sftp-*` 就是后端给的）。
    wait_js(
        &mut client,
        &format!("document.querySelector('.sftp-in-flight')?.dataset.sftpLimit === '{limit}'"),
        10_000,
        "面板表头显示出并发读数",
    )
    .await;

    // ── 4. 串行基线：逐个发、逐个等它结束 ───────────────────────────────────
    let serial = Instant::now();
    for index in 1..=FILES {
        click(
            &mut client,
            &format!(
                ".sftp-pane[data-side=\"left\"] .sftp-send[data-entry-name=\"{}\"]",
                file_name(index - 1)
            ),
            "发一个文件",
        )
        .await;
        wait_transfer_settled(&mut client, index).await;
    }
    let serial = serial.elapsed();
    let (_, _, serial_peak) = in_flight(&mut client).await;
    assert_eq!(
        serial_peak, 1,
        "逐个等结束的那一批里，最多只能有 1 个文件同时在搬"
    );
    eprintln!("报告: 档=串行 文件数={FILES} 耗时={serial:.3?}");

    // ── 5. 并发：一口气全发出去 ─────────────────────────────────────────────
    let concurrent = Instant::now();
    for index in 0..FILES {
        click(
            &mut client,
            &format!(
                ".sftp-pane[data-side=\"left\"] .sftp-send[data-entry-name=\"{}\"]",
                file_name(index)
            ),
            "一口气发一个文件",
        )
        .await;
    }
    wait_transfer_settled(&mut client, FILES * 2).await;
    let concurrent = concurrent.elapsed();
    let (limit, live, peak) = in_flight(&mut client).await;
    eprintln!(
        "报告: 档=并发 文件数={FILES} 耗时={concurrent:.3?} limit={limit} live={live} peak={peak}"
    );

    assert!(live <= limit, "在搬的个数不该超过上限");
    assert!(
        peak > 1,
        "并发那一批里应当真的有不止一个文件同时在搬（peak {peak}）—— 否则这条用例什么都没验到"
    );
    assert!(
        peak <= limit,
        "peak 不该超过上限（peak {peak} > limit {limit}）"
    );
    // 全部结束之后 peak 不回落（它是这个会话见过的最大值）—— 它正是"上限在起作用"的读数。
    assert!(
        concurrent * 2 < serial,
        "并发的墙钟时间应当显著短于串行：串行 {serial:.3?}、并发 {concurrent:.3?}"
    );

    // ── 6. 判据的另一半：对端**真盘**上 12 个文件都在，字节一一对上，没有临时名 ──
    let mut expected: Vec<String> = (0..FILES).map(file_name).collect();
    // 右栏的当前目录是服务端的根（`realpath` 给的那个），种子里那个 `inbox` 也在那里。
    expected.push("inbox".to_owned());
    expected.sort();
    assert_eq!(
        names(&root),
        expected,
        "对端根目录里该恰好是那 12 个文件与种子目录"
    );
    for (index, payload) in payloads.iter().enumerate() {
        assert_eq!(
            std::fs::read(root.join(file_name(index))).unwrap(),
            *payload,
            "第 {index} 个文件的字节必须与源相同"
        );
    }
    eprintln!("服务端: 链路搬动段数={}", link.chunks());

    // ── 7. 收尾：结束会话（两侧断开、清理落地） ─────────────────────────────
    click(&mut client, ".sftp-stop", "结束 SFTP 会话").await;
    wait_js(
        &mut client,
        "!document.querySelector('.sftp-pane[data-side=\"left\"]')",
        30_000,
        "会话结束后两栏消失",
    )
    .await;
    let _ = client.invoke_command("vault_lock", None).await;
}

/// 第 `index` 个文件叫什么。
fn file_name(index: usize) -> String {
    format!("f{index:02}.bin")
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
/// 绕过 React 的 value tracker（用原型上的 setter 再手动派发 `change`）：直接写
/// `select.value = …` 时 React 记着上一次的值，`change` 会被它当成"没变"。
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
/// 不写死"先密钥后口令"两步：链上每一跳各来一轮，写死步数会在拓扑一变时**静默少答一轮**。
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
    // ⚠️ 完成条件读**探针**而不是界面：一栏在重新连接期间界面上的状态是上一次的
    // （plan 0703 的教训），而后端的事实随时可以断言。
    loop {
        if probe_side_state(client, side).await == "connected" {
            return asked;
        }
        assert!(
            Instant::now() < deadline,
            "{side} 这一侧没连上（问到过的：{asked:?}）"
        );
        let _ = client
            .wait_for_expression(any_prompt, None, Some(2_000), None)
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

/// 某一侧在**后端**的状态（读 `sftp` 探针）。
///
/// ⚠️ 不读界面上的 `data-sftp-state`：一栏在重新连接期间界面显示的是上一次的状态，
/// 而那正是 plan 0703 的 E2E 红过一次的地方（后端的事实随时可以断言，界面的呈现要先等到）。
async fn probe_side_state(client: &mut VictauriClient, side: &str) -> String {
    let entries = sftp_entries(client).await;
    entries
        .first()
        .and_then(|entry| entry.pointer("/sides"))
        .and_then(Value::as_array)
        .and_then(|sides| {
            sides
                .iter()
                .find(|info| info.pointer("/side").and_then(Value::as_str) == Some(side))
        })
        .and_then(|info| info.pointer("/state"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// 等"最新那条传输"不再是 `running`（读 `sftp` 探针 —— 后端那条任务收工之后才写它）。
///
/// 不用固定等待猜（`AGENTS.md` §7）：这里等的是后端自己记下的结局。
async fn wait_transfer_settled(client: &mut VictauriClient, at_least: usize) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let transfers = sftp_transfers(client).await;
        // 两个条件都要：这条传输**已经登记**（否则"没有 running"只是因为命令还没到后端），
        // 而且它**已经结束**（后端那条任务收工之后才写 state）。
        if transfers.len() >= at_least && running_transfers(client).await == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "传输一直没结束（探针里有 {} 条，期望至少 {at_least}）",
            transfers.len()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `sftp` 探针里还没结束的传输条数。
async fn running_transfers(client: &mut VictauriClient) -> usize {
    sftp_transfers(client)
        .await
        .iter()
        .filter(|transfer| transfer.pointer("/state").and_then(Value::as_str) == Some("running"))
        .count()
}

/// `sftp` 探针里那个会话的 `transfers` 列表。
async fn sftp_transfers(client: &mut VictauriClient) -> Vec<Value> {
    let entries = sftp_entries(client).await;
    entries
        .first()
        .and_then(|entry| entry.pointer("/transfers"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `sftp` 探针里那个会话的并发读数 `(limit, live, peak)`。
async fn in_flight(client: &mut VictauriClient) -> (u32, u32, u32) {
    let entries = sftp_entries(client).await;
    let read = |field: &str| {
        entries
            .first()
            .and_then(|entry| entry.pointer(&format!("/inFlight/{field}")))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or_else(|| panic!("探针里没有 inFlight/{field}：{entries:?}"))
    };
    (read("limit"), read("live"), read("peak"))
}

/// `app_state { probe: "sftp" }` 的原始列表。
async fn sftp_entries(client: &mut VictauriClient) -> Vec<Value> {
    let value = client
        .call_tool("app_state", json!({ "probe": "sftp" }))
        .await
        .expect("读不到 sftp probe —— 它注册进 lib.rs 了吗？");
    value.as_array().cloned().unwrap_or_default()
}
