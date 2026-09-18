//! **四套池的 round-trip**（plan 0403 的判据本身）：写入 → 读出 → 字段一致。
//!
//! 每一套池都按同一套动作走一遍：建行 → 读回逐字段比 → 改 → 再读回 → 删 → 确认不在了。
//! 只比"插进去了没有"是不够的：漏掉一个字段的写入（比如 `autostart` 忘了进 SQL）在
//! "插进去了"这条判据下照样绿。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "store_common/mod.rs"]
mod common;

use akasha_lib::store::{StoreError, forwards, hosts, keys, serial};
use common::new_vault;

/// 一把一眼能认出来的"私钥"。
const PEM: &[u8] = b"-----BEGIN OPENSSH PRIVATE KEY-----\nakasha-pool-fixture\n-----END OPENSSH PRIVATE KEY-----\n";

fn private(pem: &[u8]) -> keys::PrivateKey {
    keys::PrivateKey::new(pem.to_vec()).unwrap()
}

// ── 密钥池 ──────────────────────────────────────────────────────────────────

#[test]
fn key_round_trips_including_the_private_bytes() {
    let (_dir, conn) = new_vault("keys-roundtrip");

    let mut pem = private(PEM);
    let id = keys::insert_key(
        &conn,
        &keys::NewKey {
            name: "work".into(),
            public_key: "ssh-ed25519 AAAAC3Nza work".into(),
            comment: Some("公司的钥匙".into()),
        },
        &mut pem,
    )
    .unwrap();

    let stored = keys::key(&conn, id).unwrap();
    assert_eq!(
        stored,
        keys::Key {
            id,
            name: "work".into(),
            public_key: "ssh-ed25519 AAAAC3Nza work".into(),
            comment: Some("公司的钥匙".into()),
        }
    );

    // 私钥**逐字节**回来（PEM 是文本，但它照旧按字节存/取：口令与密钥都不假设 UTF-8）
    let mut read = keys::private_key(&conn, id).unwrap();
    assert_eq!(read.byte_len(), PEM.len(), "长度也是字段之一");
    assert_eq!(&*read.expose().unwrap(), PEM);

    // 换私钥
    let mut replacement = private(b"-----BEGIN OPENSSH PRIVATE KEY-----\nsecond\n");
    keys::set_private_key(&conn, id, &mut replacement).unwrap();
    let mut read = keys::private_key(&conn, id).unwrap();
    assert_eq!(
        &*read.expose().unwrap(),
        b"-----BEGIN OPENSSH PRIVATE KEY-----\nsecond\n"
    );

    // 改元数据（不含私钥）
    keys::update_key(
        &conn,
        &keys::Key {
            id,
            name: "work-2".into(),
            public_key: "ssh-ed25519 AAAAC3Nza work2".into(),
            comment: None,
        },
    )
    .unwrap();
    assert_eq!(keys::key(&conn, id).unwrap().name, "work-2");
    assert_eq!(keys::key(&conn, id).unwrap().comment, None);

    // 删
    keys::delete_key(&conn, id).unwrap();
    assert!(matches!(
        keys::key(&conn, id),
        Err(StoreError::NoSuchRow { pool: "keys", .. })
    ));
    assert!(matches!(
        keys::private_key(&conn, id),
        Err(StoreError::NoSuchRow { pool: "keys", .. })
    ));
    assert!(matches!(
        keys::delete_key(&conn, id),
        Err(StoreError::NoSuchRow { pool: "keys", .. })
    ));
}

#[test]
fn an_empty_or_oversized_private_key_never_reaches_the_database() {
    // 空：一把"看起来有、其实没有"的钥匙，在**构造**这一层就拒绝。
    assert!(matches!(
        keys::PrivateKey::new(Vec::new()),
        Err(StoreError::EmptyPrivateKey)
    ));

    // 超页：同上，理由见 `pools/keys.rs` 的模块文档（库里不许出现读不出来的行）。
    match keys::PrivateKey::new(vec![b'x'; keys::MAX_PEM_LEN + 1]) {
        Err(StoreError::SecretTooLong { max }) => assert_eq!(max, keys::MAX_PEM_LEN),
        other => panic!("超过一页的私钥不该造得出来：{:?}", other.is_ok()),
    }

    // ⚠️ 这里写不出 `.unwrap_err()` 那种紧凑写法：`PrivateKey` 没有 `Debug`
    // （与 `Passphrase` 同一条理由）—— 那个编译错误本身就是"密钥打不进日志"的证据。
}

