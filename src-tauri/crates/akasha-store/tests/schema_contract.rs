//! **v1 的形状**（plan 0403）：四张表、每张表的列、外键真的生效、`CHECK` 真的拦得住。
//!
//! 这一份守的是"库自己拦下来的不变量"（ADR-0002 D7 里那句"`user_version` 是唯一的格式
//! 权威"的落地）。它和别的契约测试一样有个共同的对手：**静默失效** ——
//! 外键写了但 `PRAGMA foreign_keys` 没开、`CHECK` 写了但被拼错的列名引用、
//! 表建了但 `open` 不看它。这几件事都不会报错，只会让库比以为的更松。
//!
//! ⚠️ 其中**形状快照**那条（[`the_shape_of_v1_is_pinned`]）红的时候不要顺手改期望值：
//! 改列 = 换格式，那要 `FORMAT_VERSION + 1` 加迁移（ADR-0002 D7）。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod common;

use akasha_store::{StoreError, TABLES, VaultState, create, open, vault_path};
use common::{PASSPHRASE, columns_of, fixture_dir, new_vault, pass};

// ── 1. STRICT 表的前提：SQLite 得够新 ────────────────────────────────────────

#[test]
fn sqlite_is_new_enough_for_strict_tables() {
    let (_dir, conn) = new_vault("sqlite-version");
    let version: String = conn
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .unwrap();
    let mut parts = version.split('.');
    let major: u32 = parts.next().unwrap().parse().unwrap();
    let minor: u32 = parts.next().unwrap().parse().unwrap();
    println!("SQLite {version}（SQLCipher 4.5.7 内嵌）");
    assert!(
        (major, minor) >= (3, 37),
        "`schema.rs` 用的是 STRICT 表（SQLite ≥ 3.37）—— 这个版本不认它，\
         而它不认的表现是建表语句直接报错，不是静默降级：{version}"
    );
}

// ── 2. 四张表与它们的列：v1 的形状 ──────────────────────────────────────────

#[test]
fn the_four_tables_are_the_ones_v1_promises() {
    assert_eq!(
        TABLES,
        ["keys", "hosts", "serials", "forwards"],
        "清单与顺序都是有意的：顺序就是建表顺序（外键的目标要先存在）"
    );

    let (_dir, conn) = new_vault("schema-tables");
    for table in TABLES {
        let columns = columns_of(&conn, table);
        assert!(!columns.is_empty(), "{table} 不存在");
    }
}

/// **v1 的形状快照。** 它红了意味着"格式变了"，而不是"测试过时了"：
/// 加列 / 改列 / 删列都要 `FORMAT_VERSION + 1` 并给出迁移（ADR-0002 D7），
/// 否则老库会被新代码读成一个"能开但形状不对"的东西。
#[test]
fn the_shape_of_v1_is_pinned() {
    let (_dir, conn) = new_vault("schema-shape");

    let expected: [(&str, &[&str]); 4] = [
        (
            "keys",
            &["id", "name", "private_pem", "public_key", "comment"],
        ),
        (
            "hosts",
            &[
                "id", "name", "host", "port", "user", "auth", "key_id", "jump_id",
            ],
        ),
        (
            "serials",
            &[
                "id",
                "name",
                "port",
                "baud",
                "data_bits",
                "stop_bits",
                "parity",
                "flow",
            ],
        ),
        (
            "forwards",
            &[
                "id",
                "name",
                "direction",
                "bind_host",
                "bind_port",
                "target_host",
                "target_port",
                "host_id",
                "autostart",
            ],
        ),
    ];

    for (table, columns) in expected {
        assert_eq!(
            columns_of(&conn, table),
            columns,
            "{table} 的列变了 —— 这不是「改一下期望值」的事：\
             改列等于换格式，要 `FORMAT_VERSION + 1` 加迁移（ADR-0002 D7）"
        );
    }
}

// ── 3. 外键：声明 ≠ 生效 ────────────────────────────────────────────────────

#[test]
fn foreign_keys_are_actually_on() {
    let (_dir, conn) = new_vault("foreign-keys-on");
    let on: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        on, 1,
        "`PRAGMA foreign_keys` 是连接级且**默认关闭**的：DDL 里写了 REFERENCES 也不生效，\
         而不生效的表现是「删掉还在被引用的密钥会成功」"
    );
}

