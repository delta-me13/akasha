//! plan 0701 的库内验收：**SFTP 会话**（ADR-0006 D2）。
//!
//! 判据（ROADMAP 原文）=「直接打开 SFTP 即可用」。这条用例盯着它的**库那一半**：
//! 会话建得起来、列得出对端给的条目、并且**对端没开 SFTP 时会明确失败**（不是干等）。
//!
//! | 断言 | 在哪 |
//! |---|---|
//! | 会话能建起来 | `connection.sftp()` 返回句柄 |
//! | 列目录拿到对端的条目 | 名字与类型与服务端给的一致（按名字排序） |
//! | 路径是 `realpath` 的结果 | `listing.path == "/"`（服务端 `realpath` 的固定回答） |
//! | 对端记到一次子系统请求 | `observed().sftp_subsystems == 1` |
//! | **对端没开 SFTP → 明确失败** | 负例：`sftp_with_timeout(1)` 在期限附近失败，错误带原因 |
//!
//! ⚠️ 与 E2E（`src-tauri/tests/sftp_dual_pane.rs`）的分工：这里验的是**库这条链**
//! （通道、子系统、列目录），那边验的是**app 的接线**（两栏、两侧独立、会话归属）。

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use akasha_ssh::testing::{ServerOptions, SftpItem, start};
use akasha_ssh::{CredentialCache, SftpKind, SshAuth, SshConnection, SshError};
use support::{CountingProvider, connect_options};

/// 登录口令（测试服务端与客户端约定的那一句）。
const PASSWORD: &str = "sftp-session-password";
const USER: &str = "cyrene";

#[tokio::test]
async fn sftp_lists_what_the_server_offers() {
    let mut options = ServerOptions::password(PASSWORD);
    options.sftp = Some(vec![SftpItem::file("alpha.txt"), SftpItem::dir("beta")]);
    let server = start(options).await;

    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new(PASSWORD));
    // 口令认证：不用 agent、不带私钥（与 app 侧 `Auth::Password` 那一条一样）。
    let mut connect = connect_options(&server, USER, SshAuth::keys(Vec::new()), cache, provider);
    let connection = SshConnection::connect(&mut connect)
        .await
        .expect("连接测试服务端失败");

    let client = connection.sftp().await.expect("SFTP 会话应当建得起来");
    let listing = client.list(".").await.expect("列目录失败");

    assert_eq!(
        listing.path, "/",
        "路径应当是服务端 `realpath` 给出的那个（不是我们送进去的 `.`）"
    );
    let names: Vec<&str> = listing
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["alpha.txt", "beta"],
        "条目应当与对端给的一致，并按名字排序（对端给的顺序是随机的）"
    );
    let kind = |name: &str| {
        listing
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.kind)
    };
    assert_eq!(kind("alpha.txt"), Some(SftpKind::File));
    assert_eq!(kind("beta"), Some(SftpKind::Directory));

    assert_eq!(
        server.shared.observed().sftp_subsystems,
        1,
        "对端应当被请求过一次 sftp 子系统"
    );

    connection.disconnect().await;
}

#[tokio::test]
async fn a_server_without_sftp_fails_with_a_reason() {
    // 对端**不提供** sftp（`options.sftp = None`）—— 客户端能看到的唯一现象就是
    // "等不到第一条回复"，所以这条用例盯的是：它会不会在**期限之内**失败，且原因说得清。
    let server = start(ServerOptions::password(PASSWORD)).await;

    let cache = Arc::new(CredentialCache::new());
    // 口令按**同一句**给：这条用例要的是"连上了、但子系统开不出来"，不是认证失败。
    let provider = Arc::new(CountingProvider::new(PASSWORD));
    let mut connect = connect_options(&server, USER, SshAuth::keys(Vec::new()), cache, provider);
    let connection = SshConnection::connect(&mut connect)
        .await
        .expect("连接本身应当成功（服务端只是没开 sftp）");

    let started = Instant::now();
    // ⚠️ 不用 `expect_err`：`SftpClient` 没有 `Debug`（它内部是上游的会话），
    // 而那个方法要求成功那一侧也能打出来。这里只需要失败那一侧。
    let failure = match connection.sftp_with_timeout(1).await {
        Ok(_) => panic!("对端没开 sftp，会话不该建得起来"),
        Err(err) => err,
    };
    let waited = started.elapsed();

    assert!(
        matches!(failure, SshError::Sftp { .. }),
        "失败应当是 SFTP 那一档（用户要看的是对端有没有开 SFTP）：{failure:?}"
    );
    let message = failure.to_string();
    assert!(
        message.contains("sftp 子系统"),
        "错误消息该说清是子系统没被认下：{message}"
    );
    assert!(
        waited < Duration::from_secs(5),
        "它应当在我们给的期限（1 s）附近失败，而不是一直等：等了 {waited:?}"
    );
    assert_eq!(
        server.shared.observed().sftp_subsystems,
        0,
        "被拒的那一次不该记成「认下了」"
    );

    connection.disconnect().await;
}
