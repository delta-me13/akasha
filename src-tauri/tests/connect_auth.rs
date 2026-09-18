//! plan 0502 的判据（**crate 层**）：连接、认证顺序、凭据缓存。
//!
//! 判据原文 =「同主机开三个 Session **只问一次**凭据」。这里的三个 "Session" 就是
//! 三个 [`SshTransport`]（= 三个 Session 各自的载体，ADR-0003 D5 一实体一连接）。
//!
//! ⚠️ 测试目标是**进程内的 russh 服务端**（见 `support`）：它证明的是客户端这条链，
//! 不是与 OpenSSH 的互操作 —— 那条记在 `docs/STATUS.md` 的待验证里。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "ssh_support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use akasha_pty::{ExitStatus, TerminalSize, Transport};
use akasha_lib::ssh::{
    CredentialCache, HostKeyVerifier, KeyCandidate, PinnedHostKey, SshAuth, SshError, SshTarget,
    SshTransport,
};
use support::{
    CountingProvider, ServerOptions, connect_options, connect_options_with, read_until, round_trip,
    start,
};

/// 每个用例自己建一个 runtime（ADR-0003 D2：库**不**自建 runtime，由调用方给 `Handle`）。
///
/// 用 `#[test]` 而不是 `#[tokio::test]`：同步门面内部走 `Handle::block_on`，
/// 而它在 runtime 线程上会 panic —— 测试线程必须**不在** runtime 上下文里，
/// 这正好也是 app 的真实形态（命令层不是 runtime 线程）。
/// `Result::expect_err` 要求 `Ok` 一侧 `Debug`，而 `SshTransport` 刻意没有 `Debug`
/// （它握着一条连接与两个队列，打出来没有意义）。所以这里写一个不要求 `Debug` 的版本。
fn expect_connect_error(
    runtime: &tokio::runtime::Runtime,
    options: akasha_lib::ssh::SshConnect,
) -> SshError {
    match SshTransport::connect(runtime.handle(), options) {
        Ok(_) => panic!("这个连接本该失败，却连上了"),
        Err(err) => err,
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// 判据：**三个连接、一次提问**。
#[test]
fn three_sessions_ask_for_one_credential() {
    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions::password("hunter2")));

    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("hunter2"));

    let mut sessions = Vec::new();
    for index in 0..3 {
        let options = connect_options(
            &server,
            "cyrene",
            SshAuth::agent_only(),
            Arc::clone(&cache),
            Arc::clone(&provider),
        );
        let mut transport = SshTransport::connect(runtime.handle(), options)
            .unwrap_or_else(|err| panic!("第 {} 个连接失败：{err}", index + 1));
        // 每个连接都要**真的能用**，不能只是"握手成功"：只用计数会放过
        // "缓存的凭据其实没被拿去认证"这种情形。
        round_trip(&mut transport, &format!("ping-{index}"));
        sessions.push(transport);
    }

    assert_eq!(provider.calls(), 1, "同一台主机开三个连接只该问一次凭据");
    assert_eq!(
        cache.len(),
        1,
        "缓存里应当正好一条（key = 主机 + 用户 + 方式）"
    );
    assert_eq!(
        server.shared.observed.lock().unwrap().passwords,
        vec!["hunter2".to_owned(); 3],
        "服务端每一次都应当收到同一句口令"
    );

    for mut transport in sessions {
        transport.shutdown().expect("收尾失败");
    }
}

/// 键里必须**带用户**：不然同一台主机的两个用户会共用一句口令。
#[test]
fn another_user_gets_asked_separately() {
    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions::password("hunter2")));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("hunter2"));

    for user in ["alice", "bob"] {
        let options = connect_options(
            &server,
            user,
            SshAuth::agent_only(),
            Arc::clone(&cache),
            Arc::clone(&provider),
        );
        SshTransport::connect(runtime.handle(), options)
            .unwrap_or_else(|err| panic!("{user} 的连接失败：{err}"));
    }

    assert_eq!(provider.calls(), 2, "两个用户各问一次");
    assert_eq!(cache.len(), 2);
}

