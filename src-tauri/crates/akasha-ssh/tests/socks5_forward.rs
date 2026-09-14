//! plan 0603 的库内验收：**动态转发 `-D` 的 SOCKS5 服务端**。
//!
//! 判据（ROADMAP 原文）=「配置 SOCKS5 代理后**能访问远端网络**」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 经 SOCKS5 能访问远端服务 | 握手 → `CONNECT` → 写一行读回同一行（回声服务答的） |
//! | 目标由**客户端**说 | 目标名是握手报文里那一个，服务端的请求表里也是它 |
//! | 目标只可能经对端到达 | 目标名是 RFC 2606 保留域，用例自行解析一次并断言**失败** |
//! | 每条入站连接各开一条通道 | 再连一次 → 请求数变成 2 |
//! | 失败**分类**真的到了客户端 | 表里没有的名字 → `REP 0x02`（不是 `0x00`，也不是 `0x01`） |
//! | 拒绝之后连接被关掉 | 读完 `REP` 再读 → EOF（不是挂住） |
//! | 协议边界被明确拒绝 | `BIND` → `0x07`、未知 `ATYP` → `0x08`，且**不**开通道 |
//! | 停止后端口释放 | `shutdown()` → 再连该端口被拒 |
//!
//! ## 为什么客户端是手写的
//!
//! 库里的实现是**服务端**。用例这一侧必须是一个独立的客户端实现 —— 直接复用服务端的
//! 解析代码去"验"服务端，只会验出两块代码互相一致，验不出协议本身对不对。
//! 这也正是 app 的 E2E 还要再请一个**第三方**客户端（`curl --socks5-hostname`）的原因。
//!
//! ## "远端服务"在这个测试里是什么
//!
//! 与 plan 0602 同一种构造：一个进程内的回声服务端，通过测试服务端的**中继表**挂在一个
//! 只有对端认识的名字下。无特权环境做不出真正的网络隔离（见 plan 0505 的同一条说明），
//! 所以"够不着"这件事由**名字**保证。

mod support;

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use akasha_ssh::testing::Relay;
use akasha_ssh::{CredentialCache, Ingress, LocalListener, SshAuth, SshConnection};
use support::{CountingProvider, ServerOptions, connect_options, start, wait_until};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// 登录口令（测试服务端与客户端约定的那一句）。
const PASSWORD: &str = "socks5-password";
const USER: &str = "cyrene";

/// **只有对端认识**的名字（RFC 2606 的保留顶级域，永远解析不出来）。
const TARGET_NAME: &str = "akasha-socks5.invalid";
/// 中继表里**没有**的名字：客户端要它 → 对端拒绝开通道 → 客户端该收到 `0x02`。
const UNKNOWN_NAME: &str = "akasha-socks5-unknown.invalid";
/// 写进去、再从 SOCKS5 流里读回来的那串字节。
const PAYLOAD: &[u8] = b"akasha-socks5\n";

/// 一个进程内的回声服务端 —— 它就是"远端服务"。
async fn start_echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("回声服务绑定失败");
    let addr = listener.local_addr().expect("取回声服务地址失败");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let (mut read, mut write) = socket.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    addr
}

/// 连上 SOCKS5 监听端口并走完问候（写三种方法，断言服务端选中无认证）。
async fn greet(port: u16) -> TcpStream {
    let mut stream = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .expect("连 SOCKS5 端口超时")
    .expect("连 SOCKS5 端口失败");
    // 故意多给两个方法：只有真按 RFC 逐个看过的实现才会选中 0x00。
    stream
        .write_all(&[0x05, 0x03, 0x02, 0x80, 0x00])
        .await
        .expect("写问候失败");
    let mut chosen = [0u8; 2];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut chosen))
        .await
        .expect("等问候回复超时")
        .expect("读问候回复失败");
    assert_eq!(chosen, [0x05, 0x00], "服务端必须选中无认证");
    stream
}

