//! **known_hosts 缓存**（plan 0503）：读写回环 + "记住"这条路**不许静默改写**。
//!
//! 这一份的重心不是 CRUD 本身，而是**写路径上的一条安全约束**（ADR-0003 D11）：
//! 主机密钥变化是中间人攻击的典型形态，而它会伪装成"顺手更新一下缓存"。
//! 所以"同一个 `(host, port, key_type)` 上已经记着别的密钥"这件事在库这一层就是**拒绝**，
//! 而不是靠每个调用方自觉先比对。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "store_common/mod.rs"]
mod common;

use akasha_lib::store::StoreError;
use akasha_lib::store::known_hosts::{self, NewKnownHost};
use common::new_vault;

/// 一把假的主机密钥（内容不重要，重要的是**逐字节**可比）。
fn key(seed: u8) -> Vec<u8> {
    (0..32).map(|i| seed.wrapping_add(i)).collect()
}

fn record(host: &str, port: u16, key_type: &str, seed: u8) -> NewKnownHost {
    NewKnownHost {
        host: host.to_owned(),
        port,
        key_type: key_type.to_owned(),
        key_blob: key(seed),
        fingerprint: format!("SHA256:seed{seed}"),
    }
}

#[test]
fn a_recorded_key_comes_back_and_can_be_forgotten() {
    let (_dir, conn) = new_vault("known-hosts-roundtrip");

    let id = known_hosts::remember(&conn, &record("example.com", 22, "ssh-ed25519", 1)).unwrap();
    assert_eq!(known_hosts::known_hosts(&conn).unwrap().len(), 1);

    let found = known_hosts::lookup(&conn, "example.com", 22, "ssh-ed25519")
        .unwrap()
        .expect("刚记下的密钥应当查得到");
    assert_eq!(found.id, id);
    assert_eq!(found.key_blob, key(1), "判定材料要原样回来");
    assert_eq!(found.fingerprint, "SHA256:seed1");
    assert_eq!(known_hosts::known_host(&conn, id).unwrap(), found);

    known_hosts::forget(&conn, id).unwrap();
    assert!(known_hosts::known_hosts(&conn).unwrap().is_empty());

    // 再删一次：那一项已经不在了，调用方点的是**某一项**，必须说出来。
    assert!(matches!(
        known_hosts::forget(&conn, id),
        Err(StoreError::NoSuchRow {
            pool: "known_hosts",
            ..
        })
    ));
}

#[test]
fn recording_the_same_key_again_is_idempotent() {
    let (_dir, conn) = new_vault("known-hosts-idempotent");

    let first = known_hosts::remember(&conn, &record("example.com", 22, "ssh-ed25519", 7)).unwrap();
    let again = known_hosts::remember(&conn, &record("example.com", 22, "ssh-ed25519", 7)).unwrap();

    assert_eq!(first, again, "同一把密钥再确认一次应当返回原来那一行");
    assert_eq!(
        known_hosts::known_hosts(&conn).unwrap().len(),
        1,
        "也不该多出一行"
    );
}

#[test]
fn recording_a_different_key_for_the_same_host_is_refused() {
    let (_dir, conn) = new_vault("known-hosts-no-silent-rewrite");

    known_hosts::remember(&conn, &record("example.com", 22, "ssh-ed25519", 1)).unwrap();
    let err =
        known_hosts::remember(&conn, &record("example.com", 22, "ssh-ed25519", 2)).unwrap_err();

    assert!(
        matches!(
            &err,
            StoreError::Conflict {
                pool: "known_hosts",
                ..
            }
        ),
        "同一主机同一类型已经记着别的密钥 → 拒绝（D11：不静默改写）：{err:?}"
    );
    let kept = known_hosts::lookup(&conn, "example.com", 22, "ssh-ed25519")
        .unwrap()
        .unwrap();
    assert_eq!(kept.key_blob, key(1), "库里那一把必须还是原来那把");

    // 要走"接受新密钥"那条路，得先显式忘掉它 —— 这是让用户看见"你正在丢掉一把旧密钥"的时刻。
    known_hosts::forget_host(&conn, "example.com", 22).unwrap();
    known_hosts::remember(&conn, &record("example.com", 22, "ssh-ed25519", 2)).unwrap();
    assert_eq!(
        known_hosts::lookup(&conn, "example.com", 22, "ssh-ed25519")
            .unwrap()
            .unwrap()
            .key_blob,
        key(2)
    );
}

