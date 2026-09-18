//! plan 0503 的判据（**crate 层**）：三态判定 + "确认过的记住、变了的拒绝"。
//!
//! 判据原文（ROADMAP）=「host key 变化时**拒绝连接并提示**（不静默接受）」+
//!「未知 host key 由用户**确认后进缓存**」。
//!
//! 两个注入点在测试里都是**桩**：`FakeCache` 替库、`CountingPrompt` 替用户
//! （与 0502 的 `CountingProvider` 同一个套路）。所以这一份验的是**策略本身**；
//! 库那一侧（表、`UNIQUE`、"不许改写"）由 `akasha-store` 的 `known_hosts_roundtrip` 守。
//!
//! ⚠️ 服务端仍是**进程内**的 `russh`（见 `support`）：它证明客户端这条链，
//! 不是与 OpenSSH 的互操作。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "ssh_support/mod.rs"]
mod support;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use akasha_lib::ssh::{
    CredentialCache, HostKey, HostKeyCache, HostKeyPrompt, HostKeyVerifier, KnownHostsVerifier,
    RecordedHostKey, RecordedIn, SshAuth, SshError, SshTarget, SshTransport,
};
use rand::rng;
use russh::keys::{Algorithm, HashAlg, PrivateKey};
use support::{CountingProvider, Running, ServerOptions, connect_options_with, start};

/// 替库的内存缓存。键是 `(host, port, key_type)` —— 与库里的 `UNIQUE` 同一件事。
#[derive(Default)]
struct FakeCache {
    rows: Mutex<HashMap<(String, u16, String), RecordedHostKey>>,
    /// "记住"发生了**几次**（比"表里有几行"更有信息量：幂等写入与没写是两回事）。
    writes: AtomicUsize,
}

impl FakeCache {
    fn len(&self) -> usize {
        self.rows.lock().unwrap().len()
    }

    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }

    /// 预置一条记录。**不经过** `remember`：测"记录对不上"时，那条记录不是用户刚确认的，
    /// 而是上次连接时留下的 —— 用同一个入口造它会把两条路混成一条。
    fn seed(&self, host: &str, port: u16, key_type: &str, blob: Vec<u8>, fingerprint: &str) {
        self.rows.lock().unwrap().insert(
            (host.to_owned(), port, key_type.to_owned()),
            RecordedHostKey {
                blob,
                fingerprint: fingerprint.to_owned(),
            },
        );
    }
}

impl HostKeyCache for FakeCache {
    fn recorded(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
    ) -> Result<Option<RecordedHostKey>, SshError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(&(host.to_owned(), port, key_type.to_owned()))
            .cloned())
    }

    fn remember(&self, host: &str, port: u16, key: &HostKey) -> Result<(), SshError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.seed(
            host,
            port,
            key.algorithm(),
            key.blob().to_vec(),
            key.fingerprint(),
        );
        Ok(())
    }
}

/// 替用户的提问桩：记次数 + 一个可调的答案。
struct CountingPrompt {
    calls: AtomicUsize,
    answer: bool,
}

