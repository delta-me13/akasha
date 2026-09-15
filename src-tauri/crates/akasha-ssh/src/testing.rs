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
use std::fs::File as StdFile;
use std::io;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle as SftpHandle, Name, OpenFlags, Status, StatusCode,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

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
    /// 收到过几次 `sftp` 子系统请求、并且**认下了**（plan 0701）。
    ///
    /// 数的是"认下"而不是"收到"：被拒的那一次在客户端那一侧应当表现为
    /// `SftpClient` 建不起来 —— 两件事合起来才说明拒绝那条路通了。
    pub sftp_subsystems: usize,
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
    /// 提供 `sftp` 子系统，并在根目录里把这几项**真的建出来**（plan 0701 / 0702）。
    /// `None` = 不提供 —— 那时 `subsystem_request` **明确拒绝**。
    ///
    /// ⚠️ 它在 `start()` 那一刻被**读一次并落成真目录**（见 [`Running::sftp_root`]）：
    /// 此后改这个字段不会改变任何事。
    pub sftp: Option<Vec<SftpItem>>,
    /// 每次 `read` / `write` 之前先睡这么久（plan 0702）。
    ///
    /// 存在的理由很具体：**传输的中断要有可乘之机**。全速跑的本机回环上，一个几兆的文件
    /// 在一瞬间就搬完了，用例来不及点"取消" —— 于是判据会变成一条随机器快慢而红的用例。
    /// 放慢服务端（而不是在用例里睡固定时间）让"取消"这件事发生在**确定的位置**：
    /// 传输确实在跑，且它还剩很多没搬。
    pub sftp_delay: Option<Duration>,
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

/// 测试服务端那个 SFTP 根目录里的一项（plan 0701 / 0702）。
///
/// `start()` 把它**落到真盘上**：一个文件就是真文件（`content` 是它的字节），
/// 一个目录就是真目录。于是"文件真的到了对端盘上"这条判据可以拿**真盘**去答，
/// 而不是读客户端自己报的数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpItem {
    /// 条目名。
    pub name: String,
    /// 是不是目录 —— 客户端据此决定"这一项能不能进去"。
    pub directory: bool,
    /// 文件内容（目录忽略它）。
    pub content: Vec<u8>,
}

impl SftpItem {
    /// 一个**空**的普通文件。
    pub fn file(name: &str) -> Self {
        Self::file_with(name, Vec::new())
    }

    /// 一个有内容的普通文件（下载的判据要它：字节对不对得比）。
    pub fn file_with(name: &str, content: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.to_owned(),
            directory: false,
            content: content.into(),
        }
    }

    /// 一个目录。
    pub fn dir(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            directory: true,
            content: Vec::new(),
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
    /// 已经建立、还没收工的那些连接（plan 0605）。
    ///
    /// 重连那几条判据需要一个"把线路切断"的动作，见 [`Connection::cut`]。
    /// 已经收工的不必留着（accept 时顺手 retain 掉），否则这张表会随重连次数一直长。
    connections: Arc<Mutex<Vec<Connection>>>,
    /// **会话通道**（plan 0701）：`sftp` 子系统在 `subsystem_request` 里拿到的是一个
    /// `ChannelId`，而那个子系统要的是**通道本体**。
    ///
    /// 因此 `channel_open_session` 把通道存下来（此前它是被丢掉的）。
    /// ⚠️ 丢弃不再安全：上游把通道数据同时交给通道自己的接收端**与** `Handler::data()`
    /// （见 `shell_channels` 的说明），而"存下来"只是多一个持有者，不改变数据路径。
    session_channels: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
    /// SFTP 根目录（plan 0702）：**`Some` = 这个服务端提供 `sftp` 子系统**。
    ///
    /// 它是 `ServerOptions::sftp` 在 `start()` 那一刻落成的真目录 —— 之后那一条不再被读，
    /// 于是"提供了没有"这个问题只有一个答案（两处各存一份必然漂移）。
    sftp_root: Option<PathBuf>,
    /// 每次 `read` / `write` 前的等待（见 `ServerOptions::sftp_delay`）。
    sftp_delay: Option<Duration>,
}

