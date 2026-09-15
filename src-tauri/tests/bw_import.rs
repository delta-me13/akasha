//! plan 0903 的**端到端**验收：从 Bitwarden 只读导入 SSH key 条目，再用导入进来的那把钥匙连上。
//!
//! 判据（ROADMAP 原文）=「导入后可用该密钥建立 SSH 连接」。逐条落点：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 只取 `type = 5`，其余一条都不进 | 假 `bw` 的 `list items --raw` 里同时有一条登录条目，报告里的 `sshKeys` 只数出 1，而池里只有那一把 |
//! | 报告说得清"哪几条进来了" | 面板上的报告带名字与**上游给的指纹** |
//! | 导入的钥匙接得到主机上 | `~/.ssh/config` 导入时 `IdentityFile` 的 basename 与钥匙名相同 → `vault_hosts` 里那一行 `keyId` 非空 |
//! | **可用该密钥建立 SSH 连接** | 用那一行开会话 → 连上，且服务端记下的 `offered_keys` 里就是**那个指纹**（"连上"与"用的是这把钥匙"是两件事，都要断） |
//! | 库锁着时不写 | 单独一步：不登录（没有 session）就导入 → 报的是"先登录/先解锁"那一档，池里一行都没多 |
//!
//! 第 8 / 9 段是 **plan 0904**（离线缓存）的判据：自检先报"完好"、把库里那把私钥换成另一把
//! 之后必须报"对不上"（诱饵 —— 少了它，"自检"与"永远说好"分不开）；再改掉假 `bw` 的
//! `revisionDate`，联网比对从"没变"翻成"上游变过"。
//!
//! ## 这台机器上跑它需要什么
//!
//! * app 起得来（`just test-e2e` 自己会起）；
//! * 假 `bw` 放在 `managed` 那一轴的落点里（`<数据目录>/bitwarden/bw-<版本>/`）——
//!   本机没有真实 vault（`docs/bitwarden.md` §8 记着这条），而"命令怎么拼、形状怎么解析、
//!   导入之后能不能真连"这几件事不需要 vault 就能钉住；
//! * 私钥是**现生成**的 ed25519（`akasha_ssh::testing::key_pair`），服务端认它的公钥 ——
//!   于是"哪把钥匙连上的"有据可查，而不是"反正连上了"。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use akasha_ssh::testing::{ServerOptions, start};
use serde_json::{Value, json};
use support::{
    CLOSE_TIMEOUT, PromptScript, USER, answer_prompts, click, connect_and_prepare, observed,
    open_bitwarden_panel, open_vault, payload, text_of, type_line, unlock, wait_connected, wait_js,
};
use victauri_test::VictauriClient;

/// 假 `bw` 自报的版本。**与目录名一致**（`installed_versions` 按目录名挑最新版本）。
const FAKE_VERSION: &str = "2026.8.0";
/// 假 `bw` 认得的那条主密码。
const RIGHT_PASSWORD: &str = "e2e-right-password";
/// 库里那把钥匙的名字 = 上游条目名 = `IdentityFile` 的文件名（三处必须逐字符相同）。
const KEY_NAME: &str = "akasha-e2e-bw-key";
/// 主机池那一行的名字（配置里的 `Host` 模式）。
const HOST_NAME: &str = "akasha-e2e-bw-host";
/// 上游那条**不是** SSH key 的条目（用来证明"其余条目一条都不进"）。
const LOGIN_CIPHER: &str = "11111111-1111-1111-1111-111111111111";
/// SSH key 那一条的 id。
const KEY_CIPHER: &str = "22222222-2222-2222-2222-222222222222";
/// 上游报的 `revisionDate`（"上游变过"那一档就是把它改掉）。
const REVISION: &str = "2026-09-02T03:04:05.000Z";
/// 改过之后的那个值。
const REVISION_LATER: &str = "2027-01-01T00:00:00.000Z";

