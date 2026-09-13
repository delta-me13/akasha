//! plan 0505 的库内验收：**`direct-tcpip` 原语 + 跳板**。
//!
//! 判据（ROADMAP 原文）=「ProxyJump 可连通**只对跳板机可见**的目标」。
//!
//! ## "只对跳板机可见"在这个测试里怎么成立
//!
//! 无特权环境里**做不出真正的网络隔离**（换 netns 要 root，加防火墙规则要 root），所以这条性质
//! 不是靠"连不上"造的，而是靠**名字**造的：客户端拿到的目标是
//! `akasha-jump-only.invalid:22`，而那个名字**只在跳板服务端的中继表里**存在
//! （`.invalid` 是 RFC 2606 保留的、永远解析不出来的顶级域）。
//!
//! 于是"直连"这条路**在构造上就不存在** —— 客户端连它要去的地址都解析不出来。而正例里字节
//! 确实到了**目标**服务端，那只可能经过跳板。另一半由负控补上（`a_direct_attempt…`）：
//! 不经跳板试一次**必须失败**，否则正例分不清"经了跳板"与"这个名字其实能直连"。
//!
//! 服务端两侧都在**测试进程内**（`akasha_ssh::testing`），所以"对端看到了什么"与"我们这边
//! 发生了什么"可以在一条用例里对账。

mod support;

use std::sync::Arc;
use std::time::Duration;

use akasha_ssh::testing::Relay;
use akasha_ssh::{
    CredentialCache, PinnedHostKey, SshAuth, SshConnect, SshError, SshTarget, SshTransport,
    Transport,
};
use support::{
    CONNECT_TIMEOUT, CountingProvider, Running, ServerOptions, connect_options, connect_options_to,
    read_until, start, wait_until,
};

/// 两跳各自的登录口令 —— **不一样**是有意的：它证明每一跳各问各的凭据（D8 的缓存键含 host）。
const JUMP_PASSWORD: &str = "jump-host-password";
const TARGET_PASSWORD: &str = "inner-host-password";

/// **只有跳板认识**的名字（理由见文件头）。端口取 22 而不是真实端口：在跳板的表里，
/// `22` 只是一个键，真正连到哪由那张表决定 —— 与真实 `HostName` + `ProxyJump` 完全同形。
const INNER_NAME: &str = "akasha-jump-only.invalid";
const INNER_PORT: u16 = 22;
const USER: &str = "cyrene";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("建 runtime 失败")
}

/// 起两台服务端：目标（只在跳板后面）与跳板（带着"我替你连到哪"的那张表）。
async fn two_hosts() -> (Running, Running) {
    let target = start(ServerOptions::password(TARGET_PASSWORD)).await;
    let jump = start(ServerOptions {
        password: Some(JUMP_PASSWORD.to_owned()),
        relay: vec![Relay {
            host: INNER_NAME.to_owned(),
            port: INNER_PORT,
            to: target.addr,
        }],
        ..ServerOptions::default()
    })
    .await;
    (jump, target)
}

/// 一条到目标（那个只有跳板认识的名字）的连接参数：钉住目标的指纹、给目标的口令。
fn inner_options(target: &Running, password: &str, timeout: Duration) -> SshConnect {
    connect_options_to(
        SshTarget::new(INNER_NAME, INNER_PORT, USER),
        Arc::new(PinnedHostKey::new(target.fingerprint.clone())),
        SshAuth::agent_only(),
        Arc::new(CredentialCache::new()),
        Arc::new(CountingProvider::new(password)),
        timeout,
    )
}

/// 判据本体：经跳板连上**只有它看得见**的目标，字节能双向流。
#[test]
fn jumping_reaches_a_host_only_the_bastion_can_see() {
    let runtime = runtime();
    let (jump, target) = runtime.block_on(two_hosts());

    // 两跳各一份连接参数（也就是各问各的凭据、各校各的主机密钥）。
    let cache = Arc::new(CredentialCache::new());
    let jump_options = connect_options(
        &jump,
        USER,
        SshAuth::agent_only(),
        Arc::clone(&cache),
        Arc::new(CountingProvider::new(JUMP_PASSWORD)),
    );
    let target_options = connect_options_to(
        SshTarget::new(INNER_NAME, INNER_PORT, USER),
        Arc::new(PinnedHostKey::new(target.fingerprint.clone())),
        SshAuth::agent_only(),
        Arc::clone(&cache),
        Arc::new(CountingProvider::new(TARGET_PASSWORD)),
        CONNECT_TIMEOUT,
    );

    let mut transport =
        SshTransport::connect_via(runtime.handle(), vec![jump_options], target_options)
            .expect("经跳板应当连得上");

    // ① 双向流：我们发出去的字节到了**目标**服务端，它的回声也回到了我们这里。
    let reader = transport.output_stream().expect("读端只能取一次");
    transport
        .write(b"echo via-the-bastion\n")
        .expect("写入失败");
    let seen = read_until(reader, b"via-the-bastion", Duration::from_secs(5));
    assert!(
        seen.windows(b"via-the-bastion".len())
            .any(|window| window == b"via-the-bastion"),
        "回声里没等到那句话（拿到 {} 字节）",
        seen.len()
    );

    // ② 目标那一半：那串字节真的到了**目标**服务端 —— 而客户端解析不出它的地址。
    assert!(
        target
            .shared
            .observed()
            .shell_data
            .windows(b"via-the-bastion".len())
            .any(|window| window == b"via-the-bastion"),
        "目标服务端没收到那串字节"
    );

    // ③ 跳板那一半：它被要求连的**正是那个只有它认识的名字**（端口也一样）。
    let via_jump = jump.shared.observed();
    assert_eq!(
        via_jump.direct_tcpip.len(),
        1,
        "跳板应当恰好被要求转发一次，实际：{:?}",
        via_jump.direct_tcpip
    );
    assert_eq!(via_jump.direct_tcpip[0].host, INNER_NAME);
    assert_eq!(via_jump.direct_tcpip[0].port, u32::from(INNER_PORT));
    // 两台机器各校各的密钥：目标是目标，跳板是跳板（指纹不同 = 我们真的握了两次手）。
    assert_ne!(jump.fingerprint, target.fingerprint);
    // 跳板自己**没有**收到过任何终端输入：它不是会话的对端，只是中转。
    assert!(via_jump.shell_data.is_empty(), "跳板上不该有终端数据");

    // ④ 收尾：目标看到通道关了；跳板上那条中继也收工（而且搬过字节 —— 它真的通着）。
    transport.shutdown().expect("收尾失败");
    wait_until(
        Duration::from_secs(5),
        "目标服务端看到通道关闭",
        || target.shared.observed().sessions_closed > 0,
    );
    wait_until(Duration::from_secs(5), "跳板上的中继收工", || {
        jump.shared.observed().relays_finished > 0
    });
    assert!(
        jump.shared.relayed_bytes() > 0,
        "中继应当搬过字节（跳板那一侧的计数）"
    );
}

