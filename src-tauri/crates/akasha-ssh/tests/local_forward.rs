//! plan 0602 的库内验收：**本地转发 `-L`**。
//!
//! 判据（ROADMAP 原文）=「转发端口**可访问远端服务**」。这条用例逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 本地端口能访问远端服务 | 从本地监听端口写一行，读回的**就是**那一行（回声服务答的） |
//! | 目标只可能经对端到达 | 目标名是 RFC 2606 保留域，用例自行解析一次并断言**失败** |
//! | 走的是 `direct-tcpip` | 服务端的请求表：恰好 1 条，`host` / `port` 与规则一致 |
//! | 每条入站连接各开一条通道 | 同一端口再连一次 → 请求数变成 2 |
//! | 字节真的过了通道 | 服务端的 `relayed_bytes > 0` |
//! | 停止后端口释放 | `shutdown()` → 再连该端口被拒 |
//! | 停止后连接断开 | 服务端的 `connections_closed ≥ 1` |
//!
//! ## "远端服务"在这个测试里是什么
//!
//! 一个进程内的回声服务端。它**同时**是我们进程里的东西，所以"够不着"这件事不是靠网络
//! 隔离造的（无特权环境做不出隔离，见 plan 0505 的同一条说明），而是靠**名字**：
//! 客户端要求转发到 `akasha-local-forward.invalid:<端口>`，那个名字**只在测试服务端的中继表里**
//! 存在。于是"字节到了回声服务"只可能经过那条 SSH 通道 —— 请求表与字节计数是这条推理的两半。

mod support;

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use akasha_ssh::testing::Relay;
use akasha_ssh::{CredentialCache, ForwardTarget, LocalListener, SshAuth, SshConnection, SshError};
use support::{CountingProvider, ServerOptions, connect_options, start, wait_until};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// 登录口令（测试服务端与客户端约定的那一句）。
const PASSWORD: &str = "local-forward-password";
const USER: &str = "cyrene";

/// **只有对端认识**的名字（RFC 2606 的保留顶级域，永远解析不出来）。
const TARGET_NAME: &str = "akasha-local-forward.invalid";

/// 写进去、再从转发端口读回来的那串字节。
const PAYLOAD: &[u8] = b"akasha-local-forward\n";

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

/// 经本地监听端口走一次往返（写一行、读回同样多字节）。
async fn round_trip(port: u16) -> Vec<u8> {
    let mut client = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .expect("连本地监听端口超时")
    .expect("连本地监听端口失败");
    client.write_all(PAYLOAD).await.expect("写失败");
    let mut echoed = vec![0u8; PAYLOAD.len()];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut echoed))
        .await
        .expect("等回声超时")
        .expect("读回声失败");
    echoed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_local_port_reaches_a_service_only_the_remote_side_can_name() {
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

    // 构造前提：这个名字在本机解析不出来。没有这一条，下面的断言分不清
    // "经了隧道"与"这个名字其实能直连"。
    assert!(
        (TARGET_NAME, echo.port()).to_socket_addrs().is_err(),
        "构造前提破了：{TARGET_NAME} 在本机居然解析得出来"
    );

    // ── 2. 一条已认证、没有通道的连接（D9 的类型）→ 绑本地端口 → 起转发 ────────
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

    // `port = 0`：由内核挑一个空闲端口，用例因此不需要猜端口号（也不必担心端口冲突）。
    let listener = LocalListener::bind("127.0.0.1", 0)
        .await
        .expect("绑定本地端口失败");
    let port = listener.bound().port();
    let forward = listener.serve(
        &tokio::runtime::Handle::current(),
        connection,
        ForwardTarget::new(TARGET_NAME, echo.port()),
    );

    // ── 3. 判据：转发端口可访问远端服务 ────────────────────────────────────────
    assert_eq!(
        round_trip(port).await,
        PAYLOAD,
        "转发端口读回的不是写进去的那一行"
    );

    // 对端那一半：它被要求连的**正是那个只有它认识的名字**，而且字节真的搬过去了。
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
    assert_eq!(round_trip(port).await, PAYLOAD, "第二条连接也该通");
    wait_until(
        Duration::from_secs(5),
        "服务端记下第二条 direct-tcpip",
        || server.shared.observed().direct_tcpip.len() == 2,
    );

    // ── 5. 停止：端口释放 + 那条连接断开 ───────────────────────────────────────
    forward.shutdown();
    wait_until(
        Duration::from_secs(5),
        "本地端口不再接受连接",
        || {
            // 监听撤掉之后连过去是"连接被拒"，不是"超时"——所以这一步可以用阻塞式 connect。
            std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
        },
    );
    wait_until(
        Duration::from_secs(5),
        "服务端看到连接断开",
        || server.shared.observed().connections_closed > 0,
    );
}

/// **负控**：没绑定就没有端口 —— 随机挑一个端口连过去必须是失败。
///
/// 没有这一条，上面那条用例的"停止后端口释放"分不清"监听真的撤了"与
/// "这个用例根本连不上任何端口"。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_port_nobody_bound_is_not_reachable() {
    let unbound = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("取一个空闲端口失败")
        .local_addr()
        .expect("取地址失败")
        .port();
    let refused = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect(("127.0.0.1", unbound)),
    )
    .await
    .expect("连接超时（本该立刻被拒）")
    .is_err();
    assert!(refused, "没人监听的端口不该连得上");
}

/// 绑定失败的那条路：错误里必须有**想绑的地址**，且与连接失败分属两个变体。
#[tokio::test]
async fn a_taken_port_is_a_listen_error_not_a_connect_error() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("占用一个端口失败");
    let port = held.local_addr().expect("取地址失败").port();
    let err = LocalListener::bind("127.0.0.1", port)
        .await
        .expect_err("端口被占用时不该绑定成功");
    assert!(
        matches!(err, SshError::Listen { .. }),
        "绑定失败必须是 Listen（用户要动的是端口，不是网络）：{err:?}"
    );
    assert!(
        err.to_string().contains(&port.to_string()),
        "错误里必须有那个端口（否则用户不知道动哪一个）：{err}"
    );
}