#[test]
fn a_different_key_type_is_not_the_same_record() {
    let (_dir, conn) = new_vault("known-hosts-key-types");

    known_hosts::remember(&conn, &record("example.com", 22, "ssh-ed25519", 1)).unwrap();

    // 换一种算法：这不是"密钥变了"，而是"这一类型没见过" —— 与上游
    // `check_known_hosts_path` 的语义一致（类型不同不算不匹配），所以查得到是 `None`（未知）。
    assert!(
        known_hosts::lookup(&conn, "example.com", 22, "rsa-sha2-512")
            .unwrap()
            .is_none()
    );
    // 同一类型、不同端口也是另一条记录（`host:port` 才是身份）。
    assert!(
        known_hosts::lookup(&conn, "example.com", 2222, "ssh-ed25519")
            .unwrap()
            .is_none()
    );

    // 不同类型可以各记一行。
    known_hosts::remember(&conn, &record("example.com", 22, "rsa-sha2-512", 3)).unwrap();
    assert_eq!(known_hosts::known_hosts(&conn).unwrap().len(), 2);

    // 忘了这台主机 = 忘了它的**全部**类型。
    assert_eq!(
        known_hosts::forget_host(&conn, "example.com", 22).unwrap(),
        2
    );
    assert_eq!(
        known_hosts::forget_host(&conn, "example.com", 22).unwrap(),
        0
    );
}

#[test]
fn clearing_drops_everything_and_touches_nothing_else() {
    let (_dir, conn) = new_vault("known-hosts-clear");
    known_hosts::remember(&conn, &record("a.example.com", 22, "ssh-ed25519", 1)).unwrap();
    known_hosts::remember(&conn, &record("b.example.com", 22, "ssh-ed25519", 2)).unwrap();

    // 四套池里的东西不受影响：清缓存不是清库。
    akasha_lib::store::hosts::insert_host(
        &conn,
        &akasha_lib::store::hosts::NewHost {
            name: "web".into(),
            host: "example.com".into(),
            port: 22,
            user: "root".into(),
            auth: akasha_lib::store::hosts::Auth::Agent,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();

    assert_eq!(known_hosts::clear(&conn).unwrap(), 2);
    assert!(known_hosts::known_hosts(&conn).unwrap().is_empty());
    assert_eq!(akasha_lib::store::hosts::hosts(&conn).unwrap().len(), 1);
}

#[test]
fn the_database_refuses_impossible_rows() {
    let (_dir, conn) = new_vault("known-hosts-constraints");

    let err =
        known_hosts::remember(&conn, &record("example.com", 0, "ssh-ed25519", 1)).unwrap_err();
    assert!(
        matches!(
            err,
            StoreError::Conflict {
                pool: "known_hosts",
                ..
            }
        ),
        "端口 0 不是端口（下界由库的 CHECK 拦）：{err:?}"
    );

    // 同一 `(host, port, key_type)` 的两行：**绕过 API** 也写不进去 —— 不变量在库里，
    // 不在"我们记得检查"的地方。
    let raw = conn.execute(
        "INSERT INTO known_hosts (host, port, key_type, key_blob, fingerprint)
         VALUES ('example.com', 22, 'ssh-ed25519', x'00', 'SHA256:x')",
        [],
    );
    assert!(raw.is_ok(), "前提：第一条能写进去");
    let raw = conn.execute(
        "INSERT INTO known_hosts (host, port, key_type, key_blob, fingerprint)
         VALUES ('example.com', 22, 'ssh-ed25519', x'01', 'SHA256:y')",
        [],
    );
    assert!(raw.is_err(), "UNIQUE (host, port, key_type) 没生效");
}