#[test]
fn a_key_knows_which_hosts_use_it() {
    let (_dir, conn) = new_vault("keys-reverse-lookup");

    let mut pem = private(PEM);
    let key_id = keys::insert_key(
        &conn,
        &keys::NewKey {
            name: "shared".into(),
            public_key: String::new(),
            comment: None,
        },
        &mut pem,
    )
    .unwrap();

    for name in ["beta", "alpha"] {
        hosts::insert_host(
            &conn,
            &hosts::NewHost {
                name: name.into(),
                host: format!("{name}.example.com"),
                port: 22,
                user: "root".into(),
                auth: hosts::Auth::PublicKey,
                key_id: Some(key_id),
                jump_id: None,
            },
        )
        .unwrap();
    }
    // 一台不用这把钥匙的：它不该出现在结果里
    hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "other".into(),
            host: "other.example.com".into(),
            port: 22,
            user: "root".into(),
            auth: hosts::Auth::Agent,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();

    let using: Vec<String> = keys::hosts_using_key(&conn, key_id)
        .unwrap()
        .into_iter()
        .map(|host| host.name)
        .collect();
    assert_eq!(using, ["alpha", "beta"], "按名字排序，顺序确定");
}

// ── ssh 配置池 ──────────────────────────────────────────────────────────────

fn new_host(name: &str, jump_id: Option<i64>) -> hosts::NewHost {
    hosts::NewHost {
        name: name.into(),
        host: format!("{name}.example.com"),
        port: 2200,
        user: "deploy".into(),
        auth: hosts::Auth::Agent,
        key_id: None,
        jump_id,
    }
}

#[test]
fn host_round_trips_and_the_lists_are_ordered() {
    let (_dir, conn) = new_vault("hosts-roundtrip");

    let bastion = hosts::insert_host(&conn, &new_host("bastion", None)).unwrap();
    let web = hosts::insert_host(&conn, &new_host("web", Some(bastion))).unwrap();

    assert_eq!(
        hosts::host(&conn, web).unwrap(),
        hosts::Host {
            id: web,
            name: "web".into(),
            host: "web.example.com".into(),
            port: 2200,
            user: "deploy".into(),
            auth: hosts::Auth::Agent,
            key_id: None,
            jump_id: Some(bastion),
        }
    );

    // 反查：谁把我当跳板
    let jumping = hosts::hosts_jumping_to(&conn, bastion).unwrap();
    assert_eq!(jumping.len(), 1);
    assert_eq!(jumping[0].id, web);

    // 改：把 web 的跳板摘掉、换认证方式（全量替换）
    let mut updated = hosts::host(&conn, web).unwrap();
    updated.jump_id = None;
    updated.auth = hosts::Auth::Password;
    updated.port = 22;
    hosts::update_host(&conn, &updated).unwrap();
    let read_back = hosts::host(&conn, web).unwrap();
    assert_eq!(read_back.jump_id, None);
    assert_eq!(read_back.auth, hosts::Auth::Password);
    assert_eq!(read_back.port, 22);
    assert!(hosts::hosts_jumping_to(&conn, bastion).unwrap().is_empty());

    // 列表按名字排序
    hosts::insert_host(&conn, &new_host("alpha", None)).unwrap();
    let names: Vec<String> = hosts::hosts(&conn)
        .unwrap()
        .into_iter()
        .map(|host| host.name)
        .collect();
    assert_eq!(names, ["alpha", "bastion", "web"]);

    // 删
    hosts::delete_host(&conn, web).unwrap();
    assert!(matches!(
        hosts::host(&conn, web),
        Err(StoreError::NoSuchRow { pool: "hosts", .. })
    ));
}

