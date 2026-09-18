//! plan 0703 的库内验收：**host ↔ host 的两档**（`scope.md` §4.1，ADR-0006 D5）。
//!
//! D5 把两档定成"选哪一对端点"，这条用例就按那个形状写：同一个 `transfer()`，
//! 换的只是目标端点怎么来。
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | **B 档**：本机只连跳板，目标在它的网络里 | 目标名字在本机**解析不出来**；跳板记到**恰好一条** `direct-tcpip`，中继搬过字节 |
//! | **B 档**：字节真的落在目标盘上 | 目标那棵真目录里是最终名、字节与源逐字节相同、没有临时名 |
//! | **B 档不可用**是一个可判定的失败 | 跳板的中继表里没有那条映射时，`over` 报 `SshError::Forward` |
//! | **A 档**（回退）：本机分别连两台 | 两档共用同一个引擎，字节同样落到目标盘上 |
//! | **两档都不落盘** | 两档的端点都是**远端**端点（引擎拿不到本机路径）；源那棵目录一个条目都没多 |
//!
//! ## "只有跳板认识目标"在这个用例里怎么成立
//!
//! 与 plan 0505 同一条做法：没有 root 就做不出真正的网络隔离，所以性质靠**名字**造 ——
//! 目标是 `akasha-sftp-only.invalid:22`（RFC 2606 保留域，永远解析不出来），它只在跳板
//! 服务端的中继表里存在。负控那一半证明"直连这条路在构造上不存在"，于是正例里落地的字节
//! 只可能经过跳板。
//!
//! ## 为什么走同步门面
//!
//! app 那一侧用 [`SshConnection::connect_via_until`]（B 档 = 多一跳前缀的链），这条用例走
//! **同一条**：于是"链真的把跳板那一段活着带上了"这件事被覆盖到，而不是只覆盖 `over`。
//! ⚠️ 由此也有一条形状上的约束：`over` **不持有**承载它的那条连接（它只造下一跳），
//! 所以它要求调用方自己把承载者活着 —— `chain` / `connect_via` 是持有它的那条路。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "ssh_support/mod.rs"]
mod support;

use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use akasha_lib::ssh::testing::{Relay, Running, ServerOptions, SftpItem, start};
use akasha_lib::ssh::transfer::{Cancel, Progress, TransferRequest, transfer};
use akasha_lib::ssh::{CredentialCache, PinnedHostKey, SshAuth, SshConnection, SshError, SshTarget};
use support::{CONNECT_TIMEOUT, CountingProvider, connect_options, connect_options_to};

/// 两台服务端各用各的登录口令：`via` 那一跳要真的再认证一次（凭据不是从目标那台借的）。
const BASTION_PASSWORD: &str = "bastion-password";
const TARGET_PASSWORD: &str = "target-password";
const USER: &str = "cyrene";

/// **只有跳板认识**的名字与端口（理由见文件头）：`22` 在跳板那张表里只是一个键。
const ONLY_NAME: &str = "akasha-sftp-only.invalid";
const ONLY_PORT: u16 = 22;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("建 runtime 失败")
}

/// 目标那一台的 SFTP 根里放一个目录（落盘判据要看它），跳板那台放源文件。
async fn two_hosts(relay: bool) -> (Running, Running) {
    let target = start(ServerOptions {
        password: Some(TARGET_PASSWORD.to_owned()),
        sftp: Some(vec![SftpItem::dir("inbox")]),
        ..ServerOptions::default()
    })
    .await;
    let bastion = start(ServerOptions {
        password: Some(BASTION_PASSWORD.to_owned()),
        sftp: Some(vec![SftpItem::file_with("from.bin", payload())]),
        // `relay = false` 就是"这台跳板不认那条转发"：真实服务端在
        // `AllowTcpForwarding no` 或"它自己连不上目标"时正是这个表现。
        relay: if relay {
            vec![Relay {
                host: ONLY_NAME.to_owned(),
                port: ONLY_PORT,
                to: target.addr,
            }]
        } else {
            Vec::new()
        },
        ..ServerOptions::default()
    })
    .await;
    (bastion, target)
}

/// 源文件的字节：够分几块（`CHUNK_BYTES` = 32 KiB），又能一眼比对。
fn payload() -> Vec<u8> {
    (0u8..=255).cycle().take(200 * 1024).collect()
}