/// 一条远端监听的键：`(对端请求的地址, 我们实际绑的端口)`。
///
/// 单独一个别名是为了让持有它的那个字段读得出来（`Arc<Mutex<HashMap<…>>>` 全写开
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
    /// accept 循环的任务句柄（[`Running::shutdown`] 用它把监听一起停掉）。
    accept: tokio::task::AbortHandle,
    /// 这个服务端的 SFTP 根目录（真盘上的一棵临时目录）。
    ///
    /// 它同时是**这一侧判据的读数口**：传输有没有留下不完整的文件、最终名的字节对不对，
    /// 都拿这个目录去答。持有它是为了在 [`Drop`] 里把目录删掉 —— 测试不该在 `/tmp`
    /// 留东西。
    sftp_root: Option<PathBuf>,
}

impl Running {
    /// **切断已经建立的连接**，监听照常（plan 0605）。
    ///
    /// 这是"线路中断"那个刺激：连接断了，而客户端接下来再去连**同一个端口**仍然连得上。
    /// 返回切断了几条：判据要能分清"切到了"与"当时一条都没有"（那两种情况下一次失败的
    /// 信息完全不一样）。
    ///
    /// ⚠️ 断开是**异步**收尾的：对端会话与它请来的远端监听都要过一会儿才真的消失，
    /// 所以断言 `connections_closed` 或者"那个端口已经还回去了"的时候要等。
    pub async fn cut_connections(&self) -> usize {
        let live: Vec<Connection> = {
            let mut held = self.shared.connections.lock().unwrap();
            held.drain(..).collect()
        };
        let cut = live.len();
        for connection in &live {
            connection.cut().await;
        }
        cut
    }

    /// **整个服务端消失**：先停监听（新连接一律被拒），再切断已建立的连接。
    ///
    /// 两种失败的次序与真实情形一致：对端进程没了，既没人听，原来那条连接也断了。
    pub async fn shutdown(&self) -> usize {
        self.accept.abort();
        self.cut_connections().await
    }

    /// 这个服务端的 SFTP 根目录（**真盘上**那一棵，plan 0702）。
    ///
    /// 传输的判据在这里读：目标目录里有没有最终名、有没有剩下的临时名、最终名的字节对不对。
    /// `None` = 这个服务端根本没提供 `sftp` 子系统。
    pub fn sftp_root(&self) -> Option<&Path> {
        self.sftp_root.as_deref()
    }

    /// 服务端此刻**还开着几条连接**（plan 0606）。
    ///
    /// 这是"连接真的断了"的**另一半证据**：客户端那边说自己账上归零了，有可能只是它丢掉了
    /// 自己的句柄；对端看不见那条会话才是这条连接确实结束。已经收工的任务顺手清掉
    /// （会话结束与"任务结束"之间有一小段，所以断言要等，同 [`Self::cut_connections`] 的说明）。
    pub fn live_connections(&self) -> usize {
        let mut held = self.shared.connections.lock().unwrap();
        held.retain(|connection| !connection.is_finished());
        held.len()
    }
}

/// 一条活着的连接（plan 0605 的"切断"动作要它）。
///
/// ⚠️ **不能靠 abort 那个包装任务来切断连接**：`russh::server::run_stream` 把真正的会话
/// **又 spawn 了一层**（`session.run(...)`），abort 外层只是把 `RunningSession` 丢掉，
/// 里面那条任务照跑 —— socket 与 handler 都归它，连接毫发无损（实测：abort 之后
/// `connections_closed` 一直是 0）。所以切断走会话自己的 `Handle::disconnect`，
/// 那也正是"服务端把这条连接断开"的真实形态。
struct Connection {
    /// 会话句柄。`run_stream` 换完 SSH id 之后才填进来，之前是 `None`
    /// （那种连接还没到能切断的地步）。
    session: Arc<Mutex<Option<russh::server::Handle>>>,
    task: JoinHandle<()>,
}

