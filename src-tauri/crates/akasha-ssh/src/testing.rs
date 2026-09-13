//! **进程内的 SSH 服务端** —— SSH 那几条判据的观察点（plan 0502 起，plan 0504 提为公开模块）。
//!
//! 为什么不用系统 `sshd`（开发机上确实有 `/usr/bin/sshd`）：
//!
//! * 它要 root 或一份精心构造的配置才能以非特权用户跑起来 —— 而 CI 的三个平台都要跑；
//! * 它**不会告诉我们**"服务端看到的认证方式顺序"这件事，而那正是 D7 的判据；
//! * 它会把状态写进用户目录（`~/.ssh`、主机密钥、日志），测试不该动用户的文件。
//!
//! ⚠️ 因此它证明的是**客户端这条链**（顺序、缓存、失效、字节往返），**不是**与
//! OpenSSH 的互操作。后者要真机实测，记在 `docs/STATUS.md` 的待验证里。
//!
//! ## 为什么它是公开模块而不是 `tests/support/`
//!
//! plan 0504 的 E2E 要**在测试进程里**起一台服务端，让**另一个进程**里的 app 连过来 ——
//! 判据的两半（"服务端看到了什么"与"界面上看到了什么"）必须在同一份代码里对账。
//! 与 `akasha_pty::testing::FakeTransport` 同一个先例：**测试脚手架住在库里，注释写明
//! 生产代码不要用它**。两份各自演进的假服务端会漂移，而漂移的方向总是"测试以为验了"。
//!
//! ## 生产代码不要用它
//!
//! 它开监听端口、接受任何带对口令的连接、把收到的字节原样回声 —— 它**不是**一个服务端实现，
//! 只是把客户端的每条分支逼出来的装置。

#![allow(dead_code)] // 每个使用点只用得到其中一部分
#![allow(clippy::unwrap_used)] // 测试脚手架：这里的 unwrap 是断言手段（生产代码的基线见 root Cargo.toml）

use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::rng;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use russh::server::ChannelOpenHandle;
use russh::server::{Auth, Msg, Response, Server, Session};
use russh::{Channel, ChannelId, MethodSet, Pty};
use tokio::net::TcpListener;

use crate::handshake::HostKey;

/// 服务端记下来的事实。**判据就断言在这里**，不靠日志反推。
#[derive(Debug, Default, Clone)]
pub struct Observed {
    /// 服务端看到的认证方式，按发生顺序。
    pub methods: Vec<String>,
    /// `password` 认证收到的口令原样（断言"服务端看到的是哪一句"）。
    pub passwords: Vec<String>,
    /// `publickey` 认证报上来的密钥指纹（断言"用的是哪把钥匙"）。
    pub offered_keys: Vec<String>,
    /// 收到过几次 pty 请求。
    pub pty_requests: usize,
    /// 收到过几次窗口尺寸变化。
    pub window_changes: usize,
    /// shell 通道上收到过的字节（断言字节往返）。
    pub shell_data: Vec<u8>,
    /// 有几条连接**已经断开**（plan 0504：关标签页之后连接真的没了）。
    ///
    /// 数的是"通道被关掉"这件事 —— 客户端收尾时会 `eof` + `close`；只数"曾经来过几条"
    /// 分不出"还挂着"与"收干净了"。
    pub sessions_closed: usize,
}

/// 服务端行为的可调部分（测试可以在运行中改它 —— 例如"把这个口令改成不认"）。
#[derive(Debug, Clone, Default)]
pub struct ServerOptions {
    /// 认哪句登录口令。`None` = 不接受口令认证。
    pub password: Option<String>,
    /// 认哪些私钥（按其 `SHA256:…` 指纹）。
    pub accepted_keys: Vec<String>,
    /// keyboard-interactive 认哪个码。`None` = 不提供这一档。
    pub keyboard_code: Option<String>,
    /// 客户端请求关闭时回哪个退出码。
    pub exit_status: u32,
}

impl ServerOptions {
    /// 只认口令。
    pub fn password(password: &str) -> Self {
        Self {
            password: Some(password.to_owned()),
            ..Self::default()
        }
    }
}

/// 服务端的共享状态：观察点 + 可调行为。
#[derive(Clone)]
pub struct Shared {
    /// 服务端看到的一切。
    pub observed: Arc<Mutex<Observed>>,
    /// 服务端的行为。
    pub options: Arc<Mutex<ServerOptions>>,
}

impl Shared {
    fn record(&self, method: &str) {
        self.observed
            .lock()
            .unwrap()
            .methods
            .push(method.to_owned());
    }

    /// 服务端现在认哪句口令（测试用它中途改行为）。
    pub fn set_password(&self, password: Option<&str>) {
        self.options.lock().unwrap().password = password.map(str::to_owned);
    }

    /// 读一份观察结果的**拷贝**（不把锁带出去）。
    pub fn observed(&self) -> Observed {
        self.observed.lock().unwrap().clone()
    }
}

/// 一个跑起来的测试服务端。
pub struct Running {
    /// 监听地址。
    pub addr: SocketAddr,
    /// 主机密钥指纹（客户端要核对的就是它）。
    pub fingerprint: String,
    /// 主机密钥的 **OpenSSH 形式**（`ssh-ed25519 AAAA…`）。
    ///
    /// 与 `fingerprint` 是同一把密钥的两种表示；测试要写一份**用户的** `known_hosts` 时
    /// 需要的是它（那份文件的第三列就是这串 base64）。
    pub host_key_openssh: String,
    /// 共享状态。
    pub shared: Shared,
}

