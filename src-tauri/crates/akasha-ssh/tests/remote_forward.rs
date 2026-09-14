//! plan 0604 的库内验收：**远程转发 `-R`**。
//!
//! 判据（ROADMAP 原文）=「远端监听端口**可回连到本机服务**」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 远端端口可回连到本机服务 | 连**服务端**那个端口写一行，读回的**就是**那一行（本机回声服务答的） |
//! | 服务端真的在听 | 请求表：恰好 1 条，地址/端口与规则一致，实际端口与回报的一致 |
//! | `port = 0` 用服务端挑的那个（D10） | 请求 `0`，`RemoteForward::port()` 非 0 且可连 |
//! | 通道是**服务端发起**的 | `forwarded_tcpip_accepted ≥ 1`，且 `relayed_bytes > 0` |
//! | 每条入站连接各一条通道 | 再连一次 → `forwarded_tcpip_accepted` 变成 2 |
//! | 本机目标不可达**被拒** | 目标指向没人听的端口 → 服务端看到 `ConnectFailed`（不是"接受了又断"） |
//! | 停止即撤销 | `shutdown()` → 远端端口不再接受连接；服务端收到 `cancel-tcpip-forward` |
//! | 停止后连接断开 | 服务端的 `connections_closed ≥ 1` |
//!
//! ## "本机服务"与"远端"在这个测试里是什么
//!
//! 两侧都在测试进程里（无特权环境做不出网络隔离，同 plan 0602 / 0505 的说明），
//! 区分它们的是**谁在听**：远端监听端口由**测试服务端**在 `tcpip-forward` 里绑，
//! 本机服务由用例自己绑。字节因此只可能这样走：
//! 用例 → 服务端的监听端口 → `forwarded-tcpip` 通道 → 被测代码 → 本机回声服务。
//! 请求表与字节计数是这条推理的两半。
//!
//! ⚠️ 服务端**只绑回环**（`sshd` 的 `GatewayPorts` 默认如此），而请求里的地址原样记账
//! —— 所以"地址"这一条判据比的是请求说了什么，不是它绑到了哪。

mod support;

use std::net::{SocketAddr, TcpListener as StdListener};
use std::sync::Arc;
use std::time::Duration;

use akasha_ssh::{
    CredentialCache, ForwardEnd, ForwardTarget, RemoteForward, SshAuth, SshConnection, SshError,
};
use support::{CountingProvider, ServerOptions, connect_options, start, wait_until};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// 登录口令（测试服务端与客户端约定的那一句）。
const PASSWORD: &str = "remote-forward-password";
const USER: &str = "cyrene";

/// 写进去、再从远端监听端口读回来的那串字节。
const PAYLOAD: &[u8] = b"akasha-remote-forward\n";

/// 一个进程内的回声服务端 —— 它就是"**本机**上的服务"。
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

/// 经**远端**监听端口走一次往返（写一行、读回同样多字节）。
async fn round_trip(port: u16) -> Vec<u8> {
    let mut client = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .expect("连远端监听端口超时")
    .expect("连远端监听端口失败");
    client.write_all(PAYLOAD).await.expect("写失败");
    let mut echoed = vec![0u8; PAYLOAD.len()];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut echoed))
        .await
        .expect("等回声超时")
        .expect("读回声失败");
    echoed
}

/// 连一次就断开（用来把"服务端开了一条通道"这件事挤出来）。
async fn connect_and_drop(port: u16) {
    let _ = TcpStream::connect(("127.0.0.1", port)).await;
}

