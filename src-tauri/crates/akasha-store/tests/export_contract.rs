//! **dump / 导出 / 还原的契约**（plan 0404 的判据本身）。
//!
//! 三条主线：
//!
//! 1. **导出件就是库**（ADR-0002 D6）：不还原也能用导出口令 `open`，内容与源**逐字段一致**；
//! 2. **明文导出有门槛**（D6）：短语不对 / 名字不自曝 → 目标路径上**什么都没有**，
//!    而门槛的另一面也要证明它真的拦住了东西 —— 通过之后写出来的确实是明文（裸 sqlite
//!    读得到私钥）；
//! 3. **不泄密**：加密导出件里 grep 不到明文私钥。**对照组**是明文件（grep 得到）——
//!    少了它，"没搜到"就分不清是"真没泄"还是"扫描器本来就搜不到东西"。
//!
//! fixture 落在 `target/store-pools/`（与其它契约测试同一处，**故意不删**）：
//! "导出件里没有明文私钥"是一条**安全声明**，要能拿一个真实文件手工 `grep` 复核。
//!
//! ⚠️ 本文件里的检验函数（`contains` / `partial_of` / `mode_of`）是**判据的一部分**：
//! 它们自己也各有一个"对照组"，否则一条永远返回 `false` 的检查会让判据全绿。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod common;

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use akasha_store::dump;
use akasha_store::export::{self, PLAINTEXT_CONFIRMATION, PlaintextAck};
use akasha_store::{
    FORMAT_VERSION, StoreError, create, forwards, hosts, keys, open, serial, vault_path,
};
use common::{PASSPHRASE, fixture_dir, new_vault, pass};
use rusqlite::Connection;

/// 一把一眼能认出来的"私钥"：判据是"这个字符串在不在文件里"，所以要认得出来。
const PEM: &[u8] = b"-----BEGIN OPENSSH PRIVATE KEY-----\nakasha-export-fixture\n-----END OPENSSH PRIVATE KEY-----\n";

/// 导出件自己的口令 —— **与库口令不同**是硬要求（D6）。
const EXPORT_PASSPHRASE: &[u8] = b"a passphrase that lives with the export";
/// 还原出来的那个库的口令。
const RESTORED_PASSPHRASE: &[u8] = b"the passphrase of the restored vault";

// ── 脚手架 ──────────────────────────────────────────────────────────────────

/// 一个写满四套池的库：1 把密钥 / 3 台主机（含一条跳板链）/ 1 个串口 / 2 条转发
/// （local 带目标 + dynamic 不带目标）—— 每条 DDL 不变量都真有数据压着。
fn populated(name: &str) -> (PathBuf, Connection) {
    let dir = fixture_dir(name);
    let conn = create(&vault_path(&dir), &mut pass(PASSPHRASE)).unwrap();

    let mut pem = keys::PrivateKey::new(PEM.to_vec()).unwrap();
    let key_id = keys::insert_key(
        &conn,
        &keys::NewKey {
            name: "work".into(),
            public_key: "ssh-ed25519 AAAAC3Nza work".into(),
            comment: Some("公司的钥匙".into()),
        },
        &mut pem,
    )
    .unwrap();

    let bastion = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "bastion".into(),
            host: "bastion.example".into(),
            port: 22,
            user: "jump".into(),
            auth: hosts::Auth::Agent,
            key_id: None,
            jump_id: None,
        },
    )
    .unwrap();
    let web = hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "web".into(),
            host: "10.0.0.5".into(),
            port: 2222,
            user: "root".into(),
            auth: hosts::Auth::PublicKey,
            key_id: Some(key_id),
            jump_id: None,
        },
    )
    .unwrap();
    hosts::insert_host(
        &conn,
        &hosts::NewHost {
            name: "db".into(),
            host: "10.0.0.9".into(),
            port: 22,
            user: "postgres".into(),
            auth: hosts::Auth::PublicKey,
            key_id: Some(key_id),
            jump_id: Some(bastion),
        },
    )
    .unwrap();

    serial::insert_serial(
        &conn,
        &serial::NewSerial {
            name: "board".into(),
            // serial 的 `port` 是**操作系统给的设备名**，不是我们的文件位置（P2 的例外，见 schema.rs）
            port: "/dev/ttyUSB0".into(),
            baud: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: serial::Parity::None,
            flow: serial::Flow::Hardware,
        },
    )
    .unwrap();

    forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: "web-console".into(),
            direction: forwards::Direction::Local,
            bind_host: "127.0.0.1".into(),
            bind_port: 8080,
            target_host: Some("10.0.0.5".into()),
            target_port: Some(80),
            host_id: web,
            autostart: true,
        },
    )
    .unwrap();
    forwards::insert_forward(
        &conn,
        &forwards::NewForward {
            name: "proxy".into(),
            direction: forwards::Direction::Dynamic,
            bind_host: "127.0.0.1".into(),
            bind_port: 1080,
            target_host: None,
            target_port: None,
            host_id: web,
            autostart: false,
        },
    )
    .unwrap();

    (dir, conn)
}