/// 假 `bw`：只实现本集成用到的那几条命令，状态放在 `BITWARDENCLI_APPDATA_DIR` 里。
///
/// `list` 那一段用**引号包住的 heredoc**（`<<'JSON'`）：整份 JSON 原样进 stdout，
/// 里面的 `$` / 反引号一个都不会被 shell 解释。
fn fake_bw_script(private_pem: &str, fingerprint: &str, revision: &str) -> String {
    let item = json!([
        {
            "id": LOGIN_CIPHER,
            "type": 1,
            "name": "e2e login",
            "login": { "username": "me", "password": "e2e-not-a-key" },
            "revisionDate": revision,
        },
        {
            "id": KEY_CIPHER,
            "type": 5,
            "name": KEY_NAME,
            "revisionDate": revision,
            "sshKey": {
                "privateKey": private_pem,
                "publicKey": "ssh-ed25519 AAAA e2e",
                "fingerprint": fingerprint,
            },
        },
    ])
    .to_string();

    format!(
        r#"#!/bin/sh
here=$(dirname "$0")
appdir=${{BITWARDENCLI_APPDATA_DIR:-$here}}
mkdir -p "$appdir" 2>/dev/null
logged="$appdir/e2e-logged-in"
unlocked="$appdir/e2e-unlocked"
case "$1" in
  --version) printf '2026.8.0\n' ;;
  --help) printf 'Usage: bw [options] [command]\n\nCommands:\n  login [options] [email]  Log into a user account.\n  status                   Show vault status.\n' ;;
  status)
    if [ -f "$logged" ]; then
      if [ -f "$unlocked" ]; then st=unlocked; else st=locked; fi
      printf '{{"serverUrl":null,"lastSync":null,"userEmail":"e2e@example.com","userId":"00000000-0000-0000-0000-000000000000","status":"%s"}}\n' "$st"
    else
      printf '{{"serverUrl":null,"lastSync":null,"status":"unauthenticated"}}\n'
    fi ;;
  config) if [ "$2" = "server" ] && [ -z "$3" ]; then printf 'https://vault.bitwarden.com'; fi ;;
  login)
    if [ "$BW_PASSWORD" = "e2e-right-password" ]; then
      : > "$logged"; : > "$unlocked"; printf 'e2e-session-key\n'
    else
      printf 'Username or password is incorrect. Try again.\n'; exit 1
    fi ;;
  unlock) : > "$unlocked"; printf 'e2e-session-key\n' ;;
  lock) rm -f "$unlocked"; printf 'Vault locked.\n' ;;
  logout) rm -f "$logged" "$unlocked"; printf 'You have logged out.\n' ;;
  sync) printf 'Syncing complete.\n' ;;
  list)
    if [ ! -f "$logged" ]; then printf 'You are not logged in.\n' >&2; exit 1; fi
    if [ ! -f "$unlocked" ]; then printf 'Vault is locked.\n' >&2; exit 1; fi
    cat <<'JSON'
{item}
JSON
    ;;
  *) printf 'unknown command\n' ;;
esac
"#
    )
}

/// 把假 `bw` 放进 `managed` 那一轴的落点（`<数据目录>/bitwarden/bw-<版本>/`）。
fn install_fake_bw(data_dir: &Path, script: &str) -> PathBuf {
    let dir = data_dir
        .join("bitwarden")
        .join(format!("bw-{FAKE_VERSION}"));
    fs::create_dir_all(&dir).expect("建版本目录");
    let path = dir.join(if cfg!(windows) { "bw.exe" } else { "bw" });
    fs::write(&path, script).expect("写假 bw");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("给可执行位");
    }
    path
}

/// 每次跑之前把上一轮留下的那两行清掉。
///
/// 不这么做的话，"同名不动"这条默认行为会让报告里出现 `exists` ——
/// 判据的意思就变了（那正是 `ssh_config_import` 用 `forget` 的同一个理由）。
/// 顺序不能反：主机那行可能引用着钥匙，先删钥匙会被外键拦住。
fn forget_previous(path: &Path) {
    let conn = open_vault(path);
    for host in akasha_store::pools::hosts::hosts(&conn).unwrap() {
        if host.name == HOST_NAME {
            akasha_store::pools::hosts::delete_host(&conn, host.id).unwrap();
        }
    }
    if let Some(id) = akasha_store::pools::keys::find_by_name(&conn, KEY_NAME).unwrap() {
        akasha_store::pools::keys::delete_key(&conn, id).unwrap();
    }
}

