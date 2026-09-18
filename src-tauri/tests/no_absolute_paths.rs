//! **P2 第 2 条**："库里不存绝对路径"（[`docs/portable.md`](../../../../docs/portable.md) §2）。
//!
//! 为什么这条必须写成测试：它**不体现在目录结构上，而体现在数据模型里**。
//! 一旦有人加一个"密钥文件路径"字段，功能照样跑通、单测照样绿 ——
//! 直到用户把整个文件夹搬到 U 盘上，那些路径才**静默失效**（而那时数据看起来"还在"）。
//!
//! 两条判据，一条比一条宽：
//!
//! 1. **没有任何列名像位置**（`path` / `dir` / `file` / `folder` / `root`）；
//! 2. **没有任何值提到我们的数据目录**，也没有值长得像绝对路径 —— 只有一个**声明的例外**
//!    （serial 的设备名，理由见 [`akasha_lib::store::serial`] 的模块文档）。
//!
//! 判据 2 覆盖的是"任何位置"，不是"我们记得检查的那几列"：表与列的清单都是**现场枚举**的
//! （`sqlite_master` + `PRAGMA table_info`），所以**将来加的列自动进入检查范围**。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "store_common/mod.rs"]
mod common;

use akasha_lib::store::{TABLES, forwards, hosts, keys, serial};
use common::{all_values, columns_of, new_vault};

/// 出现在列名里的这些子串 = "这一列存的是某个文件在哪"。
const LOCATION_WORDS: [&str; 5] = ["path", "dir", "file", "folder", "root"];

/// **唯一的例外**：serial 的 `port` 是操作系统给的设备名（`/dev/ttyUSB0` / `COM3`），
/// 不是我们的文件位置 —— 换台机器它可能不存在，但那不是"搬家之后才失效"。
///
/// ⚠️ 这条清单**只许在明确裁定之后变长**：新加一列进来之前要回答"搬走这个文件夹之后，
/// 它还有效吗"。答不上来就不该是例外。
const ABSOLUTE_PATH_EXCEPTIONS: [(&str, &str); 1] = [("serials", "port")];

/// 一列的名字看起来是不是"某个文件在哪"。
///
/// ⚠️ 规则是**按 `_` 分词、整词比较（或整词结尾）**，不是子串包含 —— 这不是讲究，
/// 是负例逼出来的：`forwards.direction` 含子串 `dir`，而它显然不是路径
/// （见 [`the_column_rule_tells_direction_from_a_path`]）。判据分不清这两者的后果很具体：
/// 下一个被它拦下的人第一反应是把整条检查删掉。
fn location_word(column: &str) -> Option<&'static str> {
    for token in column.to_ascii_lowercase().split('_').map(str::to_owned) {
        for word in LOCATION_WORDS {
            if token == word || token.ends_with(word) {
                return Some(word);
            }
        }
    }
    None
}

#[test]
fn no_column_is_named_like_a_location() {
    let (_dir, conn) = new_vault("no-paths-columns");

    for table in TABLES {
        for column in columns_of(&conn, table) {
            if let Some(word) = location_word(&column) {
                panic!(
                    "{table}.{column} 看起来是「某个文件在哪」（命中 {word:?}）—— P2 要求这类\
                     东西要么存相对标识、要么干脆存进库本身（密钥就是这么做的）。\
                     如果这一列真的不是位置，换一个名字；如果它是位置，先读 portable.md §2"
                );
            }
        }
    }
}

/// 规则自己的负例：一个该命中的、一个**诱饵**（必须不命中）。
///
/// `AGENTS.md` §6 对规则的要求是"用一对负例验证过"才算落地 —— 这条判据是测试里的规则，
/// 一样要能说清"它在工作"而不是"它什么都没匹配到"。
#[test]
fn the_column_rule_tells_direction_from_a_path() {
    assert_eq!(location_word("direction"), None, "诱饵：direction 不是 dir");
    assert_eq!(location_word("bind_host"), None);
    assert_eq!(location_word("private_pem"), None);
    assert_eq!(location_word("key_path"), Some("path"));
    assert_eq!(location_word("identity_file"), Some("file"));
    assert_eq!(location_word("datadir"), Some("dir"), "整词结尾也算");
    assert_eq!(location_word("root_dir_name"), Some("root"));
}