/// 把 `source` 加密导出到一个**独立的新目录**（判据说的是"另一目录"，不是同目录改个名）。
fn export_into_fresh_dir(dir: &Path, name: &str, source: &Connection) -> PathBuf {
    let out_dir = dir.join(name);
    fs::create_dir_all(&out_dir).unwrap();
    let file = out_dir.join("akasha-export.db");
    export::to_encrypted(
        source,
        &mut pass(PASSPHRASE),
        &file,
        &mut pass(EXPORT_PASSPHRASE),
    )
    .unwrap();
    file
}

/// 子串查找（`[u8]::contains` 拿不到，而"文件里有没有这段字节"是这条判据的全部内容）。
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// 半成品的路径：与 `export.rs` 里的规则一致（`<目标>.partial`）。
fn partial_of(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".partial");
    PathBuf::from(name)
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

// ── 1 / 2 / 9：加密导出 → 还原 → 逐字段一致 ─────────────────────────────────

#[test]
fn an_encrypted_export_restores_into_another_directory() {
    let (dir, source) = populated("export-restore");
    let before = dump::dump(&source).unwrap();
    assert_eq!(before.row_counts(), [1, 3, 1, 2], "脚手架要写满四套池");

    let file = export_into_fresh_dir(&dir, "handed-over", &source);

    // 还原到**第三个**目录，用一把**新的**口令
    let target_dir = dir.join("restored");
    fs::create_dir_all(&target_dir).unwrap();
    let restored = vault_path(&target_dir);
    export::restore(
        &file,
        &mut pass(EXPORT_PASSPHRASE),
        &restored,
        &mut pass(RESTORED_PASSPHRASE),
    )
    .unwrap();

    // 判据是**内容一致**，不是"能开"：搬家后能开但规则全丢，正是 P2 要防的那种失败
    let reopened = open(&restored, &mut pass(RESTORED_PASSPHRASE)).unwrap();
    assert_eq!(
        dump::dump(&reopened).unwrap(),
        before,
        "还原后的库必须与源逐字段一致"
    );

    // 私钥也真的在（"内容一致"里最要紧的那一格，dump 有意不含它）
    let key_id = before.keys[0].id;
    let mut pem = keys::private_key(&reopened, key_id).unwrap();
    assert_eq!(&*pem.expose().unwrap(), PEM);

    // 用别的口令打不开：导出那把、源库那把，都不行
    for wrong in [EXPORT_PASSPHRASE, PASSPHRASE] {
        assert!(
            matches!(
                open(&restored, &mut pass(wrong)),
                Err(StoreError::NotADatabase)
            ),
            "还原后的库不该被 {wrong:?} 打开"
        );
    }
}