/// 一条已认证、没有通道的连接（D9 的类型）。
async fn connected(server: &akasha_ssh::testing::Running) -> SshConnection {
    let mut options = connect_options(
        server,
        USER,
        SshAuth::agent_only(),
        Arc::new(CredentialCache::new()),
        Arc::new(CountingProvider::new(PASSWORD)),
    );
    SshConnection::connect(&mut options)
        .await
        .expect("应当连得上测试服务端")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remote_port_reaches_a_service_on_this_side() {
    // ── 1. 本机回声服务 + 一台 SSH 服务端 ─────────────────────────────────────
    let echo = start_echo().await;
    let server = start(ServerOptions::password(PASSWORD)).await;

    // ── 2. 请服务端监听（`port = 0` → 由它挑，D10 要求使用返回值）─────────────
    let connection = connected(&server).await;
    let target = ForwardTarget::new("127.0.0.1", echo.port());
    // 结束通知那一半归重连循环（plan 0605）；这几条判据只看转发本体。
    let (forward, _ending) = RemoteForward::open(
        &tokio::runtime::Handle::current(),
        connection,
        "127.0.0.1",
        0,
        target,
    )
    .await
    .expect("服务端应当认下这条转发请求");

    let port = forward.port();
    assert_ne!(port, 0, "请求 0 端口时必须用服务端回报的那个端口");
    assert_eq!(forward.bound_display(), format!("127.0.0.1:{port}"));

    // 服务端那一半：它真的绑了、而且把我们请求的地址原样记着。
    let seen = server.shared.observed();
    assert_eq!(seen.forward_requests.len(), 1, "恰好一条请求：{seen:?}");
    assert_eq!(seen.forward_requests[0].address, "127.0.0.1");
    assert_eq!(
        seen.forward_requests[0].port, 0,
        "我们请求的是内核/服务端挑"
    );
    assert!(seen.forward_requests[0].accepted, "这条请求被认下了");
    assert_eq!(
        seen.forward_requests[0].bound_port, port,
        "回报给我们的端口必须就是它实际监听的端口"
    );

    // ── 3. 判据：远端监听端口可回连到本机服务 ────────────────────────────────
    assert_eq!(
        round_trip(port).await,
        PAYLOAD,
        "远端端口读回的不是本机服务写的那一行"
    );
    wait_until(
        Duration::from_secs(5),
        "服务端记下一条被接受的 forwarded-tcpip",
        || server.shared.observed().forwarded_tcpip_accepted == 1,
    );
    wait_until(Duration::from_secs(5), "中继搬过字节", || {
        server.shared.relayed_bytes() > 0
    });
    assert!(
        server.shared.observed().forwarded_tcpip_rejected.is_empty(),
        "成功那一条不该留下拒绝记录：{:?}",
        server.shared.observed().forwarded_tcpip_rejected
    );

    // ── 4. 每条入站连接各开一条通道（不是"一条通道用到底"）────────────────────
    assert_eq!(round_trip(port).await, PAYLOAD, "第二条连接也该通");
    wait_until(
        Duration::from_secs(5),
        "服务端记下第二条被接受的通道",
        || server.shared.observed().forwarded_tcpip_accepted == 2,
    );

    // ── 5. 停止：远端端口释放 + 撤销请求到达 + 连接断开 ──────────────────────
    forward.shutdown();
    wait_until(
        Duration::from_secs(5),
        "远端端口不再接受连接",
        || std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
    );
    wait_until(
        Duration::from_secs(5),
        "服务端收到 cancel-tcpip-forward",
        || !server.shared.observed().forward_cancellations.is_empty(),
    );
    let cancels = server.shared.observed().forward_cancellations;
    let expected = format!("127.0.0.1:{port}");
    assert_eq!(
        cancels.first().map(String::as_str),
        Some(expected.as_str()),
        "撤销请求要用**服务端回报的那个端口**（不是我们请求的 0）：{cancels:?}"
    );
    wait_until(
        Duration::from_secs(5),
        "服务端看到连接断开",
        || server.shared.observed().connections_closed > 0,
    );
}

/// **掉线**：那条连接没了之后，远端转发自己结束，**而且服务端那一侧的监听也还回去了**。
///
/// 两条都是 plan 0605 的前提：没有第一条，`-R` 的隧道会永远停在"已连接"（而远端端口其实
/// 早就不在了）；没有第二条，重连时对同一个端口的 `tcpip_forward` 会被自己上一次留下的
/// 监听顶掉 —— 真实的 `sshd` 里那条监听属于连接，连接一断它就消失。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_connection_ends_the_remote_forward_and_releases_the_port() {
    let echo = start_echo().await;
    let server = start(ServerOptions::password(PASSWORD)).await;

    let connection = connected(&server).await;
    let (forward, ending) = RemoteForward::open(
        &tokio::runtime::Handle::current(),
        connection,
        "127.0.0.1",
        0,
        ForwardTarget::new("127.0.0.1", echo.port()),
    )
    .await
    .expect("服务端应当认下这条转发请求");
    let port = forward.port();
    assert_eq!(round_trip(port).await, PAYLOAD, "切之前这条转发该是通的");

    assert_eq!(server.cut_connections().await, 1, "应当恰好切掉那一条连接");

    let end = tokio::time::timeout(Duration::from_secs(5), ending.ended())
        .await
        .expect("连接断了，远端转发却没结束");
    assert_eq!(
        end,
        ForwardEnd::ConnectionLost,
        "要报「连接没了」，不是「被停止」"
    );

    // 服务端那一侧：连接没了 → 它请来的监听随之消失（端口还回去了）。
    wait_until(
        Duration::from_secs(5),
        "远端端口随连接一起消失",
        || std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
    );
    // 断开那条连接的是服务端自己，所以它这边也记到"连接结束了"。
    wait_until(
        Duration::from_secs(5),
        "服务端看到连接断开",
        || server.shared.observed().connections_closed > 0,
    );
    forward.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_local_service_is_refused_before_the_channel_is_accepted() {
    // 目标指向一个**没人听**的端口：先用阻塞式监听占住再放掉，拿到的端口在短时间内没人听。
    let echo_port = {
        let held = StdListener::bind("127.0.0.1:0").expect("取一个空闲端口失败");
        held.local_addr().expect("取地址失败").port()
    };
    let server = start(ServerOptions::password(PASSWORD)).await;
    let connection = connected(&server).await;

    let (forward, _ending) = RemoteForward::open(
        &tokio::runtime::Handle::current(),
        connection,
        "127.0.0.1",
        0,
        ForwardTarget::new("127.0.0.1", echo_port),
    )
    .await
    .expect("监听请求本身应当被认下（问题在本机服务那一步）");
    let port = forward.port();

    connect_and_drop(port).await;
    wait_until(
        Duration::from_secs(5),
        "服务端看到那条通道被拒",
        || !server.shared.observed().forwarded_tcpip_rejected.is_empty(),
    );
    let seen = server.shared.observed();
    assert_eq!(
        seen.forwarded_tcpip_accepted, 0,
        "本机服务不可达时**不许**先接受再关 —— 对端的客户端会看到一条连上了就断的连接"
    );
    assert!(
        seen.forwarded_tcpip_rejected[0].contains("ConnectFailed"),
        "拒绝的原因必须是 ConnectFailed（本机服务不可达），实际：{:?}",
        seen.forwarded_tcpip_rejected
    );

    forward.shutdown();
}

/// **请求了一个具体端口时，"我们用哪个端口"就是请求的那个。**
///
/// 协议上，服务端**只在请求的是 0 端口时**才在回复里带端口（RFC 4254 §7.1），而上游把
/// "回复里没有端口字段"表示成 `0` —— 所以"回报 0"有两种截然不同的含义，读错的表现是
/// probe 报 `127.0.0.1:0`，而且是拿 0 去撤销监听（等于没撤销）。这条用例把两种含义分开钉住：
/// 上面那条用 0（由服务端挑），这条用一个具体的端口。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_specific_remote_port_is_the_one_we_requested() {
    let echo = start_echo().await;
    let server = start(ServerOptions::password(PASSWORD)).await;
    // 先占住再放掉，拿一个此刻空闲的具体端口。
    let wanted = {
        let held = StdListener::bind("127.0.0.1:0").expect("取一个空闲端口失败");
        held.local_addr().expect("取地址失败").port()
    };

    let connection = connected(&server).await;
    let (forward, _ending) = RemoteForward::open(
        &tokio::runtime::Handle::current(),
        connection,
        "127.0.0.1",
        wanted,
        ForwardTarget::new("127.0.0.1", echo.port()),
    )
    .await
    .expect("服务端应当认下这条转发请求");

    assert_eq!(
        forward.port(),
        wanted,
        "请求了具体端口时，我们手上的端口就是它（回复里没有端口字段，上游把它表示成 0）"
    );
    assert_eq!(forward.bound_display(), format!("127.0.0.1:{wanted}"));
    let seen = server.shared.observed();
    assert_eq!(seen.forward_requests[0].bound_port, wanted);
    assert_eq!(round_trip(wanted).await, PAYLOAD, "这个端口该是通的");

    forward.shutdown();
    wait_until(
        Duration::from_secs(5),
        "撤销请求用的是那个具体端口",
        || {
            server
                .shared
                .observed()
                .forward_cancellations
                .first()
                .is_some_and(|cancel| cancel == &format!("127.0.0.1:{wanted}"))
        },
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remote_port_the_server_cannot_bind_is_a_remote_listen_error() {
    // 服务端那一侧的那个端口被占着 —— 这是本类功能最常见的失败态。
    let held = StdListener::bind("127.0.0.1:0").expect("占用一个端口失败");
    let taken = held.local_addr().expect("取地址失败").port();
    let server = start(ServerOptions::password(PASSWORD)).await;
    let connection = connected(&server).await;

    let err = RemoteForward::open(
        &tokio::runtime::Handle::current(),
        connection,
        "127.0.0.1",
        taken,
        ForwardTarget::new("127.0.0.1", 9),
    )
    .await
    .expect_err("服务端绑不上那个端口时不该成功");

    assert!(
        matches!(err, SshError::RemoteListen { .. }),
        "远端监听没拿到必须是 RemoteListen（用户要动的地方在服务端）：{err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains(&taken.to_string()),
        "错误里必须有那个端口（否则用户不知道动哪一个）：{message}"
    );
    // 请求走到了服务端，被它拒绝了。
    let seen = server.shared.observed();
    assert_eq!(seen.forward_requests.len(), 1, "{seen:?}");
    assert!(!seen.forward_requests[0].accepted, "这一条应当是拒绝");
}