/// 目标那一台的连接参数：地址是**只有跳板认识**的那个名字，指纹钉住真服务端的。
fn target_options(target: &Running, cache: Arc<CredentialCache>) -> akasha_lib::ssh::SshConnect {
    connect_options_to(
        SshTarget::new(ONLY_NAME, ONLY_PORT, USER),
        Arc::new(PinnedHostKey::new(target.fingerprint.clone())),
        SshAuth::keys(Vec::new()),
        cache,
        Arc::new(CountingProvider::new(TARGET_PASSWORD)),
        CONNECT_TIMEOUT,
    )
}

fn request<'a>(source: &'a str, target: &'a str) -> TransferRequest<'a> {
    TransferRequest {
        source_path: source,
        target_path: target,
    }
}

/// 一个服务端的 SFTP 根里有哪些条目（排序）—— 判据的读数口。
fn names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn inbox(root: &Path) -> PathBuf {
    root.join("inbox")
}

/// **B 档**：在到跳板的连接上开一条 `direct-tcpip` 直通目标，SFTP 跑在隧道里。
///
/// 本机这一侧只连跳板一台 —— 目标那个名字在本机解析不出来（本用例自己解析一次并断言失败），
/// 而它的字节确实到了目标盘上。
#[test]
fn a_tunnel_moves_a_file_between_two_hosts() {
    let runtime = runtime();
    let (bastion, target) = runtime.block_on(two_hosts(true));

    // 判据的前提：本机**够不着**目标那台（所以"字节到了目标"只可能经过跳板）。
    assert!(
        (ONLY_NAME, ONLY_PORT).to_socket_addrs().is_err(),
        "{ONLY_NAME} 在本机解析出来了 —— 这条用例的前提不成立"
    );

    let cache = Arc::new(CredentialCache::new());
    // 源那一栏：本机直连跳板（A 档里它也是这条）。
    let source_conn = SshConnection::connect_via(
        runtime.handle(),
        Vec::new(),
        connect_options(
            &bastion,
            USER,
            SshAuth::keys(Vec::new()),
            Arc::clone(&cache),
            Arc::new(CountingProvider::new(BASTION_PASSWORD)),
        ),
    )
    .expect("直连跳板失败");
    // 目标那一栏：`hops = [跳板]`，终点是那个只在跳板表里的名字。
    let tunneled = SshConnection::connect_via(
        runtime.handle(),
        vec![connect_options(
            &bastion,
            USER,
            SshAuth::keys(Vec::new()),
            Arc::clone(&cache),
            Arc::new(CountingProvider::new(BASTION_PASSWORD)),
        )],
        target_options(&target, Arc::clone(&cache)),
    )
    .expect("经跳板直通目标失败");

    runtime.block_on(async {
        let source = source_conn.sftp().await.expect("源那一台的 SFTP 建不起来");
        let sink = tunneled.sftp().await.expect("隧道里的 SFTP 建不起来");
        let copied = transfer(
            &source,
            &sink,
            &request("/from.bin", "/inbox/landed.bin"),
            &Progress::default(),
            Cancel::new().waiter(),
        )
        .await
        .expect("B 档的传输应当成功");
        assert_eq!(
            copied,
            payload().len() as u64,
            "搬过去的字节数应当是源文件的大小"
        );
    });

    // 落盘判据的第一半：目标**真盘**上是最终名、字节相同、没有临时名。
    let target_root = target.sftp_root().unwrap();
    assert_eq!(
        std::fs::read(inbox(target_root).join("landed.bin")).unwrap(),
        payload(),
        "目标盘上的字节必须与源相同"
    );
    assert_eq!(
        names(&inbox(target_root)),
        vec!["landed.bin".to_owned()],
        "目标目录里不该多出临时名"
    );

    // 第二半：源那一台一个条目都没多（源只被读，不落任何东西）。
    let bastion_root = bastion.sftp_root().unwrap();
    assert_eq!(
        names(bastion_root),
        vec!["from.bin".to_owned()],
        "源那一台的目录不该有任何变化"
    );

    // 服务端那一半：跳板被要求**恰好一次**去连那个名字，而且中继真的搬了字节。
    let seen = bastion.shared.observed();
    assert_eq!(
        seen.direct_tcpip.len(),
        1,
        "跳板应当收到恰好一条 direct-tcpip：{:?}",
        seen.direct_tcpip
    );
    assert_eq!(seen.direct_tcpip[0].host, ONLY_NAME);
    assert_eq!(seen.direct_tcpip[0].port, u32::from(ONLY_PORT));
    assert!(
        bastion.shared.relayed_bytes() > 0,
        "跳板的中继一个字节都没搬 —— 那说明 SFTP 没跑在隧道里"
    );
    eprintln!(
        "B 档：跳板收到 1 条 direct-tcpip → {ONLY_NAME}:{ONLY_PORT}，中继搬了 {} 字节；目标盘上 landed.bin 的字节数与源一致",
        bastion.shared.relayed_bytes()
    );
}