/// D8 失效条件 ②：**服务端拒绝就删掉那一条**，否则会拿错口令反复重试。
#[test]
fn a_rejected_password_is_forgotten_and_asked_again() {
    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions::password("hunter2")));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("stale"));

    // 第一次：口令是错的，认证失败（链上只有 password 一档）。
    let options = connect_options(
        &server,
        "cyrene",
        SshAuth::agent_only(),
        Arc::clone(&cache),
        Arc::clone(&provider),
    );
    let err = expect_connect_error(&runtime, options);
    assert!(
        matches!(err, SshError::AuthenticationFailed { .. }),
        "错误类型应当是认证失败，实际是 {err:?}"
    );
    assert_eq!(provider.calls(), 1);
    assert!(
        cache.is_empty(),
        "被服务端拒绝的口令**不许**留在缓存里（拿错口令反复重试会锁账号）"
    );

    // 第二次：换一句对的，必须**重新问**（因为这中间没有缓存可用）。
    server.shared.set_password(Some("hunter2"));
    provider.set_answer("hunter2");
    let options = connect_options(
        &server,
        "cyrene",
        SshAuth::agent_only(),
        Arc::clone(&cache),
        Arc::clone(&provider),
    );
    SshTransport::connect(runtime.handle(), options).expect("换对口令之后应当连上");
    assert_eq!(provider.calls(), 2);
}

/// D8 失效条件 ③：显式忘记。
#[test]
fn forgetting_and_clearing_force_a_new_question() {
    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions::password("hunter2")));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("hunter2"));

    let connect = || {
        let options = connect_options(
            &server,
            "cyrene",
            SshAuth::agent_only(),
            Arc::clone(&cache),
            Arc::clone(&provider),
        );
        SshTransport::connect(runtime.handle(), options).expect("连接失败")
    };

    connect();
    assert_eq!(provider.calls(), 1);

    // 用户点了"忘记这台主机"。
    cache.clear();
    connect();
    assert_eq!(provider.calls(), 2, "清空之后必须重新问");

    // 逐条忘记同一个效果（这里只剩一条）。
    assert!(cache.forget(&akasha_lib::ssh::CacheKey::new(
        SshTarget::new("127.0.0.1", server.addr.port(), "cyrene"),
        akasha_lib::ssh::CredentialKind::LoginPassword,
    )));
    assert!(cache.is_empty());
}

/// D8 失效条件 ②的密钥版：**私钥口令解不开就忘掉**，不拿错口令反复试同一把钥匙。
#[test]
fn a_bad_key_passphrase_is_forgotten() {
    let runtime = runtime();
    // 服务端不认任何公钥，也不认口令 —— 这把钥匙注定连不上，但"口令错"这件事
    // 必须在**本地**就被发现（解密就失败），而不是靠服务端拒绝。
    let server = runtime.block_on(start(ServerOptions::default()));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("wrong-passphrase"));

    // 这把钥匙的口令是 `akasha-test`，而 provider 会给一句错的。
    let key = KeyCandidate::new("pool-key#1", encrypted_key("akasha-test")).unwrap();
    let options = connect_options(
        &server,
        "cyrene",
        SshAuth::keys(vec![key]),
        Arc::clone(&cache),
        Arc::clone(&provider),
    );
    let err = expect_connect_error(&runtime, options);
    assert!(matches!(err, SshError::AuthenticationFailed { .. }));

    assert_eq!(
        provider.kinds(),
        vec!["key-passphrase".to_owned(), "password".to_owned()],
        "先问私钥口令（解不开），再落到口令那档"
    );
    assert!(
        provider.calls() > 1,
        "两次提问：私钥口令 1 次 + 登录口令 1 次"
    );
    assert!(
        cache.is_empty(),
        "解不开的口令不许留在缓存里（留着就是每次都失败）"
    );
}

