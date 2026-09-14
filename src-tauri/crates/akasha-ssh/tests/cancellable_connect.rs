//! plan 0606 的库内验收下半：**这次连接尝试可以被中途叫停**。
//!
//! 判据（ROADMAP 原文）=「关闭转发 `Session` 后连接数与重连任务数都归零」里的那一半
//! "立刻断连"。这条用例验的是它最底下那颗螺丝：`SshConnection::connect_via_until` 的
//! `cancel` 一就绪，**握手当场结束、socket 当场关掉**（`SshError::Cancelled`）。
//!
//! ⚠️ 为什么必须有第二个入口（而不是"扔掉 await 就完了"）：app 那条路把这次连接建在
//! **阻塞线程**上（`spawn_sync`），而丢掉 `JoinHandle` **取消不了**阻塞任务 —— 它会跑到底
//! （最长一个 `connect_timeout`），那条 socket 也就一直开着。判据里"关闭 `Session`"的
//! "立刻"正是被这件事卡住的（plan 0606 的 E2E 实测：修之前 3 秒内看不到 EOF）。
//!
//! ## 这里的"对端"是什么
//!
//! 一个**接了 TCP 就不再说话**的监听：客户端的握手会停在"等对端 banner"上，于是"这次尝试
//! 结束了吗"只可能由**取消**解释，不可能由"对端先失败"或"超时"解释（那两者都要 5 秒，
//! 而取消给的是 200 毫秒）。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use akasha_ssh::{CredentialCache, PinnedHostKey, SshAuth, SshConnection, SshError, SshTarget};
use support::{CountingProvider, connect_options_to};

/// 每个用例自己建一个 runtime（ADR-0003 D2：库**不**自建 runtime，由调用方给 `Handle`）。
///
/// 用 `#[test]` 而不是 `#[tokio::test]`：同步门面内部走 `Handle::block_on`，而它在 runtime
/// 线程上会 panic —— 测试线程必须**不在** runtime 上下文里（同 `connect_auth` 的说明）。
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// 一个"接了就不再说话"的监听：返回它的端口与"读到 EOF 的时刻"的收端。
fn silent_listener() -> (u16, std::sync::mpsc::Receiver<Instant>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut socket, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 256];
        // 只管读，什么都不回：读到 0（EOF）= 对端把那条 socket 关了。
        while let Ok(read) = socket.read(&mut buf) {
            if read == 0 {
                break;
            }
        }
        let _ = tx.send(Instant::now());
    });
    (port, rx)
}

/// 客户端参数：连那个静默端口（主机密钥策略在这儿用不上 —— 握手到不了那一步）。
fn options(port: u16) -> akasha_ssh::SshConnect {
    connect_options_to(
        SshTarget::new("127.0.0.1", port, "cyrene"),
        Arc::new(PinnedHostKey::new("这次握手到不了这一步")),
        SshAuth::agent_only(),
        Arc::new(CredentialCache::new()),
        Arc::new(CountingProvider::new("用不上")),
        Duration::from_secs(5),
    )
}

/// 取消一就绪，那次尝试立刻结束（`Cancelled`），而且**它对端看得见那条 socket 已经关了**。
#[test]
fn a_cancelled_attempt_ends_at_once_and_closes_its_socket() {
    let (port, closed) = silent_listener();
    let runtime = runtime();
    let cancel = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    let started = Instant::now();
    let outcome =
        SshConnection::connect_via_until(runtime.handle(), Vec::new(), options(port), cancel);
    let err = match outcome {
        Ok(_) => panic!("这次握手停在对端一声不吭上，本该被取消"),
        Err(err) => err,
    };
    let elapsed = started.elapsed();

    assert!(
        matches!(err, SshError::Cancelled),
        "取消要报成「中止」而不是一次连接失败：{err}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "取消在 200 毫秒时就该生效，实际用了 {elapsed:?}（超过 5 秒就是根本没被取消）"
    );

    // **另一半**：不是"函数返回了"就算完，那条 socket 也得真的关掉 —— 对端读到 EOF 才算。
    let at = closed
        .recv_timeout(Duration::from_secs(2))
        .expect("对端没读到 EOF：这次尝试结束了，但它建的 socket 还开着");
    let lag = at.duration_since(started);
    assert!(
        lag < Duration::from_secs(2),
        "对端在 {lag:?} 之后才看到 EOF —— 太晚了"
    );
}
