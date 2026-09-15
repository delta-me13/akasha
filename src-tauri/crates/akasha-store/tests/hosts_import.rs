//! **把解析出来的条目落进池**（plan 0506 的第二步）。
//!
//! 解析那一半的判据在 [`sshconfig_parse`](../../../crates/akasha-store/tests/sshconfig_parse.rs)；
//! 这里盯的是**写**：
//!
//! | 盯什么 | 为什么 |
//! |---|---|
//! | 跳板链真的挂上了（且 `jump_chain` 读得出来） | plan 0505 刚做到的那条路要能**从导入的配置**走通 |
//! | 同名默认不动、`overwrite` 才替换 | 重导一次不该毁掉用户在池里手改过的东西 |
//! | **补建的跳板条目永不覆盖** | 一条 `ProxyJump` 附带的推断不该盖掉用户写的那一行 |
//! | 成环时**一行都不写**（事务回滚） | 半批数据比没有数据难查得多 |

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod common;

use akasha_store::pools::hosts::{self, Auth, NewHost};
use akasha_store::pools::import::{Reason, import_hosts};
use akasha_store::sshconfig::{Target, parse};
use common::new_vault;

/// 一份最普通的配置：跳板（直连）+ 目标（经跳板）。
const TWO_HOSTS: &str = "\
Host bastion\n\
\x20   HostName 10.0.0.1\n\
Host target\n\
\x20   HostName target.internal\n\
\x20   ProxyJump bastion\n";

fn rows(conn: &akasha_store::Connection) -> Vec<hosts::Host> {
    hosts::hosts(conn).unwrap()
}

fn find<'a>(rows: &'a [hosts::Host], name: &str) -> &'a hosts::Host {
    rows.iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("池里没有 {name}：{rows:#?}"))
}

#[test]
fn a_parsed_config_lands_in_the_pool_with_its_jump_chain() {
    let (_dir, conn) = new_vault("import-lands");
    let imported = parse(TWO_HOSTS, "me").unwrap();

    let outcome = import_hosts(&conn, &imported.targets, false).unwrap();
    assert_eq!(
        outcome
            .created
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["bastion", "target"]
    );
    assert!(outcome.skipped.is_empty());

    let rows = rows(&conn);
    let bastion = find(&rows, "bastion");
    let target = find(&rows, "target");
    assert_eq!(target.jump_id, Some(bastion.id));
    assert_eq!(bastion.jump_id, None);
    assert_eq!(target.host, "target.internal");

    // 认证一律 `publickey` + 不带钥匙（= 走 agent）：私钥文件不导入，而"公钥认证 + 钥匙在
    // agent 里"正是池允许的一种状态。
    assert_eq!(target.auth, Auth::PublicKey);
    assert_eq!(target.key_id, None);

    // 连接那条路要的是"从目标往回走"的链 —— 导入出来的行必须能被它读出来。
    let chain = hosts::jump_chain(&conn, target.id).unwrap();
    assert_eq!(
        chain
            .iter()
            .map(|host| host.name.as_str())
            .collect::<Vec<_>>(),
        ["target", "bastion"]
    );
}

#[test]
fn the_same_name_is_left_alone_until_overwrite_is_asked_for() {
    let (_dir, conn) = new_vault("import-existing");
    let first = parse(TWO_HOSTS, "me").unwrap();
    import_hosts(&conn, &first.targets, false).unwrap();

    // 用户在池里手改过这一行（改的是导入管不着的那一列：认证方式）。
    let target_id = find(&rows(&conn), "target").id;
    let mut edited = find(&rows(&conn), "target").clone();
    edited.user = "handwritten".to_owned();
    hosts::update_host(&conn, &edited).unwrap();

    // 配置换了端口，再导一次：默认**不动**已经有的行。
    let changed = parse(&TWO_HOSTS.replace("10.0.0.1", "10.0.0.9"), "me").unwrap();
    let second = import_hosts(&conn, &changed.targets, false).unwrap();
    assert_eq!(
        second.skipped,
        [
            ("bastion".to_owned(), Reason::Exists),
            ("target".to_owned(), Reason::Exists)
        ]
    );
    assert!(second.created.is_empty() && second.updated.is_empty());
    assert_eq!(find(&rows(&conn), "bastion").host, "10.0.0.1", "没被改");
    assert_eq!(
        find(&rows(&conn), "target").user,
        "handwritten",
        "手改的还在"
    );

    // `overwrite`：整行替换成配置里的样子。
    let third = import_hosts(&conn, &changed.targets, true).unwrap();
    assert_eq!(
        third
            .updated
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["bastion", "target"]
    );
    assert_eq!(find(&rows(&conn), "bastion").host, "10.0.0.9");
    assert_eq!(find(&rows(&conn), "target").user, "me", "替换就是照配置来");
    assert_eq!(find(&rows(&conn), "target").id, target_id, "替换保留行 id");
    assert_eq!(
        find(&rows(&conn), "target").jump_id,
        Some(find(&rows(&conn), "bastion").id),
        "替换之后跳板链还在"
    );
}