#[test]
fn a_key_still_used_by_a_host_cannot_be_deleted() {
    use akasha_store::{hosts, keys};

    let (_dir, conn) = new_vault("fk-key-in-use");

    let mut pem =
        keys::PrivateKey::new(b"-----BEGIN OPENSSH PRIVATE KEY-----\nfk\n".to_vec()).unwrap();
    let key_id = keys::insert_key(
        &conn,
        &keys::NewKey {
            name: "work".into(),
            public_key: "ssh-ed25519 AAAA".into(),
            comment: None,
        },
        &mut pem,
    )
    .unwrap();

    hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "web".into(),
            host: "example.com".into(),
            port: 22,
            user: "root".into(),
            auth: hosts::Auth::PublicKey,
            key_id: Some(key_id),
            jump_id: None,
        },
    )
    .unwrap();

    let err = keys::delete_key(&conn, key_id).unwrap_err();
    assert!(
        matches!(&err, StoreError::Conflict { pool: "keys", .. }),
        "还在被 host 引用的密钥应当删不掉（ON DELETE RESTRICT），实际：{err:?}"
    );
    assert_eq!(keys::keys(&conn).unwrap().len(), 1, "也不该被删掉");
}

#[test]
fn a_host_with_rules_cannot_be_deleted() {
    use akasha_store::{forwards, hosts};

    let (_dir, conn) = new_vault("fk-host-in-use");
    let host_id = insert_plain_host(&conn, "web");

    forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: "web-db".into(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".into(),
            bind_port: 5432,
            target_host: Some("db.internal".into()),
            target_port: Some(5432),
            host_id,
            autostart: true,
        },
    )
    .unwrap();

    let err = hosts::delete_host(&conn, host_id).unwrap_err();
    assert!(
        matches!(&err, StoreError::Conflict { pool: "hosts", .. }),
        "还有转发规则挂在它身上，删不掉才对：{err:?}"
    );
}

// ── 4. CHECK：库自己拦下的"不可能的行" ──────────────────────────────────────

#[test]
fn check_constraints_reject_impossible_rows() {
    use akasha_store::{forwards, hosts, keys, serial};

    let (_dir, conn) = new_vault("check-constraints");

    // 重名：两个看起来一样的主机、行为取决于顺序 —— 顺序没有地方能看见，所以不许。
    let first = insert_plain_host(&conn, "web");
    let err = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "web".into(),
            host: "other.example.com".into(),
            port: 22,
            user: "root".into(),
            auth: hosts::Auth::Agent,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict { pool: "hosts", .. }),
        "{err:?}"
    );

    // 跳板指向自己：**写入路径先发现**（走链的检查比库早一步），所以这里是 `JumpChain`
    // 而不是 `Conflict`。库那一层也拦得住 —— 见下面那条绕开 API 的断言。
    let err = hosts::update_host(
        &conn,
        &hosts::Host {
            id: first,
            name: "web".into(),
            host: "example.com".into(),
            port: 22,
            user: "root".into(),
            auth: hosts::Auth::Agent,
            key_id: None,
            jump_id: Some(first),
        },
    )
    .unwrap_err();
    assert!(matches!(err, StoreError::JumpChain), "自环：{err:?}");

    // 同一件事走**原始 SQL**（绕过上面那层检查）：库的 `CHECK` 照样拦。
    // 这条断言的意义不在"防得住手改库"，而在于**不变量确实写在库里** ——
    // 将来多一条写入路径（导入、dump 恢复）时它自动也在保护之下。
    let raw = conn.execute(
        "INSERT INTO hosts (name, host, port, user, auth, jump_id) VALUES ('self', 'h', 22, 'u', 'agent', last_insert_rowid() + 1)",
        [],
    );
    assert!(
        raw.is_err(),
        "自环竟然被写进去了 —— 说明 CHECK (jump_id <> id) 没生效"
    );

    // 认证方式是 password，却配了一把密钥
    let mut pem = keys::PrivateKey::new(b"pem".to_vec()).unwrap();
    let key_id = keys::insert_key(
        &conn,
        &keys::NewKey {
            name: "k".into(),
            public_key: String::new(),
            comment: None,
        },
        &mut pem,
    )
    .unwrap();
    let err = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "bad".into(),
            host: "example.com".into(),
            port: 22,
            user: "root".into(),
            auth: hosts::Auth::Password,
            key_id: Some(key_id),
            jump_id: None,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict { pool: "hosts", .. }),
        "password 认证不该带 key_id：{err:?}"
    );

    // 端口：**下界由库拦（`CHECK port BETWEEN 1 AND 65535`）、上界由类型拦**（`u16` 里
    // 根本没有 65536 这个值）。两条边各由一层守，正好说明"约束该放哪一层"不是随意的。
    let err = insert_port(&conn, "port-zero", 0).unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict { pool: "hosts", .. }),
        "端口 0 不是端口：{err:?}"
    );
    insert_port(&conn, "port-max", 65535).expect("65535 是合法端口，上界是闭区间");

    // serial：数据位只有 5/6/7/8
    let err = serial::insert_serial(
        &conn,
        &serial::NewSerial {
            name: "tty".into(),
            port: "/dev/ttyUSB0".into(),
            baud: 115200,
            data_bits: 9,
            stop_bits: 1,
            parity: serial::Parity::None,
            flow: serial::Flow::None,
        },
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            StoreError::Conflict {
                pool: "serials",
                ..
            }
        ),
        "{err:?}"
    );

    // forwards：dynamic 不能有目标
    let err = forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: "bad-socks".into(),
            direction: forwards::Direction::Dynamic,
            bind_host: "127.0.0.1".into(),
            bind_port: 1080,
            target_host: Some("db.internal".into()),
            target_port: Some(5432),
            host_id: first,
            autostart: false,
        },
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
        "dynamic（SOCKS5）没有目标：{err:?}"
    );

    // local 又必须有目标
    let err = forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: "no-target".into(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".into(),
            bind_port: 5432,
            target_host: None,
            target_port: None,
            host_id: first,
            autostart: false,
        },
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
        "local 必须有目标：{err:?}"
    );
}