impl Connection {
    fn start(config: Arc<russh::server::Config>, socket: TcpStream, handler: TestServer) -> Self {
        let session = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&session);
        let task = tokio::spawn(async move {
            let Ok(running) = russh::server::run_stream(config, socket, handler).await else {
                return;
            };
            *slot.lock().unwrap() = Some(running.handle());
            // 等这条会话自己收工：`running` 同时是"会话还活着"的凭据。
            let _ = running.await;
        });
        Self { session, task }
    }

    fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// 让会话自己断开（对端因此收到一条 `SSH_MSG_DISCONNECT` 并看到连接关闭）。
    async fn cut(&self) -> bool {
        let handle = self.session.lock().unwrap().clone();
        match handle {
            Some(handle) => handle
                .disconnect(
                    russh::Disconnect::ByApplication,
                    String::new(),
                    String::new(),
                )
                .await
                .is_ok(),
            None => false,
        }
    }
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

    // SFTP 的根目录在这里**落地**（plan 0702）：`options.sftp` 是种子，真目录才是事实。
    // 两个字段都要在 `options` 被 move 进 `Shared` 之前取出来。
    let sftp_root = options.sftp.as_ref().map(|items| materialize_sftp(items));
    let sftp_delay = options.sftp_delay;
    let shared = Shared {
        observed: Arc::new(Mutex::new(Observed::default())),
        options: Arc::new(Mutex::new(options)),
        shell_channels: Arc::new(Mutex::new(HashSet::new())),
        relayed_bytes: Arc::new(AtomicU64::new(0)),
        connections: Arc::new(Mutex::new(Vec::new())),
        session_channels: Arc::new(Mutex::new(HashMap::new())),
        sftp_root: sftp_root.clone(),
        sftp_delay,
    };
    let mut server = TestServer {
        shared: shared.clone(),
        // 这个副本只用来 `new_client`（accept 循环）—— 它自己不是任何连接的 handler，
        // 所以它的 drop 不算"连接结束"，它也不持有任何远端监听。
        handler: false,
        forwardings: HashMap::new(),
    };
    // 自己写 accept 循环而不是 `run_on_socket`：后者的返回 future 借了 `server` 与
    // `listener`（edition 2024 的 `impl Trait` 会捕获输入生命期），而这里要把它
    // 丢进 `tokio::spawn`。自己循环则两边都被 move 进任务，没有借出。
    let connections = Arc::clone(&shared.connections);
    let accept = tokio::spawn(async move {
        loop {
            let Ok((socket, peer)) = listener.accept().await else {
                break;
            };
            let handler = server.new_client(Some(peer));
            let config = Arc::clone(&config);
            let connection = Connection::start(config, socket, handler);
            let mut live = connections.lock().unwrap();
            // 已经收工的那些不必留着：这张表是"当前活着的连接"，不是流水账。
            live.retain(|connection| !connection.is_finished());
            live.push(connection);
        }
    });

    Running {
        addr,
        fingerprint,
        host_key_openssh,
        shared,
        accept: accept.abort_handle(),
        sftp_root,
    }
}

impl Drop for Running {
    /// 把 SFTP 根目录删掉（见那个字段的说明）。**尽力而为**：删不掉只记一条日志式忽略，
    /// 因为 drop 里报错只会变成一次 panic，而它发生在用例的收尾上。
    fn drop(&mut self) {
        if let Some(root) = self.sftp_root.take() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

/// 把种子落成真目录（plan 0702），返回那个目录。
///
/// 目录名带进程号与一个自增号：同一个测试进程里可能同时开着几台服务端（`sftp_dual_pane`
/// 就是两台），它们各自的根目录不能撞在一起。
fn materialize_sftp(items: &[SftpItem]) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "akasha-sftp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // 上一次没删干净（进程被杀）时先清掉：`create_dir_all` 不会因为目录已存在而失败，
    // 于是旧的残留会让"列出来的条目"多出上一次的东西。
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 SFTP 根目录失败");
    for item in items {
        let path = dir.join(&item.name);
        if item.directory {
            std::fs::create_dir_all(&path).expect("建 SFTP 目录失败");
        } else {
            std::fs::write(&path, &item.content).expect("写 SFTP 文件失败");
        }
    }
    dir
}

struct TestServer {
    shared: Shared,
    /// 这个副本是不是**一条连接的 handler**（见 [`Drop`] 的实现）。
    handler: bool,
    /// 这条连接请我们开的远端监听（plan 0604 / 0605）。
    ///
    /// ⚠️ **按连接持有**（而不是放在 [`Shared`] 里全局一张表）：真实的 `sshd` 里
    /// 一条转发属于**那条连接**——连接一断，那个监听就随之消失。全局一张表的话，
    /// 客户端重连之后对同一个端口的 `tcpip_forward` 会被自己上一次留下的监听顶掉，
    /// 而那条监听其实已经没有主人了（plan 0605 的重连用例正是踩这个形状的场景）。
    forwardings: HashMap<RemoteListenKey, oneshot::Sender<()>>,
}

impl Clone for TestServer {
    /// 副本**不带**任何远端监听：`Server::new_client` 要的是"新的一条连接"，
    /// 而监听是那条连接自己请来的，不该从 accept 循环那份里继承
    /// （`oneshot::Sender` 也不能克隆 —— 它只有一个接收端）。
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            handler: self.handler,
            forwardings: HashMap::new(),
        }
    }
}