impl CountingPrompt {
    fn new(answer: bool) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            answer,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl HostKeyPrompt for CountingPrompt {
    fn confirm(&self, _target: &SshTarget, _key: &HostKey) -> Result<bool, SshError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.answer)
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn password_server(runtime: &tokio::runtime::Runtime) -> Running {
    runtime.block_on(start(ServerOptions::password("hunter2")))
}

/// 连一次。`expect_err` 要求 `Ok` 一侧有 `Debug`，而 `SshTransport` 刻意没有
/// （它握着一条连接与两个队列，打出来没有意义），所以这里自己分个岔。
fn connect(
    runtime: &tokio::runtime::Runtime,
    server: &Running,
    verifier: Arc<dyn HostKeyVerifier>,
) -> Result<(), SshError> {
    let options = connect_options_with(
        server,
        "cyrene",
        SshAuth::agent_only(),
        Arc::new(CredentialCache::new()),
        Arc::new(CountingProvider::new("hunter2")),
        verifier,
    );
    match SshTransport::connect(runtime.handle(), options) {
        Ok(_) => Ok(()),
        Err(err) => Err(err),
    }
}

/// 库里有**服务端真实那把**的 verifier（等价于"用户确认过"）。
fn confirming(cache: Arc<FakeCache>) -> Arc<dyn HostKeyVerifier> {
    Arc::new(
        KnownHostsVerifier::new(cache)
            .without_user_file()
            .with_prompt(Arc::new(CountingPrompt::new(true))),
    )
}

/// 一把**不在任何服务端上**的密钥：`(OpenSSH 文本, 指纹, blob)`。
///
/// 三种表示各有用处：写用户的文件要文本，断言"记的是哪一把"要指纹，
/// 预置库里的记录要 blob。它们出自同一把密钥 —— 分开生成会让"两个指纹不同"
/// 这条对照断言变成废话。
fn foreign_key() -> (String, String, Vec<u8>) {
    let public = PrivateKey::random(&mut rng(), Algorithm::Ed25519)
        .unwrap()
        .public_key()
        .clone();
    (
        public.to_openssh().unwrap(),
        public.fingerprint(HashAlg::Sha256).to_string(),
        public.to_bytes().unwrap(),
    )
}

/// 一份临时的"用户的 `known_hosts`"（非 22 端口要写成 `[host]:port`）。
fn user_file(name: &str, contents: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/ssh-known-hosts")
        .join(name);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("known_hosts");
    fs::write(&path, contents).unwrap();
    path
}

fn line_for(server: &Running, public_key: &str) -> String {
    format!(
        "[127.0.0.1]:{} {}\n",
        server.addr.port(),
        public_key.trim_end()
    )
}

// ── 1. 未知：**不静默接受** ──────────────────────────────────────────────────

#[test]
fn unknown_host_key_is_not_silently_accepted() {
    let runtime = runtime();
    let server = password_server(&runtime);
    let cache = Arc::new(FakeCache::default());

    // 没人可问（没配 prompt）→ 明确的错误，而不是"算了先连上"。
    let verifier: Arc<dyn HostKeyVerifier> =
        Arc::new(KnownHostsVerifier::new(cache.clone()).without_user_file());
    let err = connect(&runtime, &server, verifier).unwrap_err();

    match &err {
        SshError::HostKeyUnknown {
            host,
            port,
            fingerprint,
        } => {
            assert_eq!(host, "127.0.0.1");
            assert_eq!(*port, server.addr.port());
            assert_eq!(
                fingerprint, &server.fingerprint,
                "错误里必须带上**服务端真实**的指纹（用户要拿它去核对）"
            );
        }
        other => panic!("未知密钥该报 HostKeyUnknown，实际：{other:?}"),
    }
    assert_eq!(cache.len(), 0, "拒绝之后什么都不该被记下来");
    assert_eq!(
        server.shared.observed.lock().unwrap().methods.len(),
        0,
        "主机密钥没通过之前，认证一步都不该开始"
    );
}

// ── 2. 确认之后：记住，且**不再问** ──────────────────────────────────────────

#[test]
fn a_confirmed_host_key_is_remembered_and_never_asked_again() {
    let runtime = runtime();
    let server = password_server(&runtime);
    let cache = Arc::new(FakeCache::default());
    let prompt = Arc::new(CountingPrompt::new(true));

    let verifier = || -> Arc<dyn HostKeyVerifier> {
        Arc::new(
            KnownHostsVerifier::new(cache.clone())
                .without_user_file()
                .with_prompt(prompt.clone()),
        )
    };

    connect(&runtime, &server, verifier()).expect("用户确认之后应当连上");
    assert_eq!(prompt.calls(), 1, "第一次要问");
    assert_eq!(cache.len(), 1, "确认过的密钥要进缓存");
    assert_eq!(cache.writes(), 1);

    // 第二次：同一台主机、同一把密钥 → 不再问。这就是"缓存"这条判据的意义
    // （`scope.md` §2.2：到同一主机开三个标签不该被问三次）。
    connect(&runtime, &server, verifier()).expect("第二次也应当连上");
    assert_eq!(prompt.calls(), 1, "记下来之后不该再问第二次");
}

#[test]
fn a_declined_host_key_is_refused_and_not_recorded() {
    let runtime = runtime();
    let server = password_server(&runtime);
    let cache = Arc::new(FakeCache::default());
    let prompt = Arc::new(CountingPrompt::new(false));

    let verifier: Arc<dyn HostKeyVerifier> = Arc::new(
        KnownHostsVerifier::new(cache.clone())
            .without_user_file()
            .with_prompt(prompt.clone()),
    );
    let err = connect(&runtime, &server, verifier).unwrap_err();

    assert_eq!(prompt.calls(), 1, "问过了");
    assert!(
        matches!(err, SshError::HostKeyRejected { .. }),
        "用户否认 → 拒绝（与「没人可问」分开）：{err:?}"
    );
    assert_eq!(cache.len(), 0, "否认之后更不该记下来");
    assert_eq!(cache.writes(), 0);
}

// ── 3. 变了：拒绝，而且**绝不问**（问就等于把警报降级成一次点击） ────────────

#[test]
fn a_changed_host_key_is_refused_without_asking() {
    let runtime = runtime();
    let server = password_server(&runtime);
    let cache = Arc::new(FakeCache::default());
    let (_, foreign_fingerprint, foreign_blob) = foreign_key();
    cache.seed(
        "127.0.0.1",
        server.addr.port(),
        "ssh-ed25519",
        foreign_blob,
        &foreign_fingerprint,
    );
    let prompt = Arc::new(CountingPrompt::new(true));

    let verifier: Arc<dyn HostKeyVerifier> = Arc::new(
        KnownHostsVerifier::new(cache.clone())
            .without_user_file()
            .with_prompt(prompt.clone()),
    );
    let err = connect(&runtime, &server, verifier).unwrap_err();

    match &err {
        SshError::HostKeyChanged {
            recorded,
            presented,
            recorded_in,
            ..
        } => {
            assert_eq!(recorded, &foreign_fingerprint, "记的是哪一把要说出来");
            assert_eq!(presented, &server.fingerprint, "现在给的是哪一把也要说出来");
            assert_ne!(recorded, presented, "前提：两把确实不同");
            assert_eq!(*recorded_in, RecordedIn::Vault);
        }
        other => panic!("密钥变化该报 HostKeyChanged，实际：{other:?}"),
    }
    assert_eq!(
        prompt.calls(),
        0,
        "**变化不是一次提问**：把警报做成一次可点击的确认，正是中间人攻击要的结果"
    );
    assert_eq!(cache.writes(), 0, "更不该把新的那把记下来（不静默改写）");
}

// ── 4. 用户的 `~/.ssh/known_hosts`：只读，认它，但永不动它 ────────────────────

#[test]
fn a_matching_user_file_entry_is_accepted_and_the_file_is_never_written() {
    let runtime = runtime();
    let server = password_server(&runtime);
    let cache = Arc::new(FakeCache::default());
    let prompt = Arc::new(CountingPrompt::new(true));

    let path = user_file("matching", &line_for(&server, &server.host_key_openssh));
    let before = fs::read(&path).unwrap();

    let verifier: Arc<dyn HostKeyVerifier> = Arc::new(
        KnownHostsVerifier::new(cache.clone())
            .with_user_file(&path)
            .with_prompt(prompt.clone()),
    );
    connect(&runtime, &server, verifier).expect("文件里认得这把密钥，就该连上");

    assert_eq!(prompt.calls(), 0, "文件里有了就不必问用户");
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "用户的 known_hosts **一个字节都不许变**（D11：只读，不改写用户的文件）"
    );
    assert_eq!(
        cache.len(),
        0,
        "文件里认过的密钥不进我们的库：那份文件才是它的家，抄一份只会在用户改它之后变陈旧"
    );
}