// ── 5. `open` 不只看版本号，还看表在不在 ────────────────────────────────────

#[test]
fn a_vault_with_the_right_version_but_no_tables_is_refused() {
    let dir = fixture_dir("missing-tables");
    let db = vault_path(&dir);

    {
        let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
        // 只掉一张：`user_version` 还是 1，所以只有"查表"这一步能发现。
        conn.execute_batch("DROP TABLE forwards").unwrap();
    }

    let err = open(&db, &mut pass(PASSPHRASE)).unwrap_err();
    assert!(
        matches!(&err, StoreError::MissingTable { table: "forwards" }),
        "版本号对而表不全，必须明确拒绝 —— 否则它就是一个「能开但内容不对」的库：{err:?}"
    );

    // 对照：表全的库当然能开 —— 上面那条不是"现在什么都开不了"。
    let (_dir, conn) = new_vault("missing-tables-control");
    assert_eq!(columns_of(&conn, "forwards").len(), 9);
}

#[test]
fn a_version_we_do_not_know_is_refused_before_the_tables_are_checked() {
    // v2 的库（版本号 +1）要报版本错，而不是报"缺表"：
    // 将来加表时，先说话的是版本 —— 否则用户会以为自己的库坏了。
    let dir = fixture_dir("version-before-tables");
    let db = vault_path(&dir);
    {
        let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
    }
    let err = open(&db, &mut pass(PASSPHRASE)).unwrap_err();
    assert!(
        matches!(err, StoreError::UnsupportedVersion { found: 2 }),
        "{err:?}"
    );
}

// ── 6. `vault_state`：三种状态，不需要口令 ──────────────────────────────────

#[test]
fn vault_state_is_missing_empty_or_present() {
    let dir = fixture_dir("vault-state");
    let db = vault_path(&dir);

    assert_eq!(
        akasha_store::vault_state(&db).unwrap(),
        VaultState::Missing,
        "还没建过"
    );

    std::fs::write(&db, b"").unwrap();
    assert_eq!(
        akasha_store::vault_state(&db).unwrap(),
        VaultState::Empty,
        "0 字节 = 还没有密钥落在那里（这种文件用什么口令都能「打开」）"
    );

    {
        let _conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
    }
    assert_eq!(
        akasha_store::vault_state(&db).unwrap(),
        VaultState::Present,
        "有内容 —— 能不能打开是 `open` 的事"
    );

    // `vault_state` 全程没有口令参数：连 `user_version` 都在加密的第一页里，
    // 不开库就读不到 —— 所以"能不开库说出来的"只有这三个状态。
}

/// 插一台指定端口的主机（只用于端口边界那两条断言）。
fn insert_port(conn: &rusqlite::Connection, name: &str, port: u16) -> Result<i64, StoreError> {
    akasha_store::hosts::insert_host(
        conn,
        &akasha_store::hosts::NewHost {
            name: name.into(),
            host: "example.com".into(),
            port,
            user: "root".into(),
            auth: akasha_store::hosts::Auth::Agent,
            key_id: None,
            jump_id: None,
        },
    )
}

/// 一台最普通的 agent 认证主机。
fn insert_plain_host(conn: &rusqlite::Connection, name: &str) -> i64 {
    akasha_store::hosts::insert_host(
        conn,
        &akasha_store::hosts::NewHost {
            name: name.into(),
            host: "example.com".into(),
            port: 22,
            user: "root".into(),
            auth: akasha_store::hosts::Auth::Agent,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap()
}