#[test]
fn the_export_file_is_itself_a_vault() {
    // D6 的"自包含、可直接给另一台机器用"：不还原也能开，而且它**不是**源库的副本 ——
    // 它用自己那把口令。
    let (dir, source) = populated("export-selfcontained");
    let before = dump::dump(&source).unwrap();
    let file = export_into_fresh_dir(&dir, "handed-over", &source);

    let direct = open(&file, &mut pass(EXPORT_PASSPHRASE)).unwrap();
    assert_eq!(dump::dump(&direct).unwrap(), before);

    assert!(matches!(
        open(&file, &mut pass(PASSPHRASE)),
        Err(StoreError::NotADatabase)
    ));

    // 导出不该动源库：它仍能用原口令打开、内容一字不差
    let still_there = open(&vault_path(&dir), &mut pass(PASSPHRASE)).unwrap();
    assert_eq!(dump::dump(&still_there).unwrap(), before);
}

#[test]
fn an_export_replaces_the_file_the_user_pointed_at() {
    // 用户是在文件选择器里点名要这个路径的，所以**覆盖**是预期行为；
    // 但覆盖必须是原子的：要么是新的导出件，要么还是原来那个文件，不会是半个。
    let (dir, source) = populated("export-overwrite");
    let before = dump::dump(&source).unwrap();
    let dest = dir.join("akasha-export.db");
    fs::write(&dest, b"what was here before").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&dest, fs::Permissions::from_mode(0o644)).unwrap();

    export::to_encrypted(
        &source,
        &mut pass(PASSPHRASE),
        &dest,
        &mut pass(EXPORT_PASSPHRASE),
    )
    .unwrap();

    let reopened = open(&dest, &mut pass(EXPORT_PASSPHRASE)).unwrap();
    assert_eq!(dump::dump(&reopened).unwrap(), before);

    #[cfg(unix)]
    assert_eq!(
        mode_of(&dest),
        0o600,
        "导出件含私钥，权限位不该由 umask 决定（D12）"
    );
}

// ── 3 / 4：明文与密文的对照 ─────────────────────────────────────────────────

#[test]
fn an_encrypted_export_holds_no_plaintext_private_key() {
    let (dir, source) = populated("export-ciphertext");
    let file = export_into_fresh_dir(&dir, "handed-over", &source);

    let bytes = fs::read(&file).unwrap();
    assert!(
        !contains(&bytes, PEM),
        "加密导出件里出现了明文私钥 —— 这正是 D6 选择'同格式的加密库'要避免的"
    );

    // 对照组：同一份数据的**明文**导出件里 grep 得到 —— 证明上面那条不是"扫描器坏了"
    let plain = dir.join("same-data-plain.db");
    export::to_plaintext(
        &source,
        &plain,
        PlaintextAck::typed(PLAINTEXT_CONFIRMATION).unwrap(),
    )
    .unwrap();
    assert!(
        contains(&fs::read(&plain).unwrap(), PEM),
        "对照组失败：明文导出件里居然没有明文私钥，说明这条 grep 搜不到东西"
    );
}

#[test]
fn a_plaintext_export_is_readable_without_any_key() {
    // 门槛的另一面：通过之后写出来的**确实**是没有加密的库。这是"为什么需要门槛"的证据，
    // 也是那条 `KEY ''` 与加密那条**同一段代码**的实测（D6）。
    let (dir, source) = populated("export-plaintext");
    let before = dump::dump(&source).unwrap();
    let out = dir.join("akasha-backup-plain.db");

    export::to_plaintext(
        &source,
        &out,
        PlaintextAck::typed(PLAINTEXT_CONFIRMATION).unwrap(),
    )
    .unwrap();

    // 裸 sqlite：不送任何 key
    let plain = Connection::open(&out).unwrap();
    let pem: Vec<u8> = plain
        .query_row("SELECT private_pem FROM keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(pem, PEM, "明文件里私钥就是明文的（门槛拦的就是这个）");

    // 版本字段必须**显式**写进去（D7：`sqlcipher_export` 不传递它）
    let version: i64 = plain
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, FORMAT_VERSION);

    // 它同样能被自己的读者读出来（这个读者 = 明文件还原那条路）
    let restored_dir = dir.join("from-plain");
    fs::create_dir_all(&restored_dir).unwrap();
    let restored = vault_path(&restored_dir);
    export::restore_plaintext(&out, &restored, &mut pass(RESTORED_PASSPHRASE)).unwrap();
    let reopened = open(&restored, &mut pass(RESTORED_PASSPHRASE)).unwrap();
    assert_eq!(dump::dump(&reopened).unwrap(), before);

    #[cfg(unix)]
    assert_eq!(mode_of(&out), 0o600, "明文件更应该收紧（D12）");
}