/// **负控**：不经跳板，同一个名字**连不上**。
///
/// 没有这一条，正例分不清"经了跳板"与"这个名字其实能直连" —— 两种情况下正例的断言
/// 长得一模一样。期限给 2 秒：DNS 失败通常很快，但慢机器上不该把整条用例拖住
/// （超时也算"连不上"，因为断言的是**失败**这件事）。
#[test]
fn a_direct_attempt_at_that_name_fails() {
    let runtime = runtime();
    let (jump, target) = runtime.block_on(two_hosts());

    let options = inner_options(&target, TARGET_PASSWORD, Duration::from_secs(2));
    let err = SshTransport::connect(runtime.handle(), options)
        .err()
        .expect("不经跳板直连那个名字不可能成功");
    assert!(
        matches!(
            err,
            SshError::Connect { .. } | SshError::ConnectTimeout { .. }
        ),
        "直连失败的原因应当是连不上，实际是 {err:?}"
    );
    // 跳板这一次**根本没参与**：它没有被要求转发过任何东西。
    assert!(jump.shared.observed().direct_tcpip.is_empty());
}

/// 跳板的表里**没有**那个目标：拒绝转发，而错误说的是"转发到 X 失败"。
///
/// 这条钉住的是错误**分域**（`SshError::Forward` 而不是 `Connect`）：用户看到的必须能分清
/// "我连不上跳板机"与"跳板机连不上那台"。分不清就会去查自己的网络，而问题在对端。
#[test]
fn a_bastion_that_refuses_the_forward_says_so() {
    let runtime = runtime();
    // 跳板**不带**中继表 —— 它谁都到不了。
    let jump = runtime.block_on(start(ServerOptions::password(JUMP_PASSWORD)));
    let target = runtime.block_on(start(ServerOptions::password(TARGET_PASSWORD)));

    let jump_options = connect_options(
        &jump,
        USER,
        SshAuth::agent_only(),
        Arc::new(CredentialCache::new()),
        Arc::new(CountingProvider::new(JUMP_PASSWORD)),
    );
    let target_options = inner_options(&target, TARGET_PASSWORD, CONNECT_TIMEOUT);

    let err = SshTransport::connect_via(runtime.handle(), vec![jump_options], target_options)
        .err()
        .expect("跳板拒绝了转发，连接不可能成");
    match err {
        SshError::Forward { host, port, .. } => {
            assert_eq!(host, INNER_NAME);
            assert_eq!(port, INNER_PORT);
        }
        other => panic!("错误应当是 Forward，实际是 {other:?}"),
    }

    // 拒绝发生在"开通道"这一步：**跳板那条连接是好的**（它认证过了），
    // 坏的是"它够不着那台" —— 这正是把两者分开才能说清的话。
    let observed = jump.shared.observed();
    assert_eq!(
        observed
            .methods
            .iter()
            .filter(|method| *method == "password")
            .count(),
        1
    );
    assert_eq!(observed.direct_tcpip.len(), 1);
    assert_eq!(observed.relays_finished, 0);
}

/// 同步门面**不能在 tokio 上下文里**调（与 `connect` 同一条约束，plan 0505 没有放宽它）。
///
/// 这不是形式主义：`connect_via` 走 `Runtime::block_on`，而在 tokio 上下文里那会 panic，
/// 那条路径的调用方是 app 的命令层 —— 那里 panic 会连带丢掉整个 app（ADR-0003 D3）。
#[test]
fn the_sync_facade_refuses_to_be_called_inside_a_runtime() {
    let runtime = runtime();
    let (_jump, target) = runtime.block_on(two_hosts());

    runtime.block_on(async {
        let transport = SshTransport::connect_via(
            // ⚠️ 这里就在 runtime 里 —— 所以它必须**返回错误**而不是 panic。
            &tokio::runtime::Handle::current(),
            Vec::new(),
            inner_options(&target, TARGET_PASSWORD, CONNECT_TIMEOUT),
        );
        assert!(matches!(
            transport.err().expect("在 tokio 上下文里调用必须被拒"),
            SshError::BlockingInsideRuntime
        ));
    });
}