#[test]
fn a_changed_user_file_entry_is_refused_and_names_the_line() {
    let runtime = runtime();
    let server = password_server(&runtime);
    let cache = Arc::new(FakeCache::default());
    let prompt = Arc::new(CountingPrompt::new(true));

    let (foreign_openssh, foreign_fingerprint, _) = foreign_key();
    let path = user_file(
        "changed",
        &format!(
            "# 注释行不该影响判定\n{}",
            line_for(&server, &foreign_openssh)
        ),
    );

    let verifier: Arc<dyn HostKeyVerifier> = Arc::new(
        KnownHostsVerifier::new(cache.clone())
            .with_user_file(&path)
            .with_prompt(prompt.clone()),
    );
    let err = connect(&runtime, &server, verifier).unwrap_err();

    match &err {
        SshError::HostKeyChanged {
            recorded,
            presented,
            recorded_in,
            ..
        } => {
            match recorded_in {
                RecordedIn::UserFile { path: named, line } => {
                    assert_eq!(named, &path);
                    assert_eq!(
                        *line,
                        Some(2),
                        "要指出是**哪一行**（注释行占第 1 行）—— 上游给的行号会跳过注释，\
                         所以这个数字是我们自己数出来的"
                    );
                }
                other => panic!("记录在用户的文件里，实际：{other:?}"),
            }
            assert_eq!(recorded, &foreign_fingerprint, "文件里那一把的指纹");
            assert_eq!(presented, &server.fingerprint);
        }
        other => panic!("文件里对不上也该报 HostKeyChanged，实际：{other:?}"),
    }
    assert_eq!(prompt.calls(), 0, "变化不提问");
}

// ── 5. 两个来源打架时谁说话：**我们的库**（用户在这个程序里确认过的那次） ──────

#[test]
fn the_vault_record_wins_over_the_user_file() {
    let runtime = runtime();
    let server = password_server(&runtime);
    let cache = Arc::new(FakeCache::default());

    // 对照组：**空缓存** + 同一份文件 → 会判成"变了"（证明那份文件确实对不上）。
    let (foreign_openssh, _, _) = foreign_key();
    let path = user_file("vault-wins", &line_for(&server, &foreign_openssh));
    let control: Arc<dyn HostKeyVerifier> =
        Arc::new(KnownHostsVerifier::new(Arc::new(FakeCache::default())).with_user_file(&path));
    let err = connect(&runtime, &server, control).unwrap_err();
    assert!(
        matches!(err, SshError::HostKeyChanged { .. }),
        "对照：这份文件对不上服务端：{err:?}"
    );

    // ① 用户在 akasha 里确认过一次（库里有服务端真实那把）。
    connect(&runtime, &server, confirming(cache.clone())).expect("确认后连上");
    assert_eq!(cache.len(), 1);

    // ② 库里有记录 → 连上，不去问文件、也不问用户。
    let quiet = Arc::new(CountingPrompt::new(false)); // 万一真去问，答案是否认 → 会失败
    let verifier: Arc<dyn HostKeyVerifier> = Arc::new(
        KnownHostsVerifier::new(cache.clone())
            .with_user_file(&path)
            .with_prompt(quiet.clone()),
    );
    connect(&runtime, &server, verifier).expect("库里有确认过的记录，就该连上");
    assert_eq!(quiet.calls(), 0, "库先说话，轮不到问用户");
}