/// 发一个 CONNECT 请求（`ATYP = 域名`），返回服务端回的 `REP` 字节。
async fn request(stream: &mut TcpStream, host: &str, port: u16) -> u8 {
    request_with(stream, 0x01, 0x03, host, port).await
}

/// 同上，但命令与地址形态可由用例指定（用来验拒绝那两档）。
async fn request_with(stream: &mut TcpStream, command: u8, atyp: u8, host: &str, port: u16) -> u8 {
    let mut message = vec![0x05, command, 0x00, atyp];
    match atyp {
        // 域名：长度 + 名字。
        0x03 => {
            message.push(u8::try_from(host.len()).expect("域名不该超过 255 字节"));
            message.extend_from_slice(host.as_bytes());
        }
        // 未知形态：给两个字节，服务端应当在读完 `ATYP` 之后就拒绝。
        0x09 => message.extend_from_slice(&[0xde, 0xad]),
        other => panic!("这个用例没准备 {other:#04x} 这种 ATYP 的构造"),
    }
    message.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&message).await.expect("写请求失败");
    read_reply(stream).await
}

/// 读一条 `REP` 报文（`VER REP RSV ATYP BND.ADDR BND.PORT`），返回 `REP`。
async fn read_reply(stream: &mut TcpStream) -> u8 {
    let mut head = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut head))
        .await
        .expect("等 REP 超时")
        .expect("读 REP 失败");
    assert_eq!(head[0], 0x05, "REP 的版本必须是 5");
    let mut tail = vec![
        0u8;
        match head[3] {
            0x01 => 4 + 2,
            0x04 => 16 + 2,
            0x03 => {
                let mut len = [0u8; 1];
                stream.read_exact(&mut len).await.expect("读域名长度失败");
                1 + usize::from(len[0]) + 2
            }
            other => panic!("REP 里的 ATYP {other:#04x} 不是 RFC 1928 定义的那三种"),
        }
    ];
    stream.read_exact(&mut tail).await.expect("读 BND 失败");
    head[1]
}