/// 库里那把钥匙的行 id（`vault_hosts` 的 `keyId` 要对上它）。
fn key_row_id(path: &Path) -> Option<i64> {
    akasha_store::pools::keys::find_by_name(&open_vault(path), KEY_NAME).unwrap()
}

async fn invoke(client: &mut VictauriClient, command: &str, args: Value) -> Value {
    client
        .invoke_command(command, Some(args))
        .await
        .unwrap_or_else(|err| panic!("`{command}` 调不通 —— 它登记进 bindings.rs 了吗？{err:?}"))
}

#[tokio::test]
async fn imported_ssh_key_lands_in_the_pool_and_really_connects() {
    if support::skip_unless_e2e() {
        return;
    }
    let Some((mut client, _fixture, vault)) = connect_and_prepare().await else {
        return;
    };
    let Some(data_dir) = vault.parent().map(Path::to_path_buf) else {
        panic!("库没有父目录");
    };

    // ── 1. 现生成一对 ed25519；服务端只认它的公钥 ──────────────────────────
    let (private_pem, fingerprint) = akasha_ssh::testing::key_pair();
    let server = start(ServerOptions {
        accepted_keys: vec![fingerprint.clone()],
        ..ServerOptions::default()
    })
    .await;
    eprintln!(
        "测试服务端 127.0.0.1:{}（主机密钥 {}）；客户端那把一次性钥匙的指纹是 {}",
        server.addr.port(),
        server.fingerprint,
        fingerprint
    );

    // ── 2. 摆好 CLI：managed 那一轴 + 一份假 bw（`list` 交出一份真的私钥）──
    invoke(
        &mut client,
        "bw_cli_settings",
        json!({ "binary": "host", "appdata": "host" }),
    )
    .await;
    // ⚠️ 清掉上一次运行留下的东西：这个数据目录被**所有** E2E 目标共用。
    let _ = fs::remove_dir_all(data_dir.join("bitwarden"));
    forget_previous(&vault);
    unlock(&mut client).await;
    install_fake_bw(
        &data_dir,
        &fake_bw_script(&private_pem, &fingerprint, REVISION),
    );
    let settings = invoke(
        &mut client,
        "bw_cli_settings",
        json!({ "binary": "managed", "appdata": "managed" }),
    )
    .await;
    assert_eq!(settings["cli"]["version"].as_str(), Some(FAKE_VERSION));

    // ── 3. 面板：还没登录就导入 → 说"先登录"，而且**一行都没写** ────────────
    // ⚠️ 用 `support::open_bitwarden_panel` 而不是直接点：上一个目标（`bitwarden_login`）
    //    很可能**把面板留在开着**，而那个按钮是开关 —— 直接点会把它关掉，症状是
    //    "面板上有导入按钮"这条判据等到超时（全量跑时就是这么红的）。
    open_bitwarden_panel(&mut client).await;
    wait_js(
        &mut client,
        "!!document.querySelector('[data-bw-import]')",
        10_000,
        "面板上有导入按钮",
    )
    .await;
    click(&mut client, "[data-bw-import]", "还没登录就点导入").await;
    wait_js(
        &mut client,
        "!!document.querySelector('[data-bw-import-failure]')",
        20_000,
        "面板上给出了那句话",
    )
    .await;
    let refused = text_of(&mut client, "[data-bw-import-failure]").await;
    eprintln!("没登录就导入：{refused}");
    assert!(
        refused.contains("还没有登录"),
        "要说清下一步是做哪件事（拿到 {refused:?}）"
    );
    assert_eq!(key_row_id(&vault), None, "失败的导入不许留下半行");

    // ── 4. 登录（拿到 session key）→ 再点导入 → 报告 ───────────────────────
    let logged = invoke(
        &mut client,
        "bw_login",
        json!({ "email": "e2e@example.com", "password": RIGHT_PASSWORD, "method": null, "code": null }),
    )
    .await;
    assert_eq!(logged["hasSession"], json!(true));
    assert_eq!(logged["status"]["state"].as_str(), Some("unlocked"));

    click(&mut client, "[data-bw-import]", "导入 SSH 密钥").await;
    wait_js(
        &mut client,
        "!!document.querySelector('[data-bw-import-report]')",
        20_000,
        "导入报告出现在面板上",
    )
    .await;

    let report = text_of(&mut client, "[data-bw-import-report]").await;
    eprintln!("面板上的导入报告：{report}");
    assert!(
        report.contains("上游给了 2 条，其中 SSH 密钥 1 条") && report.contains("新增 1"),
        "报告要能说清「看了几条、其中几条是 SSH key、进了几条」：{report}"
    );
    assert!(
        report.contains(KEY_NAME) && report.contains(&fingerprint),
        "报告要带名字与上游给的指纹（拿到 {report:?}）"
    );
    assert!(
        !report.contains("e2e-not-a-key"),
        "登录条目那一侧的东西一个字都不该进报告：{report}"
    );

    let key_id = key_row_id(&vault).expect("池里该有那把钥匙了");
    eprintln!("池里那一行钥匙：id={key_id}");

    // ── 5. 把主机指到它：`IdentityFile` 的 basename 与钥匙名相同 ───────────
    let config = data_dir.join("e2e-bw-config.conf");
    fs::write(
        &config,
        format!(
            "Host {HOST_NAME}\n    HostName 127.0.0.1\n    Port {}\n    User {USER}\n    \
             IdentityFile ~/.ssh/{KEY_NAME}\n",
            server.addr.port()
        ),
    )
    .expect("写 ssh_config");
    let imported = invoke(
        &mut client,
        "import_ssh_config",
        json!({ "path": config.to_string_lossy(), "overwrite": true }),
    )
    .await;
    eprintln!("导入 ssh_config：{imported}");

    let hosts = invoke(&mut client, "vault_hosts", json!({})).await;
    let row = hosts
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["name"] == HOST_NAME))
        .unwrap_or_else(|| panic!("池里没有 {HOST_NAME}：{hosts}"));
    assert_eq!(
        row["keyId"],
        json!(key_id),
        "`IdentityFile` 的 basename 与钥匙名相同，就该接上那一行：{row}"
    );

    // ── 6. 用**导入进来的那把钥匙**开一个会话 ──────────────────────────────
    click(&mut client, ".tab-new-ssh", "打开主机选择器").await;
    wait_js(
        &mut client,
        &format!(
            "!!document.querySelector('.host-picker-item[data-host-id=\"{}\"')",
            row["id"].as_u64().unwrap()
        ),
        10_000,
        "选择器里有那一行",
    )
    .await;
    click(
        &mut client,
        &format!(
            ".host-picker-item[data-host-id=\"{}\"]",
            row["id"].as_u64().unwrap()
        ),
        "选那台用导入钥匙的主机",
    )
    .await;

    // ⚠️ `tabs: 2`：app 起来时就有一个本地终端标签页，SSH 会话是第二个
    //（`is_connected` 数的是**全部**标签页，不是新开的那一个）。
    let asked = answer_prompts(
        &mut client,
        PromptScript {
            tabs: 2,
            jump_port: server.addr.port(),
            jump_password: "",
            target_password: "",
            jump_fingerprint: &server.fingerprint,
            target_fingerprint: &server.fingerprint,
        },
    )
    .await;
    eprintln!("提示问答（按发生顺序）：{asked:?}");
    wait_connected(&mut client, 2, "用导入的钥匙开出来的 SSH 会话").await;

    // "连上"与"用的是这把钥匙"是两件事：服务端只认它的公钥，所以我们再核一次指纹。
    let seen = observed(&server);
    assert!(
        seen.methods.iter().any(|method| method == "publickey"),
        "服务端该看到一次 publickey 认证：{:?}",
        seen.methods
    );
    assert!(
        seen.offered_keys
            .iter()
            .any(|offered| offered == &fingerprint),
        "服务端看到的指纹就是导入时存下来的那一个（看到 {:?}，期望 {fingerprint}）",
        seen.offered_keys
    );
    eprintln!(
        "服务端：methods={:?} offered_keys={:?}",
        seen.methods, seen.offered_keys
    );

    // ── 7. 字节能双向流（会话真的能用） ────────────────────────────────────
    type_line(&mut client, "echo via-bitwarden-key\n").await;
    wait_js(
        &mut client,
        "window.__akashaTerminal.screenText(200).includes('via-bitwarden-key')",
        support::CONNECT_TIMEOUT_MS,
        "服务端的回声出现在了终端上",
    )
    .await;
    let shell = String::from_utf8_lossy(&observed(&server).shell_data).to_string();
    assert!(shell.contains("echo via-bitwarden-key"), "收到 {shell:?}");

    // ── 8. 离线自检（plan 0904）：先一致，再把库里那把私钥换掉 —— 必须报不一致 ──
    click(&mut client, "[data-bw-cache-verify]", "校验缓存（离线）").await;
    wait_js(
        &mut client,
        "!!document.querySelector('[data-bw-cache-report]')",
        20_000,
        "自检的读数出现在面板上",
    )
    .await;
    let verified = text_of(&mut client, "[data-bw-cache-report]").await;
    eprintln!("面板上的缓存自检：{verified}");
    assert!(
        verified.contains("完好") && verified.contains(&fingerprint),
        "刚导入的那一份该报「完好」，且带上算出来的指纹：{verified}"
    );

    // ⚠️ 诱饵：把库里那把私钥换成**另一把**（等价于"缓存被换过 / 坏了"）。
    //    少了这一步，"自检"与"永远说好"分不开。
    {
        let (other_pem, _) = akasha_ssh::testing::key_pair();
        let conn = open_vault(&vault);
        let id = akasha_store::pools::keys::find_by_name(&conn, KEY_NAME)
            .unwrap()
            .expect("库里该有那把钥匙");
        let mut other = akasha_store::pools::keys::PrivateKey::new(other_pem.into_bytes()).unwrap();
        akasha_store::pools::keys::set_private_key(&conn, id, &mut other).unwrap();
    }
    click(
        &mut client,
        "[data-bw-cache-verify]",
        "再校验一次（私钥已被换掉）",
    )
    .await;
    wait_js(
        &mut client,
        "document.querySelector('[data-bw-cache-report]')?.textContent?.includes('对不上') ?? false",
        20_000,
        "自检报出不一致",
    )
    .await;
    eprintln!(
        "换掉私钥之后的自检：{}",
        text_of(&mut client, "[data-bw-cache-report]").await
    );

    // ── 9. 联网比对（plan 0904）：先"没变"，改掉上游的 revisionDate 之后报"变过" ──
    click(&mut client, "[data-bw-cache-check]", "检查上游有没有变").await;
    wait_js(
        &mut client,
        "!!document.querySelector('[data-bw-refresh-report]')",
        20_000,
        "比对的读数出现在面板上",
    )
    .await;
    let same = text_of(&mut client, "[data-bw-refresh-report]").await;
    eprintln!("面板上的上游比对：{same}");
    assert!(
        same.contains("没变"),
        "`revisionDate` 一个字没动，该报「没变」：{same}"
    );

    // 换一份假 `bw`：上游那一条的 `revisionDate` 变了（等价于用户在 Bitwarden 那边改了它）。
    install_fake_bw(
        &data_dir,
        &fake_bw_script(&private_pem, &fingerprint, REVISION_LATER),
    );
    click(
        &mut client,
        "[data-bw-cache-check]",
        "上游改过之后再比对一次",
    )
    .await;
    wait_js(
        &mut client,
        "document.querySelector('[data-bw-refresh-report]')?.textContent?.includes('上游变过') ?? false",
        20_000,
        "比对报出「上游变过」",
    )
    .await;
    eprintln!(
        "上游改过之后的比对：{}",
        text_of(&mut client, "[data-bw-refresh-report]").await
    );

    // ── 10. 收尾：关标签页（会话零残留由别的目标盯），把轴放回默认 ──────────
    let closed = client
        .eval_js(
            "(() => { const tabs = document.querySelectorAll('.tab.is-active .tab-close'); \
             if (!tabs.length) return false; tabs[0].click(); return true; })()",
        )
        .await
        .unwrap();
    assert!(payload(&closed).as_bool().unwrap_or(false), "关不掉标签页");
    wait_js(
        &mut client,
        "document.querySelectorAll('.tab').length === 1",
        CLOSE_TIMEOUT.as_millis() as u64,
        "标签页关掉了",
    )
    .await;

    invoke(
        &mut client,
        "bw_cli_settings",
        json!({ "binary": "host", "appdata": "host" }),
    )
    .await;
    let _ = fs::remove_file(&config);
    let _ = fs::remove_dir_all(data_dir.join("bitwarden"));
}