#[test]
fn a_built_jump_host_never_overwrites_a_row_you_wrote() {
    let (_dir, conn) = new_vault("import-fallback");
    // 用户自己写的一行：端口不是 22、用户名也不是配置里的那个。
    let mine = hosts::insert_host(
        &conn,
        &NewHost {
            name: "bastion".to_owned(),
            host: "bastion.corp".to_owned(),
            port: 2222,
            user: "ops".to_owned(),
            auth: Auth::Password,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();

    // 配置里只写了"目标经 bastion 出去"，`bastion` 自己没有 `Host` 块 —— 解析器会补建一条。
    let imported = parse(
        "Host target\n  HostName target.internal\n  ProxyJump bastion\n",
        "me",
    )
    .unwrap();
    assert!(imported.targets.iter().any(|target| target.provisional));

    let outcome = import_hosts(&conn, &imported.targets, true).unwrap();
    assert_eq!(
        outcome.skipped,
        [("bastion".to_owned(), Reason::Fallback)],
        "**即使开了 overwrite**，补建的条目也不覆盖用户写的行"
    );

    let rows = rows(&conn);
    let bastion = find(&rows, "bastion");
    assert_eq!(bastion.id, mine);
    assert_eq!(bastion.host, "bastion.corp");
    assert_eq!(bastion.port, 2222);
    assert_eq!(bastion.user, "ops");
    assert_eq!(bastion.auth, Auth::Password);
    assert_eq!(
        find(&rows, "target").jump_id,
        Some(mine),
        "跳板用的是池里那一行"
    );
}

// ── `IdentityFile` 与池里的钥匙（plan 0903）──────────────────────────────────

/// 池里有一把**同名**的钥匙时，`IdentityFile` 把条目接到它上面。
///
/// 判据是"逐字符同名"，所以这一份同时钉住两半：**同名接得上**、**不同名接不上**
/// （后者与 plan 0506 的行为逐字相同 —— 钥匙在 ssh-agent 里）。
#[test]
fn an_identity_file_links_to_a_pool_key_of_the_same_name() {
    use akasha_store::pools::keys;

    let (_dir, conn) = new_vault("import-key-link");
    let mut pem =
        keys::PrivateKey::new(b"-----BEGIN OPENSSH PRIVATE KEY-----\nlink\n".to_vec()).unwrap();
    let key_id = keys::insert_key(
        &conn,
        &keys::NewKey {
            name: "id_ed25519".to_owned(),
            public_key: "ssh-ed25519 AAAA".to_owned(),
            comment: None,
        },
        &mut pem,
    )
    .unwrap();

    let imported = parse(
        "\
Host linked\n\
\x20   HostName linked.internal\n\
\x20   IdentityFile ~/.ssh/id_ed25519\n\
Host unlinked\n\
\x20   HostName unlinked.internal\n\
\x20   IdentityFile ~/.ssh/other_key\n\
Host bare\n\
\x20   HostName bare.internal\n",
        "me",
    )
    .unwrap();

    let outcome = import_hosts(&conn, &imported.targets, false).unwrap();
    let rows = rows(&conn);
    assert_eq!(
        find(&rows, "linked").key_id,
        Some(key_id),
        "basename 与池里的名字相同就该接上"
    );
    assert_eq!(
        find(&rows, "unlinked").key_id,
        None,
        "不同名不许猜 —— 退回「钥匙在 agent 里」"
    );
    assert_eq!(
        find(&rows, "bare").key_id,
        None,
        "没有 IdentityFile 的条目不受影响"
    );
    assert_eq!(
        outcome.linked,
        vec![("linked".to_owned(), "id_ed25519".to_owned())],
        "报告里要说清接上了哪一条"
    );
}

/// 成环的配置**连解析器都过不去**（那是解析器的判据）；这里手工造一批带环的条目，
/// 盯的是**存储层自己那一道**：它必须拒绝，而且**一行都不能留下**。
#[test]
fn a_cycle_handed_straight_to_the_store_is_refused_and_nothing_is_written() {
    let (_dir, conn) = new_vault("import-cycle");
    let targets = vec![
        Target {
            name: "a".to_owned(),
            host: "a.internal".to_owned(),
            port: 22,
            user: "me".to_owned(),
            key_file: None,
            jump: Some("b".to_owned()),
            provisional: false,
        },
        Target {
            name: "b".to_owned(),
            host: "b.internal".to_owned(),
            port: 22,
            user: "me".to_owned(),
            key_file: None,
            jump: Some("a".to_owned()),
            provisional: false,
        },
    ];

    let refused = import_hosts(&conn, &targets, false);
    assert!(
        matches!(refused, Err(akasha_store::StoreError::JumpChain)),
        "成环该被存储层拒绝，实际：{refused:?}"
    );
    assert!(
        rows(&conn).is_empty(),
        "失败要整批回滚 —— 半批数据比没有数据难查得多：{:#?}",
        rows(&conn)
    );
}

#[test]
fn two_hosts_that_jump_to_the_same_name_share_one_built_row() {
    let (_dir, conn) = new_vault("import-shared-jump");
    let imported = parse(
        "\
Host one\n\
\x20   HostName one.internal\n\
\x20   ProxyJump jump.example.com\n\
Host two\n\
\x20   HostName two.internal\n\
\x20   ProxyJump jump.example.com\n",
        "me",
    )
    .unwrap();

    let outcome = import_hosts(&conn, &imported.targets, false).unwrap();
    assert_eq!(
        outcome
            .created
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["jump.example.com", "one", "two"],
        "同名跳板只补建一次"
    );
    let rows = rows(&conn);
    let jump = find(&rows, "jump.example.com").id;
    assert_eq!(find(&rows, "one").jump_id, Some(jump));
    assert_eq!(find(&rows, "two").jump_id, Some(jump));
}
