//! plan 0405 的验收：把 **bin 所在文件夹整个搬走**，数据还在、还能用
//! （[`docs/portable.md`](../../docs/portable.md) §5 的五步）。
//!
//! ## 为什么这条用例**自己起 app**，而不是复用 `just test-e2e` 那个
//!
//! 被测的判据是"数据目录从 **bin 所在目录** 推导"（P2）。`just test-e2e` 那个 app 跑在
//! `target/debug/` 里 —— 那是构建目录，搬不动它（搬了就没法再跑别的用例）。
//! 所以这里把可执行文件**复制**进临时布局（`<tmp>/akasha-portable-<pid>-<tag>/`），
//! 在 A 起一次、`mv` 成 B、在 B 再起一次：被测的是"bin 在哪"。
//!
//! ⚠️ **代价**：这一段不能与别的 akasha 同时跑 —— 单实例（plan 0304）会让第二份
//! **自己退掉**，于是"app 起不来"看起来像可搬迁性坏了。配方 `portable` 会先查一遍并说清
//! 原因；这里则在 app 提前退出时把同一个原因打进 panic 消息。
//!
//! ⚠️ 两个用例都起自己的 app，**必须串行**（`--test-threads=1`，配方里已经这么调）：
//! 并行等于自己跟自己抢单实例。
//!
//! ## 判据分两半
//!
//! * **app 侧**：probe 报的数据目录（`lifecycle` 里的 `close_behavior` 证明它读的是
//!   **跟着搬走**的 `config.json`）+ `vault_unlock` 读回四套池的行数；
//! * **库侧**：搬完用同一口令打开，四行**内容**逐项一致 —— 只验行数不够，
//!   "静默丢内容"才是这条判据要防的东西。
//!
//! ## 收尾
//!
//! 自己的临时布局自己删（`Layout` 的 `Drop`，断言失败也走得到）；app 进程也收掉。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use akasha_lib::config::EXIT_NOT_WRITABLE;
use akasha_lib::store::{Passphrase, forwards, hosts, keys, serial};
use serde_json::{Value, json};
use victauri_test::VictauriClient;

/// 这条用例自己用的口令。库是自己造的，所以口令也自己定。
const PASSPHRASE: &str = "portable-e2e-passphrase";

/// 便携数据目录名（`config::PORTABLE_DIR` 的约定）、库文件名（ADR-0002 D1）。
const DATA_DIR: &str = "akasha-data";
const VAULT: &str = "akasha.db";

fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("跳过: 未设置 VICTAURI_E2E=1（该变量由 just portable 或 just test-e2e 设置）");
        return true;
    }
    false
}

/// 临时布局：`<root>/<bin>`（被测二进制的一份**复制**）+ `<root>/akasha-data/`。
struct Layout {
    root: PathBuf,
}

impl Layout {
    /// 带便携数据目录的布局（`portable.md` §4 第 1 条：目录本身就是"要便携"的标记）。
    fn new(tag: &str) -> Self {
        let this = Self::bare(tag);
        fs::create_dir_all(this.data()).expect("建便携数据目录");
        this
    }

    /// **不带**数据目录的布局：app 该走 OS 数据目录那条路（§4 第 2 条）。
    fn without_data_dir(tag: &str) -> Self {
        Self::bare(tag)
    }