/// D7：**顺序**。密钥池那条路先走，服务端不认时才落到口令；服务端看到的顺序就是判据。
#[test]
fn the_wire_shows_publickey_before_password() {
    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions::password("hunter2")));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("hunter2"));

    let key = KeyCandidate::new("pool-key#1", plain_key()).unwrap();
    let options = connect_options(
        &server,
        "cyrene",
        SshAuth::keys(vec![key]),
        Arc::clone(&cache),
        Arc::clone(&provider),
    );
    SshTransport::connect(runtime.handle(), options).expect("口令那档应当接上");

    let observed = server.shared.observed.lock().unwrap().clone();
    assert_eq!(
        observed.methods,
        vec!["publickey".to_owned(), "password".to_owned()],
        "顺序必须是公钥先、口令后（服务端看到的就是这一串）"
    );
    assert_eq!(
        observed.offered_keys.len(),
        1,
        "只报了一把钥匙（密钥池里就一把）"
    );
}

/// D7：keyboard-interactive 排在 password **之前**（2FA 主机不能被当成"没方法可用"）。
#[test]
fn the_wire_shows_keyboard_interactive_before_password() {
    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions {
        keyboard_code: Some("123456".to_owned()),
        ..ServerOptions::default()
    }));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("123456"));

    let key = KeyCandidate::new("pool-key#1", plain_key()).unwrap();
    let options = connect_options(
        &server,
        "cyrene",
        SshAuth::keys(vec![key]),
        Arc::clone(&cache),
        Arc::clone(&provider),
    );
    SshTransport::connect(runtime.handle(), options).expect("验证码那档应当接上");

    let observed = server.shared.observed.lock().unwrap().clone();
    assert_eq!(
        observed.methods,
        vec!["publickey".to_owned(), "keyboard-interactive".to_owned()],
        "顺序必须是公钥 → keyboard-interactive（口令那档根本没轮到）"
    );
    assert!(observed.passwords.is_empty(), "不该走 password");
}

/// agent 不可用（`SSH_AUTH_SOCK` 指向一个不存在的文件）时**静默落到下一档**，不是错误。
#[test]
fn an_unavailable_agent_falls_through() {
    // 指向一个不存在的 socket。**不改环境变量**：那在 Rust 2024 里是 `unsafe`，
    // 而本仓库只允许 `akasha-store` 出现 `unsafe`（AGENTS.md §3.4）——
    // 于是"agent 在哪"这件事本身就是输入的一部分。

    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions::password("hunter2")));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("hunter2"));

    let options = connect_options(
        &server,
        "cyrene",
        SshAuth::agent_at("/nonexistent/akasha-test-agent"),
        Arc::clone(&cache),
        Arc::clone(&provider),
    );
    SshTransport::connect(runtime.handle(), options).expect("agent 不在不该挡住口令认证");
    assert_eq!(provider.calls(), 1, "落到 password 那档，正常问一次");

    let observed = server.shared.observed.lock().unwrap().clone();
    assert_eq!(
        observed.methods,
        vec!["password".to_owned()],
        "agent 不可用时**不该**在线协议上留下 publickey 的痕迹"
    );
}

/// D11：主机密钥钉错时**拒绝连接**，而且错误里带着**指纹**（用户要拿它去核对）。
#[test]
fn a_wrong_host_key_is_rejected_with_its_fingerprint() {
    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions::password("hunter2")));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("hunter2"));

    let wrong: Arc<dyn HostKeyVerifier> = Arc::new(PinnedHostKey::new("SHA256:not-the-real-one"));
    let options = connect_options_with(
        &server,
        "cyrene",
        SshAuth::agent_only(),
        cache,
        provider,
        wrong,
    );
    let err = expect_connect_error(&runtime, options);
    match err {
        SshError::HostKeyRejected { fingerprint } => assert_eq!(fingerprint, server.fingerprint),
        other => panic!("错误类型应当是 HostKeyRejected，实际是 {other:?}"),
    }
    assert!(
        server.shared.observed.lock().unwrap().methods.is_empty(),
        "主机密钥没过就不该开始认证"
    );
}

