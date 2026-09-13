//! `AGENTS.md` §7 的第一条（**真实路径走通**）在 plan 0407 上的落点：真 app 上
//! 解锁 → 读一次四套池 → 锁定，并且**从外面**看那个进程的内存有没有还回来。
//!
//! ## 为什么判据是"读 app 进程的 `VmLck`"
//!
//! ROADMAP 的验收就是这么写的：「解锁 → 读一次池 → **锁定之后进程里不留机密**
//! （判据：`VmLck` 回落到解锁前的水平）」。而这一段**只能在真 app 上**验证：
//!
//! * 库层那侧的证据在 `akasha-store/tests/unlock_lifecycle.rs`（整个进程内存扫一遍）；
//! * 这里要验的是**另一件事**：app 里那条命令真的把两样东西一起丢掉了 ——
//!   而 `/proc/<pid>/status` 里的 `VmLck` 是**测试进程自己**看到的外部事实，
//!   不是 app 报给我们的数字（自报的数字证明不了自报的数字）。
//!
//! ## 数据从哪来（为什么这一条要自己造库）
//!
//! app 现在**没有**写四套池的 IPC 命令（那是后面的 plan），所以用例没法"通过 app
//! 自己的命令"造数据。造法是：**直接调 `akasha-store` 的库函数**，落到 `vault_status`
//! 报出来的那个路径上 —— 与 E2E 写 `config.json` 是同一种做法（都是"这台机器上的
//! 外部状态"）。⚠️ 造之前先确认那个文件**不是用户的真库**：是的话**显式跳过**，
//! 绝不拿测试口令去动它。
//!
//! ## 收尾
//!
//! 自己造的库自己删（`Fixture` 的 `Drop`，断言失败也走得到）。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::fs;
use std::path::{Path, PathBuf};

use akasha_store::{Passphrase, create, forwards, hosts, keys, serial};
use serde_json::{Value, json};
use victauri_test::VictauriClient;

/// 这条用例自己用的口令。它**只**用于"这个库是不是我们自己造的"这个判断。
const PASSPHRASE: &str = "e2e-unlock-lifecycle-passphrase";

fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("Skipping: set VICTAURI_E2E=1 with your Tauri dev server running");
        return true;
    }
    false
}

/// 自己造的库：**析构时删掉**（断言失败也走得到 —— 测试里 panic 是 unwind）。
struct Fixture {
    path: PathBuf,
    remove_on_drop: bool,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// 往库里放**每套池各一行**：解锁返回的行数因此不可能是"全 0 的巧合"。
fn seed(path: &Path) {
    let mut passphrase = Passphrase::new(PASSPHRASE.as_bytes().to_vec()).unwrap();
    let conn = create(path, &mut passphrase).unwrap();

    let key_id = {
        let mut private =
            keys::PrivateKey::new(b"-----BEGIN OPENSSH PRIVATE KEY-----\nseed\n".to_vec()).unwrap();
        keys::insert_key(
            &conn,
            &keys::NewKey {
                name: "seed-key".into(),
                public_key: "ssh-ed25519 AAAASEED".into(),
                comment: None,
            },
            &mut private,
        )
        .unwrap()
    };
    hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "seed-host".into(),
            host: "seed.example".into(),
            port: 2222,
            user: "seed".into(),
            auth: hosts::Auth::PublicKey,
            key_id: Some(key_id),
            jump_id: None,
        },
    )
    .unwrap();
    serial::insert_serial(
        &conn,
        &serial::NewSerial {
            name: "seed-serial".into(),
            port: "ttyUSB0".into(),
            baud: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: serial::Parity::None,
            flow: serial::Flow::None,
        },
    )
    .unwrap();
    forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: "seed-forward".into(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".into(),
            bind_port: 8080,
            target_host: Some("seed.example".into()),
            target_port: Some(80),
            host_id: hosts::hosts(&conn).unwrap()[0].id,
            autostart: false,
        },
    )
    .unwrap();
}

/// 这个库能不能用**我们自己的口令**打开 —— 也就是"它是不是我们造的那个"。
fn is_ours(path: &Path) -> bool {
    let mut passphrase = Passphrase::new(PASSPHRASE.as_bytes().to_vec()).unwrap();
    akasha_store::open(path, &mut passphrase).is_ok()
}

/// app 进程的 pid：victauri 的发现目录是 `<temp>/victauri/<pid>/`，
/// 而那个 `port` 文件里就是它监听的端口（问题 #40）。
fn app_pid(port: u16) -> Option<u32> {
    let root = std::env::temp_dir().join("victauri");
    for entry in fs::read_dir(root).ok()? {
        let entry = entry.ok()?;
        let reported = fs::read_to_string(entry.path().join("port")).ok()?;
        if reported.trim().parse::<u16>() == Ok(port) {
            return entry.file_name().to_str()?.parse::<u32>().ok();
        }
    }
    None
}