#[test]
fn no_stored_value_mentions_our_data_directory_or_looks_like_an_absolute_path() {
    let (dir, conn) = new_vault("no-paths-values");

    // 四套池各来一行：判据要在**真有数据**的库上成立，空库上它自动为真、也就没有意义。
    let mut pem =
        keys::PrivateKey::new(b"-----BEGIN OPENSSH PRIVATE KEY-----\npath-probe\n".to_vec())
            .unwrap();
    let key_id = keys::insert_key(
        &conn,
        &keys::NewKey {
            name: "path-probe".into(),
            public_key: "ssh-ed25519 AAAA path-probe".into(),
            comment: Some("/home/nobody/secret".into()),
        },
        &mut pem,
    )
    .unwrap();

    let host_id = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "path-probe".into(),
            host: "example.com".into(),
            port: 22,
            user: "root".into(),
            auth: hosts::Auth::PublicKey,
            key_id: Some(key_id),
            jump_id: None,
        },
    )
    .unwrap();

    serial::insert_serial(
        &conn,
        &serial::NewSerial {
            name: "console".into(),
            port: "/dev/ttyUSB0".into(),
            baud: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: serial::Parity::None,
            flow: serial::Flow::None,
        },
    )
    .unwrap();

    forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: "path-probe".into(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".into(),
            bind_port: 15432,
            target_host: Some("db.internal".into()),
            target_port: Some(5432),
            host_id,
            autostart: false,
        },
    )
    .unwrap();

    let data_dir = dir.to_string_lossy().into_owned();
    let values = all_values(&conn);
    assert!(
        values.len() > 15,
        "枚举出来的值太少（{}），说明取数那一步没走通",
        values.len()
    );

    let mut absolute = Vec::new();
    for (table, column, value) in &values {
        // ① 我们的数据目录：绝对不许出现。这是"搬家之后静默失效"的**唯一**来路。
        assert!(
            !value.contains(&data_dir),
            "{table}.{column} 里存了数据目录 {data_dir}：搬走文件夹之后它就不成立了"
        );

        // ② 绝对路径本身：默认也不许（例外见上）。`/home/nobody/secret` 那条注释是**故意的**
        // —— 它证明这条检查看得到值里的路径，而不是"什么都没匹配到所以全绿"。
        if value.starts_with('/')
            && !ABSOLUTE_PATH_EXCEPTIONS.contains(&(table.as_str(), column.as_str()))
        {
            absolute.push(format!("{table}.{column} = {value}"));
        }
    }
    assert_eq!(
        absolute,
        ["keys.comment = /home/nobody/secret"],
        "除了声明的例外（serial 的设备名），库里不该有绝对路径"
    );

    // 例外是**被声明**的，不是"被跳过所以看不见"：设备名确实在检查范围里。
    assert!(
        values
            .iter()
            .any(|(table, column, value)| table == "serials"
                && column == "port"
                && value == "/dev/ttyUSB0"),
        "取数那一层应当看得见 serial 的设备名 —— 它只是被声明成了例外"
    );
}

/// 判据 2 的**负例**：它必须真的能看见"看起来像路径"的值，否则上面那条永远绿。
///
/// 这一条把"诱饵"单独拎出来断言，而不是在上一条里靠脑子推 ——
/// `AGENTS.md` §6 对规则的要求是"必须用负例验证过"，而这两条判据就是规则。
#[test]
fn the_absolute_path_check_can_actually_see_a_path() {
    let (_dir, conn) = new_vault("no-paths-negative");

    let mut pem = keys::PrivateKey::new(b"pem".to_vec()).unwrap();
    keys::insert_key(
        &conn,
        &keys::NewKey {
            name: "decoy".into(),
            public_key: String::new(),
            comment: Some("/home/lycurgus/.ssh/id_ed25519".into()),
        },
        &mut pem,
    )
    .unwrap();

    let hits: Vec<String> = all_values(&conn)
        .into_iter()
        .filter(|(_, _, value)| value.starts_with('/'))
        .map(|(table, column, value)| format!("{table}.{column} = {value}"))
        .collect();

    assert_eq!(
        hits,
        ["keys.comment = /home/lycurgus/.ssh/id_ed25519"],
        "取数那一层必须看得见绝对路径 —— 看不见的话，上一条判据只是「什么都没匹配到」"
    );
}

/// **新加的写入路径也要守 P2**：导入 `~/.ssh/config`（plan 0506）时最容易漏出去的就是
/// `IdentityFile` —— 它天然是一个绝对路径，而且用户写得理直气壮。
///
/// 判据分两半：解析器**看见了**那条路径（报告里有它），而库里**一个值都没有提到它**。
/// 只断言后一半是不够的：一个"根本没读懂这行"的实现也照样让后一半成立。
#[test]
fn importing_a_config_does_not_leave_the_key_file_path_behind() {
    let (_dir, conn) = new_vault("no-paths-import");

    const KEY_FILE: &str = "/home/nobody/.ssh/id_ed25519";
    let config = format!("Host work\n  HostName work.example\n  IdentityFile {KEY_FILE}\n");
    let imported = akasha_lib::store::sshconfig::parse(&config, "me").unwrap();
    assert_eq!(
        imported.targets[0].key_file.as_deref(),
        Some(KEY_FILE),
        "解析器该读到这个密钥文件（报告里要说清它没有导入）"
    );
    akasha_lib::store::pools::import::import_hosts(&conn, &imported.targets, false).unwrap();

    let values = all_values(&conn);
    assert!(
        values.iter().any(|(table, _, _)| table == "hosts"),
        "池里该有导入进来的那一行：{values:?}"
    );
    assert!(
        !values.iter().any(|(_, _, value)| value.contains(KEY_FILE)),
        "密钥文件的路径不该进库（它只出现在报告里）：{values:?}"
    );
}