/// 经 SOCKS5 走一次往返：握手 → CONNECT → 写一行 → 读回同一行。
async fn round_trip(port: u16, host: &str, target_port: u16) -> Vec<u8> {
    let mut stream = greet(port).await;
    assert_eq!(
        request(&mut stream, host, target_port).await,
        0x00,
        "目标在表里，REP 必须是成功"
    );
    stream.write_all(PAYLOAD).await.expect("写失败");
    let mut echoed = vec![0u8; PAYLOAD.len()];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut echoed))
        .await
        .expect("等回声超时（转发没通）")
        .expect("读回声失败");
    echoed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_socks5_port_reaches_whatever_the_client_names() {
    // ── 1. 回声服务 + 一台 SSH 服务端（它的中继表是"远端服务"唯一的存在方式）────
    let echo = start_echo().await;
    let server = start(ServerOptions {
        password: Some(PASSWORD.to_owned()),
        relay: vec![Relay {
            host: TARGET_NAME.to_owned(),
            port: echo.port(),
            to: echo,
        }],
        ..ServerOptions::default()
    })
    .await;

    // 构造前提：这个名字在本机解析不出来（否则下面的断言分不清"经了隧道"与"直连"）。
    assert!(
        (TARGET_NAME, echo.port()).to_socket_addrs().is_err(),
        "构造前提破了：{TARGET_NAME} 在本机居然解析得出来"
    );

    // ── 2. 一条已认证、没有通道的连接 → 绑 SOCKS5 端口 → 起转发 ────────────────
    let mut options = connect_options(
        &server,
        USER,
        SshAuth::agent_only(),
        Arc::new(CredentialCache::new()),
        Arc::new(CountingProvider::new(PASSWORD)),
    );
    let connection = SshConnection::connect(&mut options)
        .await
        .expect("应当连得上测试服务端");

    // `port = 0`：由内核挑一个空闲端口，用例因此不必猜端口号。
    let listener = LocalListener::bind("127.0.0.1", 0, Ingress::Socks5)
        .await
        .expect("绑定 SOCKS5 端口失败");
    let port = listener.bound().port();
    let forward = listener.serve(&tokio::runtime::Handle::current(), connection);

    // ── 3. 判据：经 SOCKS5 能访问远端服务 ──────────────────────────────────────
    assert_eq!(
        round_trip(port, TARGET_NAME, echo.port()).await,
        PAYLOAD,
        "SOCKS5 那条流读回的不是写进去的那一行"
    );

    // 对端那一半：它被要求连的**正是客户端在握手里说的那个名字**，字节也真的搬过去了。
    wait_until(
        Duration::from_secs(5),
        "服务端记下 direct-tcpip 请求",
        || !server.shared.observed().direct_tcpip.is_empty(),
    );
    let first = server.shared.observed();
    assert_eq!(
        first.direct_tcpip.len(),
        1,
        "对端应当恰好被要求转发一次，实际：{:?}",
        first.direct_tcpip
    );
    assert_eq!(first.direct_tcpip[0].host, TARGET_NAME);
    assert_eq!(first.direct_tcpip[0].port, u32::from(echo.port()));
    wait_until(Duration::from_secs(5), "中继搬过字节", || {
        server.shared.relayed_bytes() > 0
    });

    // ── 4. 每条入站连接各开一条通道（不是"一条通道用到底"）─────────────────────
    assert_eq!(
        round_trip(port, TARGET_NAME, echo.port()).await,
        PAYLOAD,
        "第二条连接也该通"
    );
    wait_until(
        Duration::from_secs(5),
        "服务端记下第二条 direct-tcpip",
        || server.shared.observed().direct_tcpip.len() == 2,
    );

    // ── 5. 失败分类真的到了客户端：表里没有的名字 → 0x02 且连接被关掉 ──────────
    let mut stream = greet(port).await;
    assert_eq!(
        request(&mut stream, UNKNOWN_NAME, 9).await,
        0x02,
        "对端拒绝开通道时必须回「规则不允许」0x02，而不是通用的 0x01（客户端只能看到这一个字节）"
    );
    // 拒绝之后服务端要**关掉**这条连接：客户端看到 EOF，而不是挂在那儿等一个不会来的目标。
    let mut leftover = [0u8; 1];
    let closed = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut leftover))
        .await
        .expect("被拒之后连接没被关掉（客户端会挂住）");
    assert_eq!(
        closed.expect("读失败"),
        0,
        "连接应当以 EOF 结束，而不是继续给字节"
    );
    // 通道**尝试过**开（我们确实要求了），所以请求表里会有这个名字 —— 它是"分类来自对端的
    // 拒绝"这条推理的另一半。
    let seen = server.shared.observed();
    assert_eq!(
        seen.direct_tcpip
            .last()
            .map(|request| request.host.as_str()),
        Some(UNKNOWN_NAME),
        "表里没有的名字也该留下一条开通道的请求：{:?}",
        seen.direct_tcpip
    );

    // ── 6. 协议边界：不支持的命令与地址形态立刻被拒，**不**去开通道 ────────────
    let before = server.shared.observed().direct_tcpip.len();
    let mut stream = greet(port).await;
    assert_eq!(
        request_with(&mut stream, 0x02, 0x03, TARGET_NAME, echo.port()).await,
        0x07,
        "BIND（0x02）本版本不做 → 0x07"
    );
    let mut stream = greet(port).await;
    assert_eq!(
        request_with(&mut stream, 0x01, 0x09, "", 0).await,
        0x08,
        "不认的地址形态 → 0x08"
    );
    assert_eq!(
        server.shared.observed().direct_tcpip.len(),
        before,
        "被拒的请求不该去开通道（开了就等于把不支持的形态当成能用的）"
    );

    // ── 7. 停止：端口释放 + 那条连接断开 ───────────────────────────────────────
    forward.shutdown();
    wait_until(
        Duration::from_secs(5),
        "SOCKS5 端口不再接受连接",
        || std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
    );
    wait_until(
        Duration::from_secs(5),
        "服务端看到连接断开",
        || server.shared.observed().connections_closed > 0,
    );
}