// ── 5：明文门槛 ─────────────────────────────────────────────────────────────

#[test]
fn the_confirmation_phrase_is_matched_word_for_word() {
    // "逐字"这条要求会被各种"差不多"试探：空、前缀、多一个空格、多一个句号、换个写法。
    // 每一个都必须被拒 —— 否则门槛就退化成"界面上有个输入框"。
    for near_miss in [
        "",
        "私钥",
        "私钥会变成明文 ",
        " 私钥会变成明文",
        "私钥会变成明文。",
        "私钥会变明文",
        "PLAINTEXT",
    ] {
        assert!(
            matches!(
                PlaintextAck::typed(near_miss),
                Err(StoreError::PlaintextRefused { .. })
            ),
            "{near_miss:?} 不该换到凭据"
        );
    }

    // 对照组：逐字敲对就给（否则上面那条可能只是"什么输入都拒"）
    assert!(PlaintextAck::typed(PLAINTEXT_CONFIRMATION).is_ok());
}

#[test]
fn a_plaintext_export_without_a_self_describing_name_is_refused() {
    let (dir, source) = populated("export-plain-name");
    let file = dir.join("akasha-backup.db");

    let err = export::to_plaintext(
        &source,
        &file,
        PlaintextAck::typed(PLAINTEXT_CONFIRMATION).unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(&err, StoreError::PlaintextRefused { reason } if reason.contains("plain")),
        "报错要说清是名字不合规：{err}"
    );
    assert!(!file.exists(), "被拒的导出不该留下任何文件");
    assert!(!partial_of(&file).exists(), "也不该留下半成品");

    // 对照组：名字自曝（大小写不敏感）就放行
    let ok = dir.join("akasha-backup-PLAIN.db");
    export::to_plaintext(
        &source,
        &ok,
        PlaintextAck::typed(PLAINTEXT_CONFIRMATION).unwrap(),
    )
    .unwrap();
    assert!(ok.exists());
}

// ── 6 / 7：独立口令与失败清理 ───────────────────────────────────────────────

#[test]
fn an_export_may_not_reuse_the_passphrase_it_comes_from() {
    let (dir, source) = populated("export-independent");
    let dest = dir.join("akasha-export.db");

    let err = export::to_encrypted(&source, &mut pass(PASSPHRASE), &dest, &mut pass(PASSPHRASE))
        .unwrap_err();
    assert!(
        matches!(err, StoreError::SharedPassphrase),
        "导出用库口令必须被拒：{err}"
    );
    assert!(!dest.exists(), "被拒的导出不该留下任何文件");

    // 还原那条路同理：还原出来的库不能与导出件同口令
    let file = export_into_fresh_dir(&dir, "handed-over", &source);
    let target_dir = dir.join("restored");
    fs::create_dir_all(&target_dir).unwrap();
    let err = export::restore(
        &file,
        &mut pass(EXPORT_PASSPHRASE),
        &vault_path(&target_dir),
        &mut pass(EXPORT_PASSPHRASE),
    )
    .unwrap_err();
    assert!(matches!(err, StoreError::SharedPassphrase), "{err}");
    assert!(!vault_path(&target_dir).exists());
}

