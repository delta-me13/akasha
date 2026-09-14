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
use std::collections::{HashMap, HashSet};
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use rand::rng;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use russh::server::ChannelOpenHandle;
use russh::server::{Auth, Msg, Response, Server, Session};
use russh::{Channel, ChannelId, MethodSet, Pty};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

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
    /// 有几条**连接**已经断开（plan 0601：隧道停下来之后连接真的没有了）。
    ///
    /// 与 [`Observed::sessions_closed`] 分开是必要的：那个数的是**通道**关闭，
    /// 而隧道**没有通道**（ADR-0003 D4：隧道是纯字节管道）—— 它的连接断没断，
    /// 只有连接级的那一个数看得见。
    pub connections_closed: usize,
    /// 收到的 `direct-tcpip` 请求，按发生顺序（plan 0505：**跳板那一半的判据**）。
    ///
    /// "我们经了跳板"不能只看客户端 —— 这句话的证据是**对端被要求去连什么**。
    pub direct_tcpip: Vec<ForwardRequest>,
    /// 有几次中继**已经结束**（`copy_bidirectional` 收工）。
    pub relays_finished: usize,
    /// 收到的 `tcpip-forward` 请求，按发生顺序（plan 0604：`-R` 那一半的判据）。
    pub forward_requests: Vec<RemoteForwardRequest>,
    /// 收到的 `cancel-tcpip-forward`，按发生顺序（形态是 `地址:端口`）。
    pub forward_cancellations: Vec<String>,
    /// 有几条 `forwarded-tcpip` 通道**被对端接受**（plan 0604）。
    pub forwarded_tcpip_accepted: usize,
    /// 被对端拒掉的通道各是什么原因。
    ///
    /// 为什么记**原因**而不是只记次数：`ConnectFailed`（它连不上自己的本机服务）与
    /// `AdministrativelyProhibited`（它不认这条通道）是两件完全不同的事；
    /// 而"先接受、连不上再关"的实现会在这里**什么都不留下** ——
    /// 那正是这条用例要分得开的。
    pub forwarded_tcpip_rejected: Vec<String>,
}

/// 一次 `tcpip-forward` 请求（对端要求我们在**自己这一侧**监听）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteForwardRequest {
    /// 对端要求监听的地址（原样记下；我们实际只绑回环，见 `Handler::tcpip_forward`）。
    pub address: String,
    /// 对端要求的端口。`0` = 由我们挑。
    pub port: u32,
    /// 我们实际监听的端口（`0` 的请求就看它）。被拒时是 0。
    pub bound_port: u16,
    /// 我们认下了没有。
    pub accepted: bool,
}

/// 一次 `direct-tcpip` 请求（对端被告知"去连这里"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardRequest {
    /// 客户端要求连的地址 —— **在跳板机的网络里解析**，与客户端自己的 DNS 无关。
    pub host: String,
    pub port: u32,
    /// 客户端自报的发起方地址（RFC 4254 §7.2）。空串 = 它说"不知道"。
    pub originator: String,
    pub originator_port: u32,
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
    /// "我可以替你连到哪" —— 一台**真的**跳板机里那张表的位置。
    ///
    /// 没有匹配项的 `direct-tcpip` 一律**拒绝**（drop `reply`），与真实服务端的
    /// "转发不允许 / 连不上"同形。
    pub relay: Vec<Relay>,
}

/// 一条中继映射：**对端要求连**的地址 → **我们真的连**哪。
///
/// 两边不一样是刻意的：跳板机的意义就在于它能解析/够得着我们够不着的东西，
/// 所以测试里的目标地址是**只有跳板才认识**的（见 `tests/jump_host.rs`）。
#[derive(Debug, Clone)]
pub struct Relay {
    /// 客户端要求的地址（`host_to_connect`）。
    pub host: String,
    pub port: u16,
    /// 真的连到哪 —— 通常是测试进程内另一台服务端的监听地址。
    pub to: SocketAddr,
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
    /// 哪些通道是 **shell 通道**（由 `pty_request` / `shell_request` 认出来）。
    ///
    /// 为什么需要它：上游把通道数据**同时**交给通道自己的接收端**和** `Handler::data()`
    /// （`server/encrypted.rs:1251`），所以"回声"这件事必须只对 shell 通道做。否则
    /// `direct-tcpip` 那条通道上的字节会被原样打回去 —— 客户端读到的是**自己刚写的 id 行**，
    /// 表现为 `Bad packet size: 1397966893`（那串数字正是 `"SSH-"`，plan 0505 踩到过）。
    shell_channels: Arc<Mutex<HashSet<ChannelId>>>,
    /// 中继一共搬过多少字节（两个方向之和）—— **边搬边记**。
    ///
    /// 为什么不是"收工时一次性交出来"：`copy_bidirectional` 出错时**不交出**已搬的字节数，
    /// 而收尾那一下报错是常态（客户端撤了）。那样一来"搬过字节"这条判据会在最需要它的时候
    /// 永远是 0，而它本来是最直接的一条证据（见 [`Counted`]）。
    relayed_bytes: Arc<AtomicU64>,
    /// 已经接下的远端监听：`(地址, 实际端口) → 停掉那个接受循环`（plan 0604）。
    ///
    /// `cancel-tcpip-forward` 要在这里找到对应那一条。**真的停掉**而不是只记账：
    /// 判据里有一条是"停止之后远端端口不再接受连接"。
    forwardings: Arc<Mutex<HashMap<RemoteListenKey, oneshot::Sender<()>>>>,
}