/// D4：SSH 终端有尺寸（`window_change`）、有结局（`exit-status`）、**没有本地 pid**。
#[test]
fn the_shell_round_trips_bytes_resize_and_exit_status() {
    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions {
        password: Some("hunter2".to_owned()),
        exit_status: 7,
        ..ServerOptions::default()
    }));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("hunter2"));

    let options = connect_options(&server, "cyrene", SshAuth::agent_only(), cache, provider);
    let mut transport = SshTransport::connect(runtime.handle(), options).expect("连接失败");

    // 能力位：SSH 终端这样答（隧道是另一个载体，见阶段 6）。
    let capabilities = transport.capabilities();
    assert!(capabilities.resize && capabilities.exit_status);
    assert_eq!(
        transport.session_leader(),
        None,
        "SSH 没有本地进程要收（ADR-0003 D4；答错的表现是看门狗去收一个不存在的 pid）"
    );

    // 读端只能取一次（`Transport::output_stream` 的契约），所以这一条用例只取一次，
    // 把"字节往返"与"尺寸到达对端"两件事**读在同一段输出里**。
    let reader = transport.output_stream().expect("读端只能取一次");
    transport.write(b"hello over ssh\n").expect("写入失败");
    transport
        .resize(TerminalSize::new(120, 40))
        .expect("resize 不该失败");
    // 两条命令走同一条队列，先入先出；服务端把收到的尺寸回声回来，
    // 于是"读到 window:120x40"同时证明了两件事都真的到了对端。
    let seen = read_until(reader, b"window:120x40", Duration::from_secs(5));
    assert!(
        seen.windows(15).any(|window| window == b"hello over ssh\n"),
        "回声里应当有写入的字节（拿到 {} 字节：{:?}）",
        seen.len(),
        String::from_utf8_lossy(&seen)
    );
    assert!(
        seen.windows(13).any(|window| window == b"window:120x40"),
        "服务端应当收到 120x40 的 window_change（拿到 {:?}）",
        String::from_utf8_lossy(&seen)
    );

    let status = transport.shutdown().expect("收尾失败");
    assert_eq!(
        status,
        Some(ExitStatus::Code(7)),
        "远端 shell 的退出码必须被带回来（D4 的 exit_status）"
    );
    // 幂等：收尾两次返回同一个结局，而不是报错。
    let again = transport.shutdown().expect("第二次收尾不该报错");
    assert_eq!(again, Some(ExitStatus::Code(7)));

    let observed = server.shared.observed.lock().unwrap().clone();
    assert_eq!(observed.pty_requests, 1, "开通道时请求了一次 pty");
    assert!(
        observed.window_changes >= 1,
        "resize 应当变成 window_change"
    );
    assert!(observed.shell_data.starts_with(b"hello over ssh\n"));
}

/// 同步门面**不能在 tokio 上下文里调用** —— 但也不许 panic（调用方是 app 的命令层）。
#[test]
fn connecting_from_inside_a_runtime_is_an_error_not_a_panic() {
    let runtime = runtime();
    let server = runtime.block_on(start(ServerOptions::password("hunter2")));
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("hunter2"));

    let handle = runtime.handle().clone();
    let err = handle
        .clone()
        .block_on(async move {
            let options =
                connect_options(&server, "cyrene", SshAuth::agent_only(), cache, provider);
            SshTransport::connect(&handle, options).map(|_| ())
        })
        .expect_err("在 runtime 里调同步门面必须报错");
    assert!(
        matches!(err, SshError::BlockingInsideRuntime),
        "实际是 {err:?}"
    );
}

/// 现生成一把 **未加密**的 ed25519 私钥。
///
/// 为什么不把私钥当 fixture 提交进仓库：测试用的私钥当然谁都拿得到，但"仓库里躺着一把
/// 私钥"会让每一个看到它的人多花一次时间判断"这是不是真东西"。现生成零成本。
fn plain_key() -> Vec<u8> {
    use russh::keys::{Algorithm, PrivateKey};
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("生成私钥失败");
    key.to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .expect("序列化私钥失败")
        .as_bytes()
        .to_vec()
}

/// 现生成一把**用口令加密**的 ed25519 私钥。
fn encrypted_key(passphrase: &str) -> Vec<u8> {
    use russh::keys::{Algorithm, PrivateKey};
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("生成私钥失败");
    let encrypted = key
        .encrypt(&mut rand::rng(), passphrase)
        .expect("加密私钥失败");
    encrypted
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .expect("序列化私钥失败")
        .as_bytes()
        .to_vec()
}