#[test]
fn a_jump_chain_may_not_close_into_a_cycle() {
    let (_dir, conn) = new_vault("hosts-jump-cycle");

    let a = hosts::insert_host(&conn, &new_host("a", None)).unwrap();
    let b = hosts::insert_host(&conn, &new_host("b", Some(a))).unwrap();
    let c = hosts::insert_host(&conn, &new_host("c", Some(b))).unwrap();

    // a → c → b → a：链已经存在（c→b→a），把 a 的跳板改成 c 就成环了。
    let mut host_a = hosts::host(&conn, a).unwrap();
    host_a.jump_id = Some(c);
    let err = hosts::update_host(&conn, &host_a).unwrap_err();
    assert!(
        matches!(err, StoreError::JumpChain),
        "成环的跳板链要在写入时报错，而不是等到连接时绕圈：{err:?}"
    );
    assert_eq!(
        hosts::host(&conn, a).unwrap().jump_id,
        None,
        "报错之后那一行不该被改动"
    );

    // 自环也走同一条路
    host_a.jump_id = Some(a);
    assert!(matches!(
        hosts::update_host(&conn, &host_a),
        Err(StoreError::JumpChain)
    ));
}

/// 整条链读得出来，而且**目标在前**（plan 0505 的连接路径就吃这个顺序）。
#[test]
fn a_jump_chain_reads_from_the_target_back_to_the_bastion() {
    let (_dir, conn) = new_vault("hosts-jump-chain");

    let outer = hosts::insert_host(&conn, &new_host("outer", None)).unwrap();
    let middle = hosts::insert_host(&conn, &new_host("middle", Some(outer))).unwrap();
    let inner = hosts::insert_host(&conn, &new_host("inner", Some(middle))).unwrap();

    let chain = hosts::jump_chain(&conn, inner).unwrap();
    let names: Vec<&str> = chain.iter().map(|host| host.name.as_str()).collect();
    assert_eq!(
        names,
        ["inner", "middle", "outer"],
        "第一项是**要连的那台**，最后一项是最外层（app 直接连的那台）"
    );

    // 没有跳板的主机：链就是它自己一项（"直连"也是一种链，只是长度为 1）。
    let alone = hosts::jump_chain(&conn, outer).unwrap();
    assert_eq!(alone.len(), 1);
    assert_eq!(alone[0].id, outer);
}

/// 读路径**自己也**挡成环与过深：写入路径挡不住有人手工改库，而链上真有环时连接会**挂住**
/// （不是报错，是"点了没反应"）。这里用 SQL 直接造一个环 —— 那正是要防的那种库。
#[test]
fn a_cycle_that_was_smuggled_into_the_file_is_caught_on_read() {
    let (_dir, conn) = new_vault("hosts-jump-smuggled-cycle");

    let a = hosts::insert_host(&conn, &new_host("a", None)).unwrap();
    let b = hosts::insert_host(&conn, &new_host("b", Some(a))).unwrap();
    // 绕过 `update_host`（它会拒绝）：手工把 a 的跳板接到 b 上。
    conn.execute("UPDATE hosts SET jump_id = ?1 WHERE id = ?2", [b, a])
        .unwrap();

    let err = hosts::jump_chain(&conn, b).unwrap_err();
    assert!(
        matches!(err, StoreError::JumpChain),
        "读路径也要拦住成环：{err:?}"
    );
}

#[test]
fn a_jump_chain_deeper_than_the_limit_is_refused() {
    let (_dir, conn) = new_vault("hosts-jump-too-deep");

    // 造一条比上限更长的链（每台都是上一台的跳板）。
    let mut previous: Option<i64> = None;
    let mut last = 0;
    for index in 0..40 {
        last = hosts::insert_host(&conn, &new_host(&format!("hop{index:02}"), previous)).unwrap();
        previous = Some(last);
    }

    let err = hosts::jump_chain(&conn, last).unwrap_err();
    assert!(
        matches!(err, StoreError::JumpChain),
        "深得离谱的链要报错而不是一路连下去：{err:?}"
    );
}

// ── serial 配置池 ───────────────────────────────────────────────────────────

