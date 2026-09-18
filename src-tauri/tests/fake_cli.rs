//! 用**一个假的 `bw`**（一段 shell 脚本）把调用这一层钉住。
//!
//! 为什么需要它：真实的 `bw` 要一个 vault 才能走完登录（`docs/bitwarden.md` §8 如实记着
//! 这条待实测）。但"我们把命令拼成什么样、机密走的是环境还是 argv、超时会不会杀进程"
//! 这几件事**不需要 vault**，而它们恰恰是这一层最容易写错的地方 ——
//! 换个 vault 也测不出"口令被塞进了 argv"。
//!
//! ⚠️ 只在 Unix 上跑：假 `bw` 是一段 `sh` 脚本（Windows 上的等价物是 `cmd` 批处理，
//! 而那条路 CI 上还没有对应目标 —— 见 `docs/STATUS.md`）。

#![cfg(unix)]
#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::path::{Path, PathBuf};
use std::time::Duration;

use akasha_lib::bw::{AppData, BinarySource, BwError, Cli, Located, Settings, Timeouts};

/// 一个临时目录（本仓库没有 `tempfile` 依赖，与其余 crate 同一条口径）。
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("akasha-bw-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 造一个假 `bw`。它把自己收到的 **argv** 与 **`$BW_PASSWORD`** 各写一份到同目录，
/// 于是"机密走了哪条路"与"命令长什么样"都成了可断言的事实。
fn fake_bw(dir: &Path) -> Located {
    let program = dir.join("bw");
    let script = r#"#!/bin/sh
here=$(dirname "$0")
printf '%s\n' "$@" > "$here/args.txt"
printf '%s' "${BW_PASSWORD-}" > "$here/password.txt"
if [ -f "$here/fail" ]; then
  cat "$here/fail"
  exit 1
fi
if [ -f "$here/slow" ]; then
  sleep 5
fi
case "$1" in
  --version) printf '2026.8.0\n' ;;
  --help) printf 'Usage: bw [options] [command]\n\nCommands:\n  login [options] [email] [password]  Log into a user account.\n  status                              Show server, last sync, user information, and vault status.\n' ;;
  status) printf '{"serverUrl":"https://vault.example.com","status":"locked"}\n' ;;
  unlock) printf 'sess-%s\n' "$BW_PASSWORD" ;;
  lock) printf 'locked\n' ;;
  sync) printf 'Syncing complete.\n' ;;
  list) printf '[{"id":"c-1","type":1,"name":"a login","login":{"password":"hunter2"}},{"id":"c-2","type":5,"name":"k","revisionDate":"rev","sshKey":{"privateKey":"pem","publicKey":"pub","fingerprint":"SHA256:x"}}]\n' ;;
  *) printf 'unknown command\n' ;;
esac
"#;
    std::fs::write(&program, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    Located {
        program,
        appdata: None,
        settings: Settings {
            binary: BinarySource::Managed,
            appdata: AppData::Host,
        },
    }
}

/// 快期限：超时那条判据不该让测试等 20 秒。
fn quick() -> Timeouts {
    Timeouts {
        local: Duration::from_millis(400),
        status: Duration::from_millis(400),
        network: Duration::from_millis(400),
    }
}