impl Drop for TestServer {
    /// 每条连接一个 handler，russh 在连接结束时销毁它 —— 于是"它被销毁"就是
    /// "那条连接结束了"。这是**连接级**的观察点（隧道没有通道，只能看这一个）。
    ///
    /// `#[derive(Clone)]` 的中间副本**不会**被算进来：只有 [`Server::new_client`]
    /// 返回的那个副本把 `handler` 置为真。
    ///
    /// 连接结束后顺带**放掉它请来的那些远端监听**：真实 `sshd` 的转发属于连接，
    /// 连接一断端口就还回去（见 `forwardings` 字段的说明）。
    fn drop(&mut self) {
        if self.handler {
            self.shared.observed.lock().unwrap().connections_closed += 1;
            for (_, stop) in self.forwardings.drain() {
                let _ = stop.send(());
            }
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
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        // 存下来：`sftp` 子系统的请求随后要用**同一个通道**（那个回调只给 `ChannelId`，
        // 见 `session_channels` 的说明）。其余通道（shell / 转发）不受影响 ——
        // 数据仍然同时走 `Handler::data()`。
        self.shared
            .session_channels
            .lock()
            .unwrap()
            .insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    /// `sftp` 子系统（plan 0701）。两档见 [`ServerOptions::sftp`]：`start()` 建出了根目录
    /// 就认下并把通道交给一个最小但**碰真盘**的 SFTP 服务端；没有（或不是 `sftp`）
    /// **明确回绝** —— 客户端要在 `request_subsystem` 那里拿到 `false`，
    /// 而不是等到第一条请求超时。
    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let root = if name == "sftp" {
            self.shared.sftp_root.clone()
        } else {
            None
        };
        let Some(root) = root else {
            let _ = session.channel_failure(channel_id);
            return Ok(());
        };
        let Some(channel) = self
            .shared
            .session_channels
            .lock()
            .unwrap()
            .remove(&channel_id)
        else {
            let _ = session.channel_failure(channel_id);
            return Ok(());
        };
        self.shared.observed.lock().unwrap().sftp_subsystems += 1;
        let _ = session.channel_success(channel_id);
        let delay = self.shared.sftp_delay;
        russh_sftp::server::run(channel.into_stream(), SftpRoot::new(root, delay)).await;
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
        // 记在**这条连接**名下：连接一结束，这个监听随之消失（真实 `sshd` 就是这么做的）。
        self.forwardings.insert((address.to_owned(), bound), stop);

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
        let stop = self.forwardings.remove(&(address.to_owned(), bound));
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

/// **测试用的 SFTP 服务端**（plan 0701 起，plan 0702 扩到读写）。
///
/// ⚠️ 它**不是**一个完整的 SFTP 实现：没有扩展、没有 `setstat`、没有符号链接，
/// 权限位是粗的（只分清文件 / 目录 / 链接）。它要回答的只有一件事：
/// **客户端那条路走通之后，真盘上发生了什么**。
///
/// 这个"真盘"是 plan 0702 改的。此前它是一张内存表，够验"列目录"，但答不出传输的判据 ——
/// 交付的判据是"目标目录里**没有**最终名下的文件"，而内存表只能证明"客户端自己以为写成功了"。
/// 根目录由 `start()` 建（见 [`materialize_sftp`]），路径 `/x/y` 映射到根目录下的 `x/y`。
///
/// ⚠️ 用的是阻塞的 `std::fs`：这里是测试脚手架，一次读写的耗时不值得为它引一层异步文件 API，
/// 而它跑在测试进程里、不服务于生产代码（模块文档的"生产代码不要用它"）。
struct SftpRoot {
    /// 真盘上的根目录；客户端看到的 `/` 就是它。
    root: PathBuf,
    /// 每个 `read` / `write` 之前先等这么久（`ServerOptions::sftp_delay`）。
    delay: Option<Duration>,
    /// 打开着的文件句柄：句柄名 → (它指向哪个路径, 文件本体)。
    files: HashMap<String, (PathBuf, StdFile)>,
    /// 已经建好的目录句柄：句柄名 → **还没发出去的那些条目**。
    ///
    /// 用"取走"而不是"记一个发过的标记"：`readdir` 的结束条件是回 `EOF`，
    /// 而"发过一次就 EOF"正是这个结构表达的东西。
    dirs: HashMap<String, Vec<File>>,
    /// 句柄名里的自增部分（同一个连接里不能重复）。
    next_handle: u64,
}

impl SftpRoot {
    fn new(root: PathBuf, delay: Option<Duration>) -> Self {
        Self {
            root,
            delay,
            files: HashMap::new(),
            dirs: HashMap::new(),
            next_handle: 0,
        }
    }

    /// 客户端给的路径 → 真盘上的路径。
    ///
    /// 自己解析 `.` / `..` 而不是 `Path::join`：SFTP 的路径规范是 POSIX 的，
    /// 而 `..` 必须**夹在根目录上**（测试服务端不该让客户端走到 `/tmp` 去）——
    /// `PathBuf::pop` 做不到这件事（它会一直退到文件系统根）。
    fn resolve(&self, path: &str) -> PathBuf {
        let mut parts: Vec<&str> = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        let mut resolved = self.root.clone();
        for part in parts {
            resolved.push(part);
        }
        resolved
    }

    /// 真盘上的路径 → 客户端看到的路径（`realpath` 与目录条目的名字都由它给）。
    fn virtual_path(&self, resolved: &Path) -> String {
        let relative = resolved.strip_prefix(&self.root).unwrap_or(resolved);
        let text = relative.to_string_lossy().replace('\\', "/");
        if text.is_empty() {
            "/".to_owned()
        } else {
            format!("/{text}")
        }
    }

    /// 一个还没用过的句柄名。
    fn handle(&mut self, kind: char) -> String {
        self.next_handle += 1;
        format!("{kind}{}", self.next_handle)
    }

    /// `open` 打开的那个路径（`fstat` 与 `close` 都要它）。
    ///
    /// ⚠️ `id` 必须**原样**进返回的句柄：上游的服务端分发是
    /// `Ok(packet) => packet.into()`，也就是说响应里的请求号**取自 handler 的返回值**，
    /// 不是分发器补上去的。写死 0 的表现是"服务端答了、客户端永远等不到"。
    fn open_file(
        &mut self,
        id: u32,
        path: &str,
        pflags: OpenFlags,
    ) -> Result<SftpHandle, StatusCode> {
        let resolved = self.resolve(path);
        let mut options = std::fs::OpenOptions::new();
        options.read(pflags.contains(OpenFlags::READ));
        options.write(pflags.contains(OpenFlags::WRITE));
        options.append(pflags.contains(OpenFlags::APPEND));
        if pflags.contains(OpenFlags::CREATE) || pflags.contains(OpenFlags::EXCLUDE) {
            options.create(true);
        }
        if pflags.contains(OpenFlags::TRUNCATE) {
            options.truncate(true);
        }
        let file = options.open(&resolved).map_err(status_of)?;
        let name = self.handle('f');
        self.files.insert(name.clone(), (resolved, file));
        Ok(SftpHandle { id, handle: name })
    }
}

impl russh_sftp::server::Handler for SftpRoot {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> impl std::future::Future<Output = Result<russh_sftp::protocol::Version, Self::Error>> + Send
    {
        // 明确**不声明任何扩展**（`limits@openssh.com` / `fsync@openssh.com` 一个都不给）：
        // 客户端在缺扩展时的降级路径因此每次都被走到，而那正是真实 `sshd` 之外最该覆盖的一档。
        std::future::ready(Ok(russh_sftp::protocol::Version::new()))
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<SftpHandle, Self::Error> {
        self.open_file(id, &filename, pflags)
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        // 目录句柄与文件句柄共用一个命名空间（前缀不同），所以两边都试一下。
        self.files.remove(&handle);
        self.dirs.remove(&handle);
        Ok(ok_status(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        let Some((_, file)) = self.files.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        file.seek(SeekFrom::Start(offset)).map_err(status_of)?;
        let mut buffer = vec![0u8; len as usize];
        let filled = file.read(&mut buffer).map_err(status_of)?;
        if filled == 0 {
            // 协议规定的读完标记是 `EOF` 状态，不是"发一批空的"。
            return Err(StatusCode::Eof);
        }
        buffer.truncate(filled);
        Ok(Data { id, data: buffer })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        let Some((_, file)) = self.files.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        file.seek(SeekFrom::Start(offset)).map_err(status_of)?;
        file.write_all(&data).map_err(status_of)?;
        Ok(ok_status(id))
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let Some((_, file)) = self.files.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        let metadata = file.metadata().map_err(status_of)?;
        Ok(Attrs {
            id,
            attrs: attributes_of(&metadata),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let resolved = self.resolve(&path);
        let metadata = std::fs::metadata(&resolved).map_err(status_of)?;
        Ok(Attrs {
            id,
            attrs: attributes_of(&metadata),
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let resolved = self.resolve(&path);
        let metadata = std::fs::symlink_metadata(&resolved).map_err(status_of)?;
        Ok(Attrs {
            id,
            attrs: attributes_of(&metadata),
        })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        let resolved = self.resolve(&filename);
        std::fs::remove_file(&resolved).map_err(status_of)?;
        Ok(ok_status(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        let from = self.resolve(&oldpath);
        let to = self.resolve(&newpath);
        std::fs::rename(&from, &to).map_err(status_of)?;
        Ok(ok_status(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<SftpHandle, Self::Error> {
        let resolved = self.resolve(&path);
        let reader = std::fs::read_dir(&resolved).map_err(status_of)?;
        let mut entries = Vec::new();
        for entry in reader {
            let entry = entry.map_err(status_of)?;
            let metadata = entry.metadata().map_err(status_of)?;
            entries.push(File::new(
                entry.file_name().to_string_lossy().into_owned(),
                attributes_of(&metadata),
            ));
        }
        let name = self.handle('d');
        self.dirs.insert(name.clone(), entries);
        Ok(SftpHandle { id, handle: name })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        match self.dirs.get_mut(&handle) {
            None => Err(StatusCode::Failure),
            Some(entries) if entries.is_empty() => Err(StatusCode::Eof),
            Some(entries) => Ok(Name {
                id,
                files: std::mem::take(entries),
            }),
        }
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        // 客户端拿这一句的返回值当"当前目录"，所以这里必须回答**已经解析过**的路径
        // （`.` → `/`，`/a/../b` → `/b`）—— 与真实服务端的 `realpath` 同义。
        let resolved = self.resolve(&path);
        Ok(Name {
            id,
            files: vec![File::new(
                self.virtual_path(&resolved),
                FileAttributes::dummy(),
            )],
        })
    }
}

/// 一条 `OK` 状态回执。
fn ok_status(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_owned(),
        language_tag: "en-US".to_owned(),
    }
}

/// 真盘的元数据 → 协议里的属性。
///
/// ⚠️ 只设**命中**的那一个类型位，而且**不得**顺手把另外两个设成 `false`：
/// `set_dir(false)` 做的是 `permissions &= !DIR`，而 `REG`（0o100000）、`LNK`（0o120000）、
/// `DIR`（0o040000）的位**互相重叠** —— 清掉 `LNK` 会连带清掉 `REG`，于是普通文件的类型
/// 变成"都不是"，客户端读出来是 `Other`（实测：这一条正是本文件第一次跑出来的一处红）。
fn attributes_of(metadata: &std::fs::Metadata) -> FileAttributes {
    let mut attrs = FileAttributes::empty();
    attrs.size = Some(metadata.len());
    if metadata.is_dir() {
        attrs.set_dir(true);
    } else if metadata.file_type().is_symlink() {
        attrs.set_symlink(true);
    } else if metadata.is_file() {
        attrs.set_regular(true);
    }
    attrs
}

/// 操作系统的错误 → 协议状态码。
///
/// 只分"没有这个文件"与"权限不够"两类：其余一律 `Failure`。**不猜**是一个刻意的选择 ——
/// 猜错的类别会把客户端引到一条与实际原因无关的路上（同 `SshError` 的分法）。
fn status_of(err: io::Error) -> StatusCode {
    match err.kind() {
        io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
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