/// 从 OpenSSH 文本（`ssh-ed25519 AAAA… comment`）造一把 [`HostKey`]。
///
/// 存在的理由：`HostKey` 的正常来路是"服务端在握手里给的"，所以生产代码里没有构造口
/// （那是刻意的，见 `handshake.rs`）。而**提问那条路**要验的分支（接受 / 拒绝 / 没人答）
/// 需要一个输入，于是构造口留在这里 —— 与这个模块的其余部分同一条纪律。
pub fn host_key_from_openssh(text: &str) -> HostKey {
    let key = PublicKey::from_openssh(text).expect("测试公钥必须是合法的 OpenSSH 文本");
    HostKey::from_public(&key).expect("测试公钥必须能编码成线格式")
}

/// 起一个测试服务端（绑定 `127.0.0.1:0`，随机端口）。
pub async fn start(options: ServerOptions) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定端口失败");
    let addr = listener.local_addr().expect("取本地地址失败");

    let host_key = PrivateKey::random(&mut rng(), Algorithm::Ed25519).expect("生成主机密钥失败");
    let fingerprint = host_key
        .public_key()
        .fingerprint(HashAlg::Sha256)
        .to_string();
    let host_key_openssh = host_key.public_key().to_openssh().expect("公钥编码失败");

    let mut config = russh::server::Config::default();
    config.keys.push(host_key);
    // 上游默认让每条被拒的认证等 1 秒（常数时间，防时序侧信道）。测试要快，设 0。
    config.auth_rejection_time = Duration::from_millis(0);
    let config = Arc::new(config);

    let shared = Shared {
        observed: Arc::new(Mutex::new(Observed::default())),
        options: Arc::new(Mutex::new(options)),
    };
    let mut server = TestServer {
        shared: shared.clone(),
    };
    // 自己写 accept 循环而不是 `run_on_socket`：后者的返回 future 借了 `server` 与
    // `listener`（edition 2024 的 `impl Trait` 会捕获输入生命期），而这里要把它
    // 丢进 `tokio::spawn`。自己循环则两边都被 move 进任务，没有借出。
    tokio::spawn(async move {
        loop {
            let Ok((socket, peer)) = listener.accept().await else {
                break;
            };
            let handler = server.new_client(Some(peer));
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                let _ = russh::server::run_stream(config, socket, handler).await;
            });
        }
    });

    Running {
        addr,
        fingerprint,
        host_key_openssh,
        shared,
    }
}

#[derive(Clone)]
struct TestServer {
    shared: Shared,
}

impl TestServer {
    /// "我不认这个，但我还愿意谈这些" —— 真实服务端就是这么答的。
    fn reject() -> Auth {
        Auth::Reject {
            proceed_with_methods: Some(MethodSet::server_supported()),
            partial_success: false,
        }
    }
}

impl Server for TestServer {
    type Handler = Self;

    fn new_client(&mut self, _peer: Option<SocketAddr>) -> Self {
        self.clone()
    }
}

impl russh::server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.shared.record("password");
        self.shared
            .observed
            .lock()
            .unwrap()
            .passwords
            .push(password.to_owned());
        let accepted = self.shared.options.lock().unwrap().password.clone();
        if accepted.as_deref() == Some(password) {
            Ok(Auth::Accept)
        } else {
            Ok(Self::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &russh::keys::PublicKey,
    ) -> Result<Auth, Self::Error> {
        self.shared.record("publickey");
        let fingerprint = public_key.fingerprint(HashAlg::Sha256).to_string();
        self.shared
            .observed
            .lock()
            .unwrap()
            .offered_keys
            .push(fingerprint.clone());
        if self
            .shared
            .options
            .lock()
            .unwrap()
            .accepted_keys
            .iter()
            .any(|accepted| accepted == &fingerprint)
        {
            Ok(Auth::Accept)
        } else {
            Ok(Self::reject())
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let Some(code) = self.shared.options.lock().unwrap().keyboard_code.clone() else {
            return Ok(Self::reject());
        };
        match response {
            // 第一次：把问题交出去。**在这一刻记一次**（后面还有一次带答案的回调，
            // 那一次不是新的一档，记两次会把顺序读错）。
            None => {
                self.shared.record("keyboard-interactive");
                Ok(Auth::Partial {
                    name: Cow::Borrowed("otp"),
                    instructions: Cow::Borrowed("enter code"),
                    prompts: Cow::Owned(vec![(Cow::Borrowed("code"), false)]),
                })
            }
            Some(mut answers) => {
                let answer = answers
                    .next()
                    .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    .unwrap_or_default();
                if answer == code {
                    Ok(Auth::Accept)
                } else {
                    Ok(Self::reject())
                }
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _cols: u32,
        _rows: u32,
        _pix_w: u32,
        _pix_h: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared.observed.lock().unwrap().pty_requests += 1;
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared
            .observed
            .lock()
            .unwrap()
            .shell_data
            .extend_from_slice(data);
        // 回声：客户端"能拿到输出"这条判据靠它。
        let _ = session.data(channel, data.to_vec());
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        cols: u32,
        rows: u32,
        _pix_w: u32,
        _pix_h: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared.observed.lock().unwrap().window_changes += 1;
        // 把收到的尺寸**回声**回去：客户端读到它就等于"window_change 真的到了对端"，
        // 而不是"我们发出去过"。没有这一条，断言只能靠"发过"，那测的是我们自己。
        let _ = session.data(channel, format!("window:{cols}x{rows}\n").into_bytes());
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // 客户端说"我关通道了" —— 一个真实 shell 在这时会报结局再关。
        self.shared.observed.lock().unwrap().sessions_closed += 1;
        let status = self.shared.options.lock().unwrap().exit_status;
        let _ = session.exit_status_request(channel, status);
        let _ = session.close(channel);
        Ok(())
    }
}