#[test]
fn version_and_variant_come_from_the_real_command_lines() {
    let dir = scratch("version");
    let cli = Cli::new(&fake_bw(&dir));

    assert_eq!(cli.version().unwrap(), "2026.8.0");
    assert_eq!(
        cli.variant().unwrap(),
        akasha_lib::bw::Variant::Oss,
        "假 help 里没有 device-approval 那一行"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn status_is_read_through_the_real_parse_path() {
    let dir = scratch("status");
    let cli = Cli::new(&fake_bw(&dir));

    let status = cli.status().unwrap();
    assert_eq!(status.state, akasha_lib::bw::State::Locked);
    assert_eq!(
        status.server_url.as_deref(),
        Some("https://vault.example.com")
    );

    // 命令确实是 `status --raw`（少了 `--raw` 拿到的是给人看的那一版，形状完全不同）。
    let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert_eq!(args, "status\n--raw\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **本文件最重要的一条**：口令走环境、不走 argv（ADR-0007 D8）。
#[test]
fn the_master_password_travels_in_the_environment_not_in_argv() {
    let dir = scratch("password");
    let cli = Cli::new(&fake_bw(&dir));
    let password = "correct horse battery staple";

    let mut session = cli.unlock(password).unwrap();

    // ① 子进程真的收到了它（会话 key 就是这个脚本按收到的口令拼出来的）；
    assert_eq!(
        &*session.expose().unwrap(),
        format!("sess-{password}").as_bytes(),
        "`--passwordenv` 要把口令交给子进程"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("password.txt")).unwrap(),
        password
    );

    // ② 而 argv 里**没有**它 —— 这一条才是"argv 对同机进程可见"那条约束的落点。
    let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert!(
        !args.contains(password),
        "口令不许出现在 argv 里（`ps` 就能读到）：{args}"
    );
    assert!(
        args.contains("--passwordenv"),
        "走的是官方那条非交互入口：{args}"
    );
    assert!(
        args.contains("--nointeraction"),
        "不许让子进程等人回答：{args}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// session key 同样走环境（`BW_SESSION`），不走 `--session`。
#[test]
fn the_session_key_travels_in_the_environment_too() {
    let dir = scratch("session");
    let cli = Cli::new(&fake_bw(&dir));
    let mut session = cli.unlock("pw").unwrap();

    cli.sync(&mut session).unwrap();

    let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert_eq!(
        args, "sync\n",
        "带 session 的命令只多一个环境变量，不多参数"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failures_are_classified_from_what_the_cli_prints() {
    for (printed, expected) in [
        ("You are not logged in.\n", "not logged in"),
        ("Vault is locked.\n", "锁着"),
        (
            "Unable to fetch ServerConfig from https://api.bitwarden.com FetchError: ETIMEDOUT\n",
            "网络失败",
        ),
        (
            "Vault is corrupted, please re-import\n",
            "Vault is corrupted",
        ),
    ] {
        let dir = scratch("fail");
        let cli = Cli::new(&fake_bw(&dir));
        std::fs::write(dir.join("fail"), printed).unwrap();

        let err = cli.status().unwrap_err();
        match expected {
            "not logged in" => assert!(matches!(err, BwError::NotLoggedIn), "{err:?}"),
            "锁着" => assert!(matches!(err, BwError::Locked), "{err:?}"),
            "网络失败" => assert!(matches!(err, BwError::Network { .. }), "{err:?}"),
            other => match err {
                BwError::CommandFailed { message, .. } => {
                    assert!(message.contains(other), "{message}")
                }
                other => panic!("{other:?}"),
            },
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// `list items`：session key 走环境、命令带 `--raw`，而**解析只在纯函数那一层**
/// （所以这里拿到的是原始字节，一个登录口令也没被我们留下）。
#[test]
fn listing_items_goes_through_the_same_session_environment() {
    let dir = scratch("items");
    let cli = Cli::new(&fake_bw(&dir));
    let mut session = cli.unlock("pw").unwrap();

    let raw = cli.items(&mut session).unwrap();
    let found = akasha_lib::bw::items::parse(&raw).unwrap();
    assert_eq!(found.total, 2);
    assert_eq!(
        found
            .ssh_keys
            .iter()
            .map(|k| k.name.as_str())
            .collect::<Vec<_>>(),
        ["k"],
        "登录那一条不许进"
    );

    let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert_eq!(
        args, "list\nitems\n--raw\n--nointeraction\n",
        "少了 --raw 拿到的是给人看的那一版；少了 --nointeraction 会让它等人回答"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 超时：**杀掉并收干净**，不许把一个还在跑的 `bw` 留在那里。
#[test]
fn a_hung_cli_is_killed_and_reported_as_a_timeout() {
    let dir = scratch("slow");
    let cli = Cli::new(&fake_bw(&dir)).with_timeouts(quick());
    std::fs::write(dir.join("slow"), b"").unwrap();

    let err = cli.version().unwrap_err();
    assert!(matches!(err, BwError::Timeout { .. }), "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 路径不对（不存在 / 不是一个能跑的文件）要给可读原因，而不是 panic。
#[test]
fn a_program_that_cannot_be_started_is_reported_with_its_path() {
    let dir = scratch("missing");
    let located = Located {
        program: dir.join("not-here"),
        appdata: None,
        settings: Settings::default(),
    };
    let err = Cli::new(&located).version().unwrap_err();
    match err {
        BwError::NotRunnable { path, .. } => assert!(path.ends_with("not-here"), "{path}"),
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