    fn bare(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("akasha-portable-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("建临时布局");
        let source = PathBuf::from(env!("CARGO_BIN_EXE_akasha"));
        fs::copy(
            &source,
            root.join(source.file_name().expect("可执行文件有名字")),
        )
        .expect("复制被测二进制");
        Self { root }
    }

    fn bin(&self) -> PathBuf {
        let name = PathBuf::from(env!("CARGO_BIN_EXE_akasha"));
        self.root.join(name.file_name().expect("可执行文件有名字"))
    }

    fn data(&self) -> PathBuf {
        self.root.join(DATA_DIR)
    }

    fn vault(&self) -> PathBuf {
        self.data().join(VAULT)
    }

    fn config(&self) -> PathBuf {
        self.data().join("config.json")
    }

    /// 搬家：整个文件夹换一个路径（`portable.md` §5 第 3 步 = `mv`）。
    ///
    /// 同一个文件系统上就是一次 `rename`，和用户把文件夹拖到别处是同一回事；
    /// 旧的 `Layout` 析构时删一个已经不存在的路径，什么也不做。
    fn moved_to(&self, tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("akasha-portable-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::rename(&self.root, &root).expect("搬家");
        Self { root }
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // 不可写那条用例把数据目录设成了 500 —— 不先松开，里面的东西删不掉。
            let _ = fs::set_permissions(self.data(), fs::Permissions::from_mode(0o700));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// 一份**从布局里起**的 app。
struct App {
    child: Child,
    port: u16,
    token: Option<String>,
    log: PathBuf,
}

impl App {
    /// 起 app，等它把 Victauri 的发现目录（端口 + 令牌）写出来。
    ///
    /// ⚠️ 这里的 `sleep` 是**有界的就绪等待**：判据是"那个目录出现了"，等不到就报错 ——
    /// 不是 `AGENTS.md` §7 禁止的"用固定 `sleep` 猜异步完成"。
    fn start(layout: &Layout) -> Self {
        let log = layout.root.join("app.log");
        let mut child = Command::new(layout.bin())
            .stdout(Stdio::from(File::create(&log).expect("建日志文件")))
            // stderr 丢掉：那是 GTK / Mesa / dconf 的噪音，而且不读走会把它堵死。
            .stderr(Stdio::null())
            .spawn()
            .expect("起不来 app —— cargo 会先把这个 bin 构建出来");
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(90);

        while Instant::now() < deadline {
            if let Some((port, token)) = discovery(pid) {
                return Self {
                    child,
                    port,
                    token,
                    log,
                };
            }
            if let Some(status) = child.try_wait().expect("try_wait") {
                let log = read(&log);
                reap(&mut child);
                panic!(
                    "app 还没就绪就退出了（{status}）。是不是已经有一个 akasha 在跑？\
                     单实例（plan 0304）会让第二份自己退掉。\n日志：\n{log}"
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let log = read(&log);
        reap(&mut child);
        panic!("app 在 90s 内没有写出发现目录。\n日志：\n{log}");
    }

    async fn client(&self) -> VictauriClient {
        VictauriClient::connect_with_token(self.port, self.token.as_deref())
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "连不上这一份 app 的 Victauri（{err}）—— 日志：\n{}",
                    read(&self.log)
                )
            })
    }

    /// 等 `vault_status` **真的能应答**，返回它第一次成功的结果。
    ///
    /// ⚠️ 又一个"就绪 ≠ 能连上"：`invoke_command` 走的是 **webview 里的 JS bridge**，
    /// 而它是最后才好的一个（窗口建出来 → 前端从 Vite 加载完 → bridge 注册）。
    /// 所以这里等的是"命令真的答了"，不是睡够时间。
    async fn status_ready(&self, client: &mut VictauriClient) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match client.invoke_command("vault_status", None).await {
                Ok(value) => return value,
                Err(err) => {
                    if Instant::now() > deadline {
                        panic!(
                            "`vault_status` 在 60s 内一直调不通（最后一次：{err}）—— 日志：\n{}",
                            read(&self.log)
                        );
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }

    /// **完全退出**（`portable.md` §5 第 2 步）。
    ///
    /// 用 `kill()`（SIGKILL）而不是"优雅退出"是**故意**的：搬家之后数据还得在，
    /// 这件事不该依赖退出钩子跑完了没有。留一条"只有优雅退出才成立"的绿，
    /// 等于把真正的可搬迁性问题留到用户拔 U 盘那天。
    fn stop(self) {}

    /// 收掉这个进程。**`Drop` 也走它**：断言失败/超时 panic 时同样不许留一个 app 在跑 ——
    /// 留下来的那一份会让**下一次**运行被单实例挡掉（`plan 0304`），而症状看起来
    /// 会像"可搬迁性坏了"。
    fn kill(&mut self) {
        reap(&mut self.child);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.kill();
    }
}

/// 收掉一个子进程。
///
/// ⚠️ `Child` 的 `Drop` **只关句柄，不杀进程** —— 起坏了的那条路上不显式收，
/// 就会留下一个 app（`AGENTS.md` §3.3 的"零残留"）。
fn reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// 这个 pid 的 Victauri 端口与令牌（发现目录是 `<temp>/victauri/<pid>/`）。
fn discovery(pid: u32) -> Option<(u16, Option<String>)> {
    let dir = std::env::temp_dir().join("victauri").join(pid.to_string());
    let port = fs::read_to_string(dir.join("port"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let token = fs::read_to_string(dir.join("token"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    Some((port, token))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| format!("<读不到日志：{err}>"))
}

/// 起一次，等它**自己退出**（这条判据盼的就是它退出）。
fn run_until_exit(layout: &Layout) -> (ExitStatus, String) {
    let log = layout.root.join("app.log");
    let mut child = Command::new(layout.bin())
        .stdout(Stdio::from(File::create(&log).expect("建日志文件")))
        .stderr(Stdio::null())
        .spawn()
        .expect("起不来 app");
    let deadline = Instant::now() + Duration::from_secs(60);

    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return (status, read(&log));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "app 在 60s 内没有退出 —— 它没把\"便携目录不可写\"当回事。\n日志：\n{}",
                read(&log)
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// `lifecycle` probe 报的 `close_behavior` —— **不过 webview bridge**，所以前端出问题时
/// 这条断言照样成立（它观察的是 app 读了哪个数据目录里的 `config.json`）。
///
/// ⚠️ **发现目录出现 ≠ app 就绪**：Victauri 的插件 setup 比 app 自己的 `.setup()` 早，
/// 所以在刚连上的那一刻 probe 还是 `{"initialized": false}`（那次 `record()` 在 `.setup()` 里，
/// 而我们那条拒绝启动的检查就在它前面）。这条判据要的恰恰是"setup 跑完了"，
/// 所以这里**等那个字段自己出现**（有界），不把中间态当成答案、也不 sleep 猜。
async fn close_behavior(client: &mut VictauriClient) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = client
            .call_tool("app_state", json!({ "probe": "lifecycle" }))
            .await
            .expect("读 lifecycle probe");
        if let Some(value) = snapshot.pointer("/close_behavior").and_then(Value::as_str) {
            return value.to_string();
        }
        if Instant::now() > deadline {
            panic!(
                "lifecycle probe 在 30s 内一直没登记 —— app 卡在启动路径上了？最后一次：{snapshot}"
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn vault_path(status: &Value) -> PathBuf {
    PathBuf::from(
        status
            .pointer("/path")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("vault_status 里没有 path：{status}")),
    )
}

fn vault_state(status: &Value) -> String {
    status
        .pointer("/state")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("vault_status 里没有 state：{status}"))
        .to_string()
}

/// 往库里放"原件"：四套池各一行，名字各不相同（断言时才分得清谁是谁）。
///
/// 库是 **app 自己建的**（上面那次 `vault_unlock` 走的就是"没有库 → 建一个"那条路），
/// 所以这里只能 `open`（同一个口令）—— 这也顺带证明 app 建出来的库认得我们的口令。
///
/// app 现在**没有**写四套池的 IPC 命令（后面的 plan），所以造数据只能直接调库 ——
/// 与 `vault_unlock.rs` 同一种做法。
fn seed(path: &Path) {
    let mut passphrase = Passphrase::new(PASSPHRASE.as_bytes().to_vec()).unwrap();
    let conn = akasha_lib::store::open(path, &mut passphrase).unwrap();

    let key_id = {
        let mut private =
            keys::PrivateKey::new(b"-----BEGIN OPENSSH PRIVATE KEY-----\nportable\n".to_vec())
                .unwrap();
        keys::insert_key(
            &conn,
            &keys::NewKey {
                name: "portable-key".into(),
                public_key: "ssh-ed25519 AAAAPORTABLE".into(),
                comment: None,
            },
            &mut private,
        )
        .unwrap()
    };
    let host_id = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "portable-host".into(),
            host: "portable.example".into(),
            port: 2222,
            user: "portable".into(),
            auth: hosts::Auth::PublicKey,
            key_id: Some(key_id),
            jump_id: None,
        },
    )
    .unwrap();
    serial::insert_serial(
        &conn,
        &serial::NewSerial {
            name: "portable-serial".into(),
            port: "ttyUSB7".into(),
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
            name: "portable-forward".into(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".into(),
            bind_port: 18080,
            target_host: Some("portable.example".into()),
            target_port: Some(80),
            host_id,
            autostart: false,
        },
    )
    .unwrap();
}

/// 库侧：四行的**内容**（不只是行数）还在吗 —— 搬完之后的第二份证据。
fn assert_contents(path: &Path) {
    let mut passphrase = Passphrase::new(PASSPHRASE.as_bytes().to_vec()).unwrap();
    let conn = akasha_lib::store::open(path, &mut passphrase)
        .unwrap_or_else(|err| panic!("搬完之后用同一个口令打不开 {}：{err}", path.display()));

    assert_eq!(keys::keys(&conn).unwrap()[0].name, "portable-key");
    let host = &hosts::hosts(&conn).unwrap()[0];
    assert_eq!(host.name, "portable-host");
    assert_eq!(host.host, "portable.example");
    assert_eq!(serial::serials(&conn).unwrap()[0].name, "portable-serial");
    assert_eq!(
        forwards::forwards(&conn).unwrap()[0].name,
        "portable-forward"
    );
}

/// `portable.md` §5 的五步：A 起 → 完全退出 → 搬走 → B 起 → **数据还在且能用**。
#[tokio::test]
async fn data_survives_the_move() {
    if skip_unless_e2e() {
        return;
    }

    let a = Layout::new("a");
    // `config.json` 也放进数据目录：它是"数据目录挑对了"的第二个证据 ——
    // 读到 `exit` 才说明 app 读的就是**跟着搬走**的那一份，而不是在新位置上取了默认值。
    fs::write(a.config(), "{\"close_behavior\":\"exit\"}\n").expect("写配置");

    // ── 1. A：app 自己把库**建在** bin 同目录的便携目录里 ────────────────────
    let app = App::start(&a);
    let mut client = app.client().await;

    assert_eq!(
        close_behavior(&mut client).await,
        "exit",
        "app 读到的不是 A 里那份 config.json —— 它挑的是别的数据目录？"
    );

    let status = app.status_ready(&mut client).await;
    assert_eq!(vault_path(&status), a.vault(), "库不在 A 的便携目录里");
    assert_eq!(vault_state(&status), "missing");

    let created = client
        .invoke_command("vault_unlock", Some(json!({ "passphrase": PASSPHRASE })))
        .await
        .expect("vault_unlock（在便携目录里建库）");
    assert_eq!(
        created,
        json!({ "keys": 0, "hosts": 0, "serials": 0, "forwards": 0 }),
        "刚建的库当然是空的"
    );
    assert_eq!(
        client
            .invoke_command("vault_lock", None)
            .await
            .expect("vault_lock"),
        json!(true)
    );
    app.stop();

    // ── 2. 完全退出之后，把"原件"放进这个库 ─────────────────────────────────
    seed(&a.vault());
    assert_contents(&a.vault());

    // ── 3. 搬家：整个文件夹换个路径（第 3 步）────────────────────────────────
    let b = a.moved_to("b");
    assert!(!a.data().exists(), "搬完就不该还在原地");
    assert!(b.vault().is_file(), "库该跟着文件夹一起走");

    // ── 4. B：app 认的是**新位置**，而且原有的东西都还在 ─────────────────────
    let app = App::start(&b);
    let mut client = app.client().await;

    assert_eq!(
        close_behavior(&mut client).await,
        "exit",
        "搬走之后 app 读不到那份 config.json 了 —— 它在别的地方找了数据目录"
    );

    let status = app.status_ready(&mut client).await;
    assert_eq!(vault_path(&status), b.vault(), "app 还在认搬走之前那个位置");
    assert_eq!(vault_state(&status), "present");

    let contents = client
        .invoke_command("vault_unlock", Some(json!({ "passphrase": PASSPHRASE })))
        .await
        .expect("vault_unlock（搬完之后）");
    assert_eq!(
        contents,
        json!({ "keys": 1, "hosts": 1, "serials": 1, "forwards": 1 }),
        "搬家之后四套池对不上 —— 这正是判据「原有主机/密钥/规则都在」的机器可查形态"
    );
    app.stop();

    // ── 5. 库侧再逐项对一遍（行数对不代表内容对）────────────────────────────
    assert_contents(&b.vault());
}

/// 便携目录**不可写** → 启动即报错（`portable.md` §4 第 3 条），而不是静默退回 OS 目录。
///
/// 这条用例在 plan 0405 之前是**红的**：app 照常启动，日志里只有一句
/// `config not found`（把"写不进去"说成了"没有配置文件"）—— 用户以为数据在移动盘上。
#[tokio::test]
async fn an_unwritable_portable_dir_refuses_to_start() {
    if skip_unless_e2e() {
        return;
    }

    let layout = Layout::new("ro");
    make_read_only(&layout.data());

    // **正对照**：这个环境真的拦得住写吗？拦不住（以 root 跑、或文件系统不理会 mode 位）
    // 就显式跳过并写明原因 —— 不把"没验过"说成"验过了"（`AGENTS.md` §7）。
    if fs::File::create(layout.data().join("probe")).is_ok() {
        eprintln!("跳过: 运行环境不限制对数据目录的写入，造不出「便携目录不可写」这一前提");
        return;
    }

    let (status, log) = run_until_exit(&layout);
    assert_eq!(
        status.code(),
        Some(EXIT_NOT_WRITABLE),
        "便携目录不可写时必须拒绝启动（退出码 {EXIT_NOT_WRITABLE}），而不是静默换个目录。日志：\n{log}"
    );
    assert!(
        log.contains("portable data dir not writable"),
        "日志里必须有那条 error（不然用户看不出为什么起不来）：\n{log}"
    );
}

/// 另一半：**没有**便携目录时不许拒绝（`portable.md` §4 第 2 条：退回 OS 数据目录）。
///
/// 少了这一条，上面那条判据分不清"检查在工作"与"检查把谁都拒了"。
#[tokio::test]
async fn without_a_portable_dir_it_starts_anyway() {
    if skip_unless_e2e() {
        return;
    }

    let layout = Layout::without_data_dir("os");
    // 起来了（发现目录写出来了）就是通过 —— 这里不碰 IPC，它的判据只是"没被拒"。
    let app = App::start(&layout);
    app.stop();
}

#[cfg(unix)]
fn make_read_only(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o500)).expect("chmod 500");
}

#[cfg(not(unix))]
fn make_read_only(_dir: &Path) {
    // Windows 的只读目录语义不同（要 ACL / 挂载），本文件不造那种 fixture ——
    // 上面那条用例的**正对照**会因此跳过并说明原因。
}