/// 一条远端监听的键：`(对端请求的地址, 我们实际绑的端口)`。
///
/// 单独一个别名是为了让 [`Shared`] 那个字段读得出来（`Arc<Mutex<HashMap<…>>>` 全写开
/// 就是一串标点）。
type RemoteListenKey = (String, u16);

impl Shared {
    /// 认下一条 shell 通道。
    fn mark_shell(&self, channel: ChannelId) {
        self.shell_channels.lock().unwrap().insert(channel);
    }

    /// 这条通道是不是 shell 通道（只有它该收到回声）。
    fn is_shell(&self, channel: ChannelId) -> bool {
        self.shell_channels.lock().unwrap().contains(&channel)
    }

    /// 中继搬过多少字节（**可以在会话还开着的时候读**）。
    pub fn relayed_bytes(&self) -> u64 {
        self.relayed_bytes.load(Ordering::Relaxed)
    }

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

    /// 记一次 `tcpip-forward` 请求（认下与被拒都要记：被拒本身就是一条判据）。
    fn record_forward_request(&self, address: &str, port: u32, bound_port: u16, accepted: bool) {
        self.observed
            .lock()
            .unwrap()
            .forward_requests
            .push(RemoteForwardRequest {
                address: address.to_owned(),
                port,
                bound_port,
                accepted,
            });
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
        shell_channels: Arc::new(Mutex::new(HashSet::new())),
        relayed_bytes: Arc::new(AtomicU64::new(0)),
        forwardings: Arc::new(Mutex::new(HashMap::new())),
    };
    let mut server = TestServer {
        shared: shared.clone(),
        // 这个副本只用来 `new_client`（accept 循环）—— 它自己不是任何连接的 handler，
        // 所以它的 drop 不算"连接结束"。
        handler: false,
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
    /// 这个副本是不是**一条连接的 handler**（见 [`Drop`] 的实现）。
    handler: bool,
}

impl Drop for TestServer {
    /// 每条连接一个 handler，russh 在连接结束时销毁它 —— 于是"它被销毁"就是
    /// "那条连接结束了"。这是**连接级**的观察点（隧道没有通道，只能看这一个）。
    ///
    /// `#[derive(Clone)]` 的中间副本**不会**被算进来：只有 [`Server::new_client`]
    /// 返回的那个副本把 `handler` 置为真。
    fn drop(&mut self) {
        if self.handler {
            self.shared.observed.lock().unwrap().connections_closed += 1;
        }
    }
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
        let mut handler = self.clone();
        handler.handler = true;
        handler
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

    /// `direct-tcpip`：一台跳板机被要求"去连那里"。
    ///
    /// 两个分支都要有，因为判据的两半都靠它：
    /// * **有映射** → 接受 + 双向中继（于是"经跳板真的通了"有服务端这一侧的证据）；
    /// * **没有映射** → **drop `reply`** = 拒绝（真实服务端在"转发不允许 / 连不上"时就是这样）。
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        originator_address: &str,
        originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared
            .observed
            .lock()
            .unwrap()
            .direct_tcpip
            .push(ForwardRequest {
                host: host_to_connect.to_owned(),
                port: port_to_connect,
                originator: originator_address.to_owned(),
                originator_port,
            });

        let destination = self
            .shared
            .options
            .lock()
            .unwrap()
            .relay
            .iter()
            .find(|relay| relay.host == host_to_connect && u32::from(relay.port) == port_to_connect)
            .map(|relay| relay.to);

        // 没有映射 → 什么都不做。`reply` 在这一行之后被 drop，
        // 而上游的契约正是"drop 掉句柄 = 拒绝这个通道"。
        let Some(destination) = destination else {
            return Ok(());
        };

        reply.accept().await;
        let shared = self.shared.clone();
        let counter = Arc::clone(&self.shared.relayed_bytes);
        tokio::spawn(async move {
            let Ok(upstream) = TcpStream::connect(destination).await else {
                return;
            };
            // 两侧都包一层计数器，于是**两个方向**的字节都算进来（README 那半句"搬过多少"）。
            let mut client_side = Counted::new(channel.into_stream(), Arc::clone(&counter));
            let mut target_side = Counted::new(upstream, counter);
            // **两条出路都算"收工"**：收尾那一下（客户端撤了）让某一侧报错是常态，
            // 而"中继已经结束"与"结束时是否干净"是两件事。
            let _ = copy_bidirectional(&mut client_side, &mut target_side).await;
            shared.observed.lock().unwrap().relays_finished += 1;
        });
        Ok(())
    }