#[test]
fn serial_round_trips_every_field() {
    let (_dir, conn) = new_vault("serials-roundtrip");

    let id = serial::insert_serial(
        &conn,
        &serial::NewSerial {
            name: "router console".into(),
            port: "/dev/ttyUSB0".into(),
            baud: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: serial::Parity::None,
            flow: serial::Flow::Hardware,
        },
    )
    .unwrap();

    assert_eq!(
        serial::serial(&conn, id).unwrap(),
        serial::Serial {
            id,
            name: "router console".into(),
            port: "/dev/ttyUSB0".into(),
            baud: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: serial::Parity::None,
            flow: serial::Flow::Hardware,
        }
    );

    // 每个枚举取值都能进库、也能出库（DDL 的 CHECK 与 `ALL` 是同一份取值）
    for (index, parity) in serial::Parity::ALL.into_iter().enumerate() {
        for flow in serial::Flow::ALL {
            let name = format!("p{index}-{flow:?}");
            let id = serial::insert_serial(
                &conn,
                &serial::NewSerial {
                    name,
                    port: "COM3".into(),
                    baud: 9600,
                    data_bits: 7,
                    stop_bits: 2,
                    parity,
                    flow,
                },
            )
            .unwrap();
            let stored = serial::serial(&conn, id).unwrap();
            assert_eq!(stored.parity, parity);
            assert_eq!(stored.flow, flow);
        }
    }

    // 改 + 删
    let mut stored = serial::serial(&conn, id).unwrap();
    stored.baud = 9600;
    stored.parity = serial::Parity::Even;
    serial::update_serial(&conn, &stored).unwrap();
    assert_eq!(serial::serial(&conn, id).unwrap().baud, 9600);
    assert_eq!(
        serial::serial(&conn, id).unwrap().parity,
        serial::Parity::Even
    );

    serial::delete_serial(&conn, id).unwrap();
    assert!(matches!(
        serial::serial(&conn, id),
        Err(StoreError::NoSuchRow {
            pool: "serials",
            ..
        })
    ));
}

// ── 端口转发规则池 ──────────────────────────────────────────────────────────

fn new_forward(name: &str, host_id: i64, direction: forwards::Direction) -> forwards::NewForward {
    let target = match direction {
        forwards::Direction::Dynamic => (None, None),
        _ => (Some("db.internal".to_string()), Some(5432)),
    };
    forwards::NewForward {
        name: name.into(),
        direction,
        bind_host: "127.0.0.1".into(),
        bind_port: 15432,
        target_host: target.0,
        target_port: target.1,
        host_id,
        autostart: true,
    }
}

#[test]
fn forward_round_trips_in_all_three_directions() {
    let (_dir, conn) = new_vault("forwards-roundtrip");

    let host_id = hosts::insert_host(&conn, &new_host("web", None)).unwrap();

    for direction in forwards::Direction::ALL {
        let id =
            forwards::insert_forward(&conn, &new_forward(direction.as_str(), host_id, direction))
                .unwrap();
        let stored = forwards::forward(&conn, id).unwrap();
        assert_eq!(stored.direction, direction);
        assert_eq!(stored.host_id, host_id);
        assert!(stored.autostart, "`autostart` 是最容易漏进 SQL 的字段之一");
        assert_eq!(stored.bind_host, "127.0.0.1");
        assert_eq!(stored.bind_port, 15432);
        match direction {
            forwards::Direction::Dynamic => {
                assert_eq!(stored.target_host, None);
                assert_eq!(stored.target_port, None);
            }
            _ => {
                assert_eq!(stored.target_host.as_deref(), Some("db.internal"));
                assert_eq!(stored.target_port, Some(5432));
            }
        }
    }

    let mine = forwards::forwards_of_host(&conn, host_id).unwrap();
    assert_eq!(mine.len(), 3);
    assert_eq!(
        mine.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
        ["dynamic", "local", "remote"],
        "按名字排序：dynamic < local < remote"
    );

    // 改：关掉自启、换绑定端口
    let mut stored = forwards::forward(&conn, mine[0].id).unwrap();
    stored.autostart = false;
    stored.bind_port = 1080;
    forwards::update_forward(&conn, &stored).unwrap();
    let read_back = forwards::forward(&conn, mine[0].id).unwrap();
    assert!(!read_back.autostart);
    assert_eq!(read_back.bind_port, 1080);

    // 删
    forwards::delete_forward(&conn, read_back.id).unwrap();
    assert!(matches!(
        forwards::forward(&conn, read_back.id),
        Err(StoreError::NoSuchRow {
            pool: "forwards",
            ..
        })
    ));
    assert_eq!(forwards::forwards(&conn).unwrap().len(), 2);
}

#[test]
fn a_forward_needs_an_existing_host() {
    let (_dir, conn) = new_vault("forwards-dangling-host");

    let err = forwards::insert_forward(
        &conn,
        &new_forward("orphan", 999, forwards::Direction::Local),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            StoreError::Conflict {
                pool: "forwards",
                ..
            }
        ),
        "规则必须属于一台真实存在的主机（外键）：{err:?}"
    );
}