#[test]
fn a_failed_export_leaves_nothing_behind() {
    let (dir, source) = populated("export-failure");

    // 目标是个**目录**：写到 `…partial` 那一步都会成功，只有在改名那一步才会失败 ——
    // 这正是"半成品要清掉"那条清理路径唯一能被触发的地方。
    let as_dir = dir.join("akasha-export.db");
    fs::create_dir(&as_dir).unwrap();
    let err = export::to_encrypted(
        &source,
        &mut pass(PASSPHRASE),
        &as_dir,
        &mut pass(EXPORT_PASSPHRASE),
    )
    .unwrap_err();
    assert!(matches!(err, StoreError::Io(_)), "改名失败要报出来：{err}");
    assert!(
        !partial_of(&as_dir).exists(),
        "失败之后半成品必须清掉 —— 否则用户会以为备份好了"
    );
    assert!(as_dir.is_dir(), "目标本身不该被动");

    // 父目录不存在：更早一步失败，同样什么都不留
    let missing = dir.join("no-such-dir").join("akasha-export.db");
    assert!(
        export::to_encrypted(
            &source,
            &mut pass(PASSPHRASE),
            &missing,
            &mut pass(EXPORT_PASSPHRASE)
        )
        .is_err()
    );
    assert!(!missing.exists());
    assert!(!partial_of(&missing).exists());
}

#[test]
fn restore_refuses_to_overwrite_a_vault_that_is_already_there() {
    let (dir, source) = populated("export-restore-refuses");
    let file = export_into_fresh_dir(&dir, "handed-over", &source);

    let target_dir = fixture_dir("export-restore-refuses-target");
    let target = vault_path(&target_dir);
    {
        // 先把库槽占住（这就是"用户已经有一个库"那条路）
        let _occupied = create(&target, &mut pass(RESTORED_PASSPHRASE)).unwrap();
    }
    let before = fs::read(&target).unwrap();

    let err = export::restore(
        &file,
        &mut pass(EXPORT_PASSPHRASE),
        &target,
        &mut pass(RESTORED_PASSPHRASE),
    )
    .unwrap_err();
    assert!(matches!(err, StoreError::VaultExists(_)), "{err}");
    assert_eq!(
        fs::read(&target).unwrap(),
        before,
        "拒绝之后原库的字节一个都不该变"
    );

    // 明文那条路同样不覆盖
    let plain = dir.join("akasha-restore-plain.db");
    export::to_plaintext(
        &source,
        &plain,
        PlaintextAck::typed(PLAINTEXT_CONFIRMATION).unwrap(),
    )
    .unwrap();
    let err =
        export::restore_plaintext(&plain, &target, &mut pass(RESTORED_PASSPHRASE)).unwrap_err();
    assert!(matches!(err, StoreError::VaultExists(_)), "{err}");
    assert_eq!(fs::read(&target).unwrap(), before);
}

// ── 8：dump ─────────────────────────────────────────────────────────────────

#[test]
fn dump_shows_what_is_in_the_vault_and_nothing_secret() {
    let (_dir, conn) = populated("export-dump");
    let read = dump::dump(&conn).unwrap();

    assert_eq!(read.format_version, FORMAT_VERSION);
    assert_eq!(read.row_counts(), [1, 3, 1, 2]);
    assert_eq!(read.keys.len(), 1, "密钥池列出的是**不含私钥**的行");

    let text = read.to_text();
    for expected in [
        // 写死当前版本号（plan 0503 起是 v2）：它红了就意味着格式变了，
        // 而那件事本来就该在这里被看见一次（同 `schema_contract` 的快照）。
        "format: v2",
        "work",
        "ssh-ed25519 AAAAC3Nza work",
        "bastion",
        "10.0.0.5:2222",
        "auth=publickey",
        "jump=",
        "/dev/ttyUSB0",
        "115200 8N1",
        "flow=hardware",
        "dynamic",
        "10.0.0.5:80",
        "autostart=yes",
    ] {
        assert!(
            text.contains(expected),
            "dump 里该有 {expected:?}：\n{text}"
        );
    }
    assert!(
        !contains(text.as_bytes(), PEM),
        "dump 里出现了私钥：\n{text}"
    );

    // 空库也要 dump 得出来（诊断最常见的那一问："它是空的吗"）
    let (_empty_dir, empty) = new_vault("export-dump-empty");
    let nothing = dump::dump(&empty).unwrap();
    assert_eq!(nothing.row_counts(), [0, 0, 0, 0]);
    let text = nothing.to_text();
    for expected in [
        "format: v2",
        "keys: 0",
        "hosts: 0",
        "serials: 0",
        "forwards: 0",
    ] {
        assert!(text.contains(expected), "\n{text}");
    }
}