    /// `-R` 的那一半：对端请我们**在它那一侧**监听（plan 0604）。
    ///
    /// 这里真的绑一个端口 —— 判据是"远端监听端口可回连到本机服务"，
    /// 而"回连"这件事要有一个真的在听的 socket 才说得通。
    ///
    /// 绑的地址**永远**是回环，不看请求里的 `address`：真实服务端也是这样
    /// （`sshd` 的 `GatewayPorts` 默认只允许回环），而"请求什么就绑什么"会让用例
    /// 在开发机上开一个对外的端口。请求的地址原样记下来，供判据比对。
    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let requested = u16::try_from(*port).unwrap_or(0);
        let forward_address = address.to_owned();
        let listener = match TcpListener::bind(("127.0.0.1", requested)).await {
            Ok(listener) => listener,
            Err(_) => {
                // 绑不上 = 拒绝这条请求（对端因此收到 `RequestDenied`，
                // 与真实服务端的"端口被占用"同形）。
                self.shared.record_forward_request(address, *port, 0, false);
                return Ok(false);
            }
        };
        let bound = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
        *port = u32::from(bound);
        self.shared
            .record_forward_request(address, u32::from(requested), bound, true);

        let handle = session.handle();
        let shared = self.shared.clone();
        let (stop, mut stopped) = oneshot::channel::<()>();
        self.shared
            .forwardings
            .lock()
            .unwrap()
            .insert((address.to_owned(), bound), stop);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => match accepted {
                        Ok((socket, peer)) => {
                            let handle = handle.clone();
                            let shared = shared.clone();
                            let address = forward_address.clone();
                            tokio::spawn(async move {
                                let opened = handle
                                    .channel_open_forwarded_tcpip(
                                        address,
                                        u32::from(bound),
                                        peer.ip().to_string(),
                                        u32::from(peer.port()),
                                    )
                                    .await;
                                match opened {
                                    Ok(channel) => {
                                        shared.observed.lock().unwrap().forwarded_tcpip_accepted += 1;
                                        let counter = Arc::clone(&shared.relayed_bytes);
                                        let mut client_side =
                                            Counted::new(channel.into_stream(), Arc::clone(&counter));
                                        let mut target_side = Counted::new(socket, counter);
                                        let _ = copy_bidirectional(&mut client_side, &mut target_side)
                                            .await;
                                        shared.observed.lock().unwrap().relays_finished += 1;
                                    }
                                    Err(err) => {
                                        // 对端拒了这条通道：把**原因**记下来
                                        // （`ConnectFailed` = 它连不上自己的本机服务）。
                                        // socket 随作用域结束而关闭。
                                        shared
                                            .observed
                                            .lock()
                                            .unwrap()
                                            .forwarded_tcpip_rejected
                                            .push(format!("{err:?}"));
                                    }
                                }
                            });
                        }
                        Err(_) => break,
                    },
                }
            }
        });
        Ok(true)
    }

    /// 撤销上面那一条：**真的停掉**那个监听（判据里有一条是"停止之后端口不再接受连接"）。
    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let bound = u16::try_from(port).unwrap_or(0);
        let stop = self
            .shared
            .forwardings
            .lock()
            .unwrap()
            .remove(&(address.to_owned(), bound));
        let Some(stop) = stop else {
            return Ok(false);
        };
        self.shared
            .observed
            .lock()
            .unwrap()
            .forward_cancellations
            .push(format!("{address}:{bound}"));
        let _ = stop.send(());
        Ok(true)
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
        self.shared.mark_shell(channel);
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared.mark_shell(channel);
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // 只记 shell 通道的数据：`direct-tcpip` 通道上流的是**另一条 SSH 连接**的字节，
        // 把它算进"终端收到过什么"会让判据读到一个它没在说的事实。
        if !self.shared.is_shell(channel) {
            return Ok(());
        }
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

/// 数着字节搬的包装：每写出一次就加一次计数。
///
/// 为什么不用 `copy_bidirectional` 的返回值：它**出错时不交出**已搬的字节数，而收尾那一下
/// 报错是常态 —— 于是"这条中继搬过多少"会在最需要它的时候变成 0。记在写这一侧就没有这个
/// 问题（而且**会话还开着的时候就能读**，见 `Shared::relayed_bytes`）。
struct Counted<T> {
    inner: T,
    counter: Arc<AtomicU64>,
}

impl<T> Counted<T> {
    fn new(inner: T, counter: Arc<AtomicU64>) -> Self {
        Self { inner, counter }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Counted<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Counted<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let written = ready!(Pin::new(&mut self.inner).poll_write(cx, buf))?;
        self.counter.fetch_add(written as u64, Ordering::Relaxed);
        Poll::Ready(Ok(written))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