/// app 进程**已经 `mlock` 住**的内存量（kB）—— 外部事实，不是它自报的。
fn locked_kb(pid: u32) -> Option<u64> {
    let text = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("VmLck:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
}

#[tokio::test]
async fn unlocking_reads_the_pools_and_locking_gives_the_locked_memory_back() {
    if skip_unless_e2e() {
        return;
    }

    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— 用 `just test-e2e`（它会自己起 app）");

    // ── 1. 库在哪、现在是什么状态 ────────────────────────────────────────────
    let status = client
        .invoke_command("vault_status", None)
        .await
        .expect("vault_status 调不通 —— 它登记进 bindings.rs 了吗？");
    eprintln!("vault_status = {status}");
    let path = PathBuf::from(
        status
            .pointer("/path")
            .and_then(Value::as_str)
            .expect("返回值里必须有 path"),
    );
    assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("akasha.db"));
    assert_eq!(
        status.pointer("/unlocked").and_then(Value::as_bool),
        Some(false),
        "app 刚起起来就该是锁着的（没有自动解锁、也没有从配置文件读口令那条路）"
    );

    // ── 2. 造一个我们自己认识的库（除非那里已经有别人的真库）────────────────
    let state = status
        .pointer("/state")
        .and_then(Value::as_str)
        .expect("返回值里必须有 state");
    let ours_to_remove = match state {
        "missing" | "empty" => {
            seed(&path);
            true
        }
        "present" if is_ours(&path) => true, // 上一次跑留下的
        other => {
            eprintln!(
                "跳过：{} 上已经有一个库（state={other}），而且它**不是**用这条用例的口令建的 —— \
                 那是用户自己的数据，测试不许碰它",
                path.display()
            );
            return;
        }
    };
    let _fixture = Fixture {
        path: path.clone(),
        remove_on_drop: ours_to_remove,
    };

    // ── 3. 解锁，并且真的读到四套池里那四行 ─────────────────────────────────
    let pid =
        app_pid(client.port()).expect("找不到 app 的 discovery 目录 —— 拿不到 pid 就量不了 VmLck");
    let locked_before = locked_kb(pid).expect("读不到 app 的 /proc/<pid>/status");
    eprintln!("解锁前：pid={pid} VmLck={locked_before} kB");

    let contents = client
        .invoke_command("vault_unlock", Some(json!({ "passphrase": PASSPHRASE })))
        .await
        .expect("vault_unlock 调不通（口令对、库也在）");
    eprintln!("vault_unlock = {contents}");
    assert_eq!(
        contents,
        json!({ "keys": 1, "hosts": 1, "serials": 1, "forwards": 1 }),
        "解锁返回的四套池行数与我们造的对不上 —— 库真的被解开、被读通了吗？"
    );
    let locked_during = locked_kb(pid).expect("读不到 app 的 /proc/<pid>/status");
    eprintln!("解锁中：VmLck={locked_during} kB");

    // ── 4. 状态跟着走（前后端一致这条判据的对象）─────────────────────────────
    let unlocked_status = client.invoke_command("vault_status", None).await.unwrap();
    assert_eq!(
        unlocked_status
            .pointer("/unlocked")
            .and_then(Value::as_bool),
        Some(true),
        "解锁之后 vault_status 还说没解锁：{unlocked_status}"
    );

    // ── 5. 锁定，内存还回去 ────────────────────────────────────────────────
    let locked = client
        .invoke_command("vault_lock", None)
        .await
        .expect("vault_lock 调不通");
    assert_eq!(
        locked,
        json!(true),
        "刚才明明解锁着，vault_lock 却说没锁到东西"
    );
    let locked_after = locked_kb(pid).expect("读不到 app 的 /proc/<pid>/status");
    eprintln!("锁定后：VmLck={locked_after} kB（解锁前 {locked_before}，解锁中 {locked_during}）");

    assert!(
        locked_during > locked_before,
        "解锁期间 app 的 VmLck 没有涨({locked_before} → {locked_during} kB)—— \
         那条命令真的解锁了吗？（正对照：不涨的话下面那条断言什么也不证明）"
    );
    assert_eq!(
        locked_after, locked_before,
        "锁定之后 app 的 VmLck 没有回落到解锁前（{locked_before} → {locked_after} kB）"
    );

    let locked_status = client.invoke_command("vault_status", None).await.unwrap();
    assert_eq!(
        locked_status.pointer("/unlocked").and_then(Value::as_bool),
        Some(false),
        "锁定之后 vault_status 还说解锁着：{locked_status}"
    );

    // ── 6. 锁是真的：错误口令解不开，而且它**什么都不改** ────────────────────
    let wrong = client
        .invoke_command(
            "vault_unlock",
            Some(json!({ "passphrase": "definitely-not-the-passphrase" })),
        )
        .await;
    assert!(
        wrong.is_err(),
        "错误口令居然解锁成功了 —— 那「锁着」这个状态就没有意义：{wrong:?}"
    );
    eprintln!("错误口令被拒：{wrong:?}");
    let still_locked = client.invoke_command("vault_status", None).await.unwrap();
    assert_eq!(
        still_locked.pointer("/unlocked").and_then(Value::as_bool),
        Some(false),
        "错误口令失败之后库应当是**仍然锁着**的，而不是半开着：{still_locked}"
    );

    // ── 7. 锁上之后还能再解开（而且这次是"开已有的库"那条路）────────────────
    let again = client
        .invoke_command("vault_unlock", Some(json!({ "passphrase": PASSPHRASE })))
        .await
        .expect("锁定之后再用正确口令解锁应当成功");
    assert_eq!(again, contents, "重新解锁读到的内容应当一模一样");
    assert_eq!(
        client
            .invoke_command("vault_lock", None)
            .await
            .expect("再锁一次"),
        json!(true)
    );
}
