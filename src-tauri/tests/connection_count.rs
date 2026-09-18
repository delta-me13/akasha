//! plan 0606 的库内验收上半：**连接计数**跟着连接的生命周期走。
//!
//! 判据（ROADMAP 原文）=「关闭转发 `Session` 后**连接数与重连任务数**都归零」。这条用例看着
//! 它的下半句在 crate 层能不能成立：`live_connections()` 数的是本进程持有着的
//! [`SshConnection`]，**持有即计入、析构即减掉**。
//!
//! 为什么这件事要单独验：app 那条判据靠这个数说"连接没了"，而实体表不能充当证据 ——
//! 关闭命令自己就会把实体摘掉。这个数错了，`residue` 探针（同一个来源）就会跟着说谎。
//!
//! ⚠️ 反过来，计数**不能**证明 socket 真的关了：它数的是我们手上的对象。所以真实 app 上的
//! 判据还有另一半 —— 对端（测试服务端）看不到那条连接了。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "ssh_support/mod.rs"]
mod support;

use std::sync::Arc;

use akasha_lib::ssh::{CredentialCache, SshAuth, SshConnection, live_connections};
use support::{CountingProvider, ServerOptions, connect_options, start};

/// 一条连接一被持有就上账，一被丢掉就下账；两条各自计数。
#[tokio::test]
async fn a_connection_is_counted_while_it_is_held_and_not_after() {
    let server = start(ServerOptions::password("counter-password")).await;
    assert_eq!(live_connections(), 0, "一条都还没连，账上就该是空的");

    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new("counter-password"));

    let first = SshConnection::connect(&mut connect_options(
        &server,
        "cyrene",
        SshAuth::agent_only(),
        Arc::clone(&cache),
        Arc::clone(&provider),
    ))
    .await
    .expect("第一条连接失败");

    assert_eq!(live_connections(), 1, "握着一条连接，账上就该有一条");

    let second = SshConnection::connect(&mut connect_options(
        &server,
        "cyrene",
        SshAuth::agent_only(),
        cache,
        provider,
    ))
    .await
    .expect("第二条连接失败");

    // 不复用连接（D5）在这个数上看得见：两条就是两条。
    assert_eq!(live_connections(), 2, "两条连接该各算一条");

    drop(second);
    assert_eq!(live_connections(), 1, "丢掉一条，账上只剩一条");
    drop(first);
    assert_eq!(live_connections(), 0, "最后一条也丢掉了，账上就该空了");
}
