//! plan 0905 的**端到端**验收：登录 / 解锁 / 锁定接进前端（含自托管）。
//!
//! 判据（ROADMAP 原文）=「设自托管地址后登录到未解锁态、解锁到已解锁态、锁定后内存里不再有
//! token；三态与 CLI 自报一致」。逐条落点：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 两个轴是两件事 | `host` 轴上这台机器用不了 `bw`（要么 PATH 里没有、要么有却跑不起来）；切到 `managed` 而还没下载时报"还没有下载过"（ADR-0007 D5）。⚠️ 两条报错**必须不同** —— 否则用户不知道自己是该装一个还是该点下载 |
//! | 解析出 CLI | 落点里放一份假 `bw` → 命令报出它的版本与变体（`oss`），面板上写着同一串 |
//! | 自托管地址 | 面板填地址 → 点"使用这个服务器" → 面板显示**读回来的**那一串（`bw config server` 的回读） |
//! | 口令错 | 面板上的那句话是 `bw` 自己的原话（`commandFailed` 档：**不猜类别、照抄**） |
//! | 登录 | 口令对 → 面板说"已解锁"、内存里有 session key |
//! | 锁定 | 点锁定 → 面板说"已登录，未解锁"、内存里没有 session key |
//! | 状态同步（D10） | **外部**把假 CLI 的解锁标记删掉（等价于用户在别处跑了 `bw lock`）→ 点"刷新状态" → 面板跟到 `locked` 且 session 被丢掉 |
//!
//! ## 为什么下载那一段不在这一条里
//!
//! `bw_cli_install` 拉的是上游的资产（约 45 MB，需要网络）。**判据的构造物必须是本机能造的**，
//! 所以这里把"下载 → 解包 → 落盘"交给 `akasha-bw` crate 层的用例（进程内假上游，
//! `akasha_bw::testing::Stub`），而这一条只验**app 接线那一半**：轴怎么解析、命令怎么串、
//! 面板怎么显示、状态怎么同步。
//!
//! ⚠️ 这也是"下载"这件事在本机的**唯一**自动化证据；真实上游那一次是人工跑的
//! （记录在 `docs/STATUS.md` 的「已验证为通过」里）。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{
    click, connect_and_prepare, fill_input, open_bitwarden_panel, text_of, wait_text_contains,
};
use victauri_test::VictauriClient;

/// 假 `bw` 自报的版本。**与目录名一致**（`installed_versions` 按目录名挑最新版本）。
const FAKE_VERSION: &str = "2026.8.0";
/// 假 `bw` 认得的那条口令。
const RIGHT_PASSWORD: &str = "e2e-right-password";
/// 那一条它不认的。
const WRONG_PASSWORD: &str = "e2e-wrong-password";
/// 自托管地址（**不会被真的访问**：假 `bw` 只把它记下来）。
const SERVER: &str = "https://vault.e2e.invalid";

/// 假 `bw`：只实现本集成用到的那几条命令，状态放在 `BITWARDENCLI_APPDATA_DIR` 里。
///
/// 与 `FakeSerialDevice` 同一个思路 —— 本机没有真实 vault（那需要一个账号），
/// 而"命令怎么拼、状态怎么读、界面怎么跟"这几件事**不需要** vault 就能钉住。
/// 脚本里的每条命令都照抄上游实测的输出形状（`docs/bitwarden.md` §7）。
fn fake_bw_script() -> &'static str {
    r#"#!/bin/sh
here=$(dirname "$0")
appdir=${BITWARDENCLI_APPDATA_DIR:-$here}
mkdir -p "$appdir" 2>/dev/null
logged="$appdir/e2e-logged-in"
unlocked="$appdir/e2e-unlocked"
serverfile="$appdir/e2e-server"
server() { if [ -f "$serverfile" ]; then cat "$serverfile"; else printf 'https://vault.bitwarden.com'; fi; }
case "$1" in
  --version) printf '2026.8.0\n' ;;
  --help) printf 'Usage: bw [options] [command]\n\nCommands:\n  login [options] [email] [password]  Log into a user account.\n  status                              Show server, last sync, user information, and vault status.\n' ;;
  status)
    if [ -f "$logged" ]; then
      if [ -f "$unlocked" ]; then st=unlocked; else st=locked; fi
      printf '{"serverUrl":"%s","lastSync":null,"userEmail":"e2e@example.com","userId":"00000000-0000-0000-0000-000000000000","status":"%s"}\n' "$(server)" "$st"
    else
      printf '{"serverUrl":null,"lastSync":null,"status":"unauthenticated"}\n'
    fi ;;
  config)
    if [ "$2" = "server" ]; then
      if [ -n "$3" ]; then printf '%s' "$3" > "$serverfile"; printf 'Saved setting\n'; else printf '%s' "$(server)"; fi
    fi ;;
  login)
    if [ "$BW_PASSWORD" = "e2e-right-password" ]; then
      : > "$logged"; : > "$unlocked"; printf 'e2e-session-key\n'
    else
      printf 'Username or password is incorrect. Try again.\n'; exit 1
    fi ;;
  unlock)
    if [ "$BW_PASSWORD" = "e2e-right-password" ]; then
      : > "$unlocked"; printf 'e2e-session-key\n'
    else
      printf 'Invalid master password.\n'; exit 1
    fi ;;
  lock) rm -f "$unlocked"; printf 'Vault locked.\n' ;;
  logout) rm -f "$logged" "$unlocked"; printf 'You have logged out.\n' ;;
  sync) printf 'Syncing complete.\n' ;;
  *) printf 'unknown command\n' ;;