/// **B 档不可用**是一个可判定的失败：跳板不认那条转发时，建链报 `SshError::Forward`。
///
/// 这条就是 app 那一侧回退的触发条件（`sftp_connect` 只在"直通没建起来"时改走本机直连）。
/// 顺带验证"直连这条路不存在"：不经跳板连目标那个名字必须失败。
#[test]
fn a_bastion_that_refuses_the_forward_is_not_a_route() {
    let runtime = runtime();
    let (bastion, target) = runtime.block_on(two_hosts(false));

    let cache = Arc::new(CredentialCache::new());
    // 负控之一：不经跳板直连那个名字 —— RFC 2606 保证它解析不出来。
    let direct = SshConnection::connect_via(
        runtime.handle(),
        Vec::new(),
        target_options(&target, Arc::clone(&cache)),
    );
    assert!(
        matches!(direct, Err(SshError::Connect { .. })),
        "本机直连那个名字应当失败"
    );

    // 负控之二：经跳板 —— 它收到请求并**拒绝**（中继表里没有那一条）。
    let err = SshConnection::connect_via(
        runtime.handle(),
        vec![connect_options(
            &bastion,
            USER,
            SshAuth::keys(Vec::new()),
            Arc::clone(&cache),
            Arc::new(CountingProvider::new(BASTION_PASSWORD)),
        )],
        target_options(&target, Arc::clone(&cache)),
    )
    .err()
    .expect("跳板没认那条转发时不该建得起链");
    match err {
        SshError::Forward { host, port, .. } => {
            assert_eq!(host, ONLY_NAME);
            assert_eq!(port, ONLY_PORT);
        }
        other => panic!("应当是 Forward（跳板拒绝转发），得到 {other:?}"),
    }

    // 服务端那一半：那条请求确实到过跳板，而且它没有中继任何字节。
    assert_eq!(
        bastion.shared.observed().direct_tcpip.len(),
        1,
        "跳板应当收到那条请求"
    );
    assert_eq!(bastion.shared.relayed_bytes(), 0, "被拒绝的转发不该搬字节");
}

/// **A 档**（回退）：本机分别连两台，字节经本机内存中转 —— 同一个引擎，同一条判据。
#[test]
fn the_relay_route_moves_the_file_when_the_tunnel_is_unavailable() {
    let runtime = runtime();
    // 跳板这次没有中继表：A 档本来就不需要它（那条路是"本机分别连两台"）。
    let (bastion, target) = runtime.block_on(two_hosts(false));
    let cache = Arc::new(CredentialCache::new());

    let source_conn = SshConnection::connect_via(
        runtime.handle(),
        Vec::new(),
        connect_options(
            &bastion,
            USER,
            SshAuth::keys(Vec::new()),
            Arc::clone(&cache),
            Arc::new(CountingProvider::new(BASTION_PASSWORD)),
        ),
    )
    .expect("直连源那一台失败");
    let target_conn = SshConnection::connect_via(
        runtime.handle(),
        Vec::new(),
        connect_options(
            &target,
            USER,
            SshAuth::keys(Vec::new()),
            Arc::clone(&cache),
            Arc::new(CountingProvider::new(TARGET_PASSWORD)),
        ),
    )
    .expect("直连目标那一台失败");

    runtime.block_on(async {
        let source = source_conn.sftp().await.expect("源那一台的 SFTP 建不起来");
        let sink = target_conn
            .sftp()
            .await
            .expect("目标那一台的 SFTP 建不起来");
        transfer(
            &source,
            &sink,
            &request("/from.bin", "/inbox/relayed.bin"),
            &Progress::default(),
            Cancel::new().waiter(),
        )
        .await
        .expect("A 档的传输应当成功");
    });

    let target_root = target.sftp_root().unwrap();
    assert_eq!(
        std::fs::read(inbox(target_root).join("relayed.bin")).unwrap(),
        payload(),
        "目标盘上的字节必须与源相同"
    );
    assert_eq!(
        names(&inbox(target_root)),
        vec!["relayed.bin".to_owned()],
        "目标目录里不该多出临时名"
    );
    assert_eq!(
        names(bastion.sftp_root().unwrap()),
        vec!["from.bin".to_owned()],
        "源那一台的目录不该有任何变化"
    );
    // A 档**不经**跳板的中继：那条路是"本机分别连两台"，一次转发都不该有。
    assert!(
        bastion.shared.observed().direct_tcpip.is_empty(),
        "A 档不该向任何一台要转发"
    );
}
