//! `akasha-ssh` 集成测试的**共用脚手架**：客户端的连接参数、数调用次数的凭据桩、
//! 阻塞读的辅助函数。
//!
//! 服务端本身**不在这里** —— 它已经提成 [`akasha_ssh::testing`]（plan 0504），
//! 因为 app 的 E2E 也要在它自己的进程里起同一台服务端。这里只 `pub use` 一下，
//! 好让各测试目标的 `use support::{…}` 保持不变。
//!
//! ⚠️ `tests/` 下的模块每个测试目标各编译一份，所以"共享"只到这一层为止；
//! 跨 crate 的东西必须住在库里（这正是那台服务端搬家的理由）。

#![allow(dead_code)] // 每个测试目标只用得到其中一部分
#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akasha_pty::{TerminalSize, Transport};
use akasha_ssh::{
    Credential, CredentialCache, CredentialProvider, CredentialRequest, HostKey, HostKeyVerifier,
    PinnedHostKey, SshAuth, SshConfig, SshConnect, SshError, SshTarget,
};

// 只 `pub use` 各测试目标真的会用到的那几个（`Observed` / `Shared` 用不到就别转出来：
// 一个没人用的转发会以 `unused_imports` 的形式红在门禁里）。
pub use akasha_ssh::testing::{Running, ServerOptions, start};

/// 数调用次数的凭据提供者（**判据"只问一次"就是数它**）。
pub struct CountingProvider {
    calls: AtomicUsize,
    answer: Mutex<String>,
    /// 每次都问了**哪一种**（断言"顺序"时比次数更有信息量）。
    kinds: Mutex<Vec<String>>,
}

impl CountingProvider {
    /// 建一个总是回答 `answer` 的桩。
    pub fn new(answer: &str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            answer: Mutex::new(answer.to_owned()),
            kinds: Mutex::new(Vec::new()),
        }
    }

    /// 被问过几次。
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// 被问过哪几种（按发生顺序）。
    pub fn kinds(&self) -> Vec<String> {
        self.kinds.lock().unwrap().clone()
    }

    /// 换一个答案（测"服务端拒绝之后重新问"那条路）。
    pub fn set_answer(&self, answer: &str) {
        *self.answer.lock().unwrap() = answer.to_owned();
    }
}

impl CredentialProvider for CountingProvider {
    fn request(&self, request: &CredentialRequest) -> Result<Credential, SshError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.kinds
            .lock()
            .unwrap()
            .push(request.kind.as_str().to_owned());
        let answer = self.answer.lock().unwrap().clone();
        Credential::new(answer.into_bytes())
    }
}

/// 客户端的连接参数。
pub fn connect_options(
    running: &Running,
    user: &str,
    auth: SshAuth,
    cache: Arc<CredentialCache>,
    provider: Arc<CountingProvider>,
) -> SshConnect {
    let verifier: Arc<dyn HostKeyVerifier> =
        Arc::new(PinnedHostKey::new(running.fingerprint.clone()));
    connect_options_with(running, user, auth, cache, provider, verifier)
}

/// 同上，但主机密钥策略由调用方给（测"钉错了"那条路）。
pub fn connect_options_with(
    running: &Running,
    user: &str,
    auth: SshAuth,
    cache: Arc<CredentialCache>,
    provider: Arc<CountingProvider>,
    host_keys: Arc<dyn HostKeyVerifier>,
) -> SshConnect {
    SshConnect {
        target: SshTarget::new("127.0.0.1", running.addr.port(), user),
        auth,
        cache,
        provider,
        host_keys,
        config: SshConfig {
            connect_timeout: Duration::from_secs(5),
            // 保活对本测试没有意义（连接活不到 30 秒），但**留着默认值**：
            // 这条路径要验的是"默认值能用"，不是"能关掉它"。
            ..SshConfig::default()
        },
        size: TerminalSize::DEFAULT,
    }
}

/// 从一个载体上读到出现 `needle` 为止（或超时）。
///
/// 为什么用一个线程 + `recv_timeout` 而不是 `set_read_timeout`：`Transport::output_stream`
/// 给的是 `Box<dyn Read>`，而阻塞读与"按时间交付"天生冲突（`docs/STATUS.md` 坑 #26）。
pub fn read_until(mut reader: Box<dyn Read + Send>, needle: &[u8], timeout: Duration) -> Vec<u8> {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    seen.extend_from_slice(&buf[..read]);
                    if tx.send(seen.clone()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut last = Vec::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(seen) => {
                last = seen;
                if last.windows(needle.len()).any(|window| window == needle) {
                    return last;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    last
}

/// 一个 `HostKeyVerifier`：**什么都拒绝**（测"密钥不被接受"时的对照组）。
pub struct RejectAll;

impl HostKeyVerifier for RejectAll {
    fn verify(&self, _target: &SshTarget, _key: &HostKey) -> Result<(), SshError> {
        Err(SshError::HostKeyRejected {
            fingerprint: "reject-all".to_owned(),
        })
    }
}

/// 让一个载体跑通一条命令（服务端会回声）。
pub fn round_trip(transport: &mut dyn Transport, text: &str) {
    let reader = transport.output_stream().expect("读端只能取一次");
    transport
        .write(format!("{text}\n").as_bytes())
        .expect("写入失败");
    let seen = read_until(reader, text.as_bytes(), Duration::from_secs(5));
    assert!(
        seen.windows(text.len())
            .any(|window| window == text.as_bytes()),
        "回声里没等到 {text}（拿到 {} 字节）",
        seen.len()
    );
}