esac
"#
}

/// 把假 `bw` 放进 `managed` 那一轴的落点（`<数据目录>/bitwarden/bw-<版本>/`）。
fn install_fake_bw(data_dir: &Path) -> PathBuf {
    let dir = data_dir
        .join("bitwarden")
        .join(format!("bw-{FAKE_VERSION}"));
    fs::create_dir_all(&dir).expect("建版本目录");
    let path = dir.join(if cfg!(windows) { "bw.exe" } else { "bw" });
    fs::write(&path, fake_bw_script()).expect("写假 bw");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("给可执行位");
    }
    path
}

/// 假 `bw` 放自己那份状态的地方（= `appdata = managed` 时的隔离目录）。
fn fake_appdata(data_dir: &Path) -> PathBuf {
    data_dir.join("bitwarden").join("appdata")
}

async fn invoke(client: &mut VictauriClient, command: &str, args: Value) -> Value {
    client
        .invoke_command(command, Some(args))
        .await
        .unwrap_or_else(|err| panic!("`{command}` 调不通 —— 它登记进 bindings.rs 了吗？{err:?}"))
}

/// `bw_cli_status` 的快照。
async fn cli_status(client: &mut VictauriClient) -> Value {
    invoke(client, "bw_cli_status", json!({})).await
}

#[tokio::test]
async fn bitwarden_login_unlock_and_lock_are_visible_in_the_panel() {
    if support::skip_unless_e2e() {
        return;
    }
    let Some((mut client, _fixture, vault)) = connect_and_prepare().await else {
        return;
    };
    let Some(data_dir) = vault.parent().map(Path::to_path_buf) else {
        panic!("库没有父目录");
    };

    // ── 1. 两个轴是两件事（ADR-0007 D5）────────────────────────────────────
    // 先回到默认那一档：别的 E2E 目标留下的设置不该影响这条判据。
    let host = invoke(
        &mut client,
        "bw_cli_settings",
        json!({ "binary": "host", "appdata": "host" }),
    )
    .await;
    let host_problem = host["cli"]["problem"].as_str().map(str::to_owned);
    // ⚠️ 这台机器上**确实**有一个 `bw`（发行版的 `bitwarden-cli` 把 `/usr/bin/bw` 指向 npm 包），
    // 而它在只读家目录里跑不起来 —— 所以这里看到的多半是"跑不起来"而不是"找不到"。
    // 两种都算"这一轴用不了"，断言因此只要求**有一句可读的话、并且提到了 bw**；
    // 是哪一种由日志记下来（这条用例的输出会进 `just test-e2e` 的日志）。
    eprintln!("host 轴上的话 = {host_problem:?}");
    match &host_problem {
        Some(problem) => assert!(
            problem.contains("bw"),
            "`host` 轴用不了时，那句话要说清是这一件事：{problem}"
        ),
        None => eprintln!(
            "注意：这台机器上 host 轴是可用的（版本 {:?}）—— 这一条的对照物因此弱一些",
            host["cli"]["version"]
        ),
    }

    // ⚠️ 清掉上一次运行留下的东西：这个数据目录被**所有** E2E 目标共用，而假 bw 会留在
    // `bitwarden/` 里 —— 不清的话"还没下载过"这一条判据会被上一次的残留顶过去
    //（第一版就是这么红的）。
    let _ = fs::remove_dir_all(data_dir.join("bitwarden"));

    // 还没下载那一段：同样是"用不了"，但**是另一句话**。
    let managed = invoke(
        &mut client,
        "bw_cli_settings",
        json!({ "binary": "managed", "appdata": "managed" }),
    )
    .await;
    let managed_problem = managed["cli"]["problem"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        managed_problem.contains("还没有下载过"),
        "`managed` 轴上还没装过，报的该是这一句：{managed_problem}"
    );
    assert_ne!(
        Some(managed_problem),
        host_problem,
        "两条轴的「用不了」必须分得开"
    );

    // ── 2. 落点里有一份可执行的 bw 之后，解析出它的版本与变体 ──────────────
    let binary = install_fake_bw(&data_dir);
    let status = cli_status(&mut client).await;
    assert_eq!(status["cli"]["version"].as_str(), Some(FAKE_VERSION));
    assert_eq!(
        status["cli"]["variant"].as_str(),
        Some("oss"),
        "假 bw 的 `--help` 没有 device-approval 那一行"
    );
    let expected_program = binary.to_string_lossy().into_owned();
    assert_eq!(
        status["cli"]["program"].as_str(),
        Some(expected_program.as_str()),
        "解析到的就是落点里那一个"
    );
    assert!(
        status["cli"]["licenseNotice"].is_null(),
        "OSS 不该有许可证提示"
    );

    // ── 3. 面板：打开就显示版本与变体、三态是"未登录" ─────────────────────
    open_bitwarden_panel(&mut client).await;
    wait_text_contains(&mut client, "[data-bw-cli]", FAKE_VERSION).await;
    wait_text_contains(&mut client, "[data-bw-cli]", "OSS").await;
    wait_text_contains(&mut client, "[data-bw-status]", "未登录").await;
    assert!(
        text_of(&mut client, "[data-bw-has-session]")
            .await
            .contains("没有 session key"),
        "还没登录就不该有 session key"
    );

    // ── 4. 自托管服务器地址：写进去，显示的必须是**读回来的**那一个 ────────
    fill_input(&mut client, "[data-bw-server]", SERVER).await;
    click(&mut client, "[data-bw-server-set]", "使用这个服务器").await;
    wait_text_contains(&mut client, "[data-bw-server-current]", SERVER).await;

    // ── 5. 口令错：面板上那句话是 `bw` 自己的原话（照抄，不猜类别）────────
    fill_input(&mut client, "[data-bw-email]", "e2e@example.com").await;
    fill_input(&mut client, "[data-bw-password]", WRONG_PASSWORD).await;
    click(&mut client, "[data-bw-login]", "登录").await;
    wait_text_contains(
        &mut client,
        "[data-bw-failure]",
        "Username or password is incorrect",
    )
    .await;
    // 失败之后状态仍是"未登录"（界面不许自己推断成别的）。
    wait_text_contains(&mut client, "[data-bw-status]", "未登录").await;

    // ── 6. 口令对：登录即解锁，session key 在内存里 ───────────────────────
    fill_input(&mut client, "[data-bw-password]", RIGHT_PASSWORD).await;
    click(&mut client, "[data-bw-login]", "登录").await;
    wait_text_contains(&mut client, "[data-bw-status]", "已解锁").await;
    wait_text_contains(&mut client, "[data-bw-has-session]", "在内存里").await;
    // ⚠️ 提交之后界面里那份主密码要被清掉（它已经交给后端了）。
    let typed = text_of(&mut client, "[data-bw-password]").await;
    assert!(
        typed.is_empty(),
        "主密码提交之后不该留在输入框里：{typed:?}"
    );

    // ── 7. 锁定：CLI 那边失效、内存里那份也没了 ──────────────────────────
    click(&mut client, "[data-bw-lock]", "锁定").await;
    wait_text_contains(&mut client, "[data-bw-status]", "已登录，未解锁").await;
    wait_text_contains(&mut client, "[data-bw-has-session]", "没有 session key").await;

    // ── 8. 解锁：回到已解锁 ─────────────────────────────────────────────
    fill_input(&mut client, "[data-bw-password]", RIGHT_PASSWORD).await;
    click(&mut client, "[data-bw-unlock]", "解锁").await;
    wait_text_contains(&mut client, "[data-bw-status]", "已解锁").await;

    // ── 9. **外部**锁过之后界面跟得上（ADR-0007 D10）─────────────────────
    // 删掉假 CLI 的解锁标记 = 用户在别处跑了一次 `bw lock`：我们手里那份 key 已经失效，
    // 而界面上还写着"已解锁"。点一次刷新，它必须跟到 CLI 自己说的那一个状态，
    // 并且**把内存里那份丢掉**。
    let marker = fake_appdata(&data_dir).join("e2e-unlocked");
    assert!(
        marker.is_file(),
        "解锁之后假 CLI 该留下标记：{}",
        marker.display()
    );
    fs::remove_file(&marker).expect("外部锁定（删掉解锁标记）");

    click(&mut client, "[data-bw-refresh]", "刷新状态").await;
    wait_text_contains(&mut client, "[data-bw-status]", "已登录，未解锁").await;
    wait_text_contains(&mut client, "[data-bw-has-session]", "没有 session key").await;
    let after = cli_status(&mut client).await;
    assert_eq!(
        after["hasSession"],
        json!(false),
        "状态说不是解锁，就该丢掉手里的 key"
    );
    assert_eq!(after["status"]["state"].as_str(), Some("locked"));

    // ── 10. 登出：回到"未登录" ───────────────────────────────────────────
    click(&mut client, "[data-bw-logout]", "登出").await;
    wait_text_contains(&mut client, "[data-bw-status]", "未登录").await;

    // 面板与 probe 说的是同一件事（读数口只有一个来源）。
    let probe = invoke(&mut client, "bw_cli_status", json!({})).await;
    assert_eq!(probe["status"]["state"].as_str(), Some("unauthenticated"));

    // 收尾：把两个轴放回默认，并把假 bw 连同隔离状态一起清掉 ——
    // 下一个目标（或人）看到的应当是"这台机器上没有 bw"，不是一个假的。
    invoke(
        &mut client,
        "bw_cli_settings",
        json!({ "binary": "host", "appdata": "host" }),
    )
    .await;
    let _ = fs::remove_dir_all(data_dir.join("bitwarden"));
}
