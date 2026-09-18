//! **Bitwarden 导入池**（plan 0903）：一次导入落两样东西 —— 私钥进 `keys`、来历进 `bw_items`。
//!
//! 盯的是那条链上**会静默出错**的四种情形：
//!
//! | 盯什么 | 不说清会怎样 |
//! |---|---|
//! | 同名条目默认不动、`overwrite` 才替换 | 重导一次就毁掉用户手改过的行 |
//! | **这一批里自己重名**时后者被跳过 | 上游允许两个条目同名，静默让后者盖掉前者 = 用户的两把钥匙少了一把而没有任何痕迹 |
//! | 同一条目再导一次是**更新**（`cipher_id` 是主键） | 池里长出第二条来历，而 `key_id` 的 `UNIQUE` 会把它变成一次约束失败 |
//! | 删掉钥匙时来历行**跟着走** | 留下一条解释不了任何东西的记录 |

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "store_common/mod.rs"]
mod common;

use akasha_lib::store::pools::bw_items::{Incoming, Skip, import_snapshot, item, items};
use akasha_lib::store::pools::keys::{self, PrivateKey};
use common::new_vault;

/// 一条条目（私钥的内容按名字生成，好让"哪一把留在池里"可断言）。
fn incoming(cipher: &str, name: &str, revision: &str) -> Incoming {
    Incoming {
        cipher_id: cipher.to_owned(),
        name: name.to_owned(),
        revision_date: revision.to_owned(),
        fingerprint: format!("SHA256:{name}"),
        public_key: format!("ssh-ed25519 AAAA-{name}"),
        private: PrivateKey::new(
            format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{name}\n").into_bytes(),
        )
        .unwrap(),
    }
}

/// 某把钥匙的私钥正文（**要的是"哪一把真的在池里"**，不是"有几行"）。
fn pem_of(conn: &akasha_lib::store::Connection, name: &str) -> String {
    let id = keys::find_by_name(conn, name).unwrap().unwrap();
    let mut key = keys::private_key(conn, id).unwrap();
    String::from_utf8(key.expose().unwrap().to_vec()).unwrap()
}

#[test]
fn one_import_writes_the_key_and_its_provenance() {
    let (_dir, conn) = new_vault("bw-import-basic");

    let outcome = import_snapshot(
        &conn,
        &mut [incoming("c-1", "id_ed25519", "2026-09-01T00:00:00Z")],
        false,
    )
    .unwrap();
    assert_eq!(outcome.created.len(), 1);
    assert!(outcome.replaced.is_empty() && outcome.skipped.is_empty());

    let rows = items(&conn).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cipher_id, "c-1");
    assert_eq!(rows[0].name, "id_ed25519");
    assert_eq!(rows[0].revision_date, "2026-09-01T00:00:00Z");
    assert_eq!(rows[0].fingerprint, "SHA256:id_ed25519");
    assert_eq!(
        rows[0].key_id,
        keys::find_by_name(&conn, "id_ed25519").unwrap().unwrap(),
        "来历要指向库里那一行"
    );
    assert!(pem_of(&conn, "id_ed25519").contains("id_ed25519"));
}

/// 同名默认不动 —— 池里那一行（连用户改过的备注）原样留着。
#[test]
fn an_existing_name_is_left_alone_until_overwrite() {
    let (_dir, conn) = new_vault("bw-import-exists");
    import_snapshot(&conn, &mut [incoming("c-1", "shared", "rev-1")], false).unwrap();

    let id = keys::find_by_name(&conn, "shared").unwrap().unwrap();
    let mut row = keys::key(&conn, id).unwrap();
    row.comment = Some("用户写的备注".to_owned());
    keys::update_key(&conn, &row).unwrap();

    let outcome = import_snapshot(&conn, &mut [incoming("c-2", "shared", "rev-2")], false).unwrap();
    assert_eq!(outcome.skipped, vec![("shared".to_owned(), Skip::Exists)]);
    assert_eq!(keys::find_by_name(&conn, "shared").unwrap(), Some(id));
    assert!(
        pem_of(&conn, "shared").contains("shared"),
        "被跳过的那一条不许换掉池里的私钥"
    );
    assert!(item(&conn, "c-2").is_err(), "被跳过的那一条不该留下来历");

    // 另一条上游条目**同名**时 `overwrite` 也救不了：那一行已经归 `c-1` 了，
    // 让 `c-2` 顶掉它等于把 `c-1` 悄悄变成"没来历"。
    let outcome = import_snapshot(&conn, &mut [incoming("c-2", "shared", "rev-2")], true).unwrap();
    assert!(outcome.replaced.is_empty());
    assert_eq!(outcome.skipped, vec![("shared".to_owned(), Skip::Claimed)]);
    assert_eq!(item(&conn, "c-1").unwrap().revision_date, "rev-1");

    // 同一条目（`c-1`）再导一次才是**替换**：私钥与公钥换成上游说的，**用户的备注留着**。
    let mut again = [incoming("c-1", "shared", "rev-2")];
    again[0].public_key = "ssh-ed25519 AAAA-shared-v2".to_owned();
    let outcome = import_snapshot(&conn, &mut again, true).unwrap();
    assert_eq!(outcome.replaced.len(), 1);
    assert_eq!(
        keys::key(&conn, id).unwrap().comment.as_deref(),
        Some("用户写的备注"),
        "换私钥没有理由抹掉用户的备注"
    );
    assert_eq!(
        keys::key(&conn, id).unwrap().public_key,
        "ssh-ed25519 AAAA-shared-v2"
    );
    assert_eq!(item(&conn, "c-1").unwrap().revision_date, "rev-2");
}

/// **这一批里自己重名**：后者被跳过，前者**必须完好** —— 这正是 `overwrite` 也救不了的那档。
#[test]
fn two_items_with_the_same_name_in_one_batch_do_not_overwrite_each_other() {
    let (_dir, conn) = new_vault("bw-import-duplicate-name");

    let mut batch = [
        incoming("c-1", "same", "rev-1"),
        incoming("c-2", "same", "rev-2"),
    ];
    let outcome = import_snapshot(&conn, &mut batch, true).unwrap();

    assert_eq!(outcome.created.len(), 1);
    assert_eq!(
        outcome.skipped,
        vec![("same".to_owned(), Skip::DuplicateName)],
        "第二条要说清是被这一批里的重名挡住的，而不是池里本来就有"
    );
    assert_eq!(
        keys::find_by_name(&conn, "same").unwrap(),
        Some(outcome.created[0].id),
        "留下的必须是第一条那一行"
    );
    assert_eq!(items(&conn).unwrap().len(), 1, "第二条不该留下来历");
    assert_eq!(item(&conn, "c-1").unwrap().revision_date, "rev-1");
}

/// 同一条目再导一次是**更新**：`revision_date` / `fingerprint` 跟着上游走，池里还是一条。
#[test]
fn re_importing_the_same_item_updates_the_record_in_place() {
    let (_dir, conn) = new_vault("bw-import-update");
    import_snapshot(&conn, &mut [incoming("c-1", "key", "rev-1")], false).unwrap();
    let id = keys::find_by_name(&conn, "key").unwrap().unwrap();

    let mut again = [incoming("c-1", "key", "rev-2")];
    again[0].fingerprint = "SHA256:changed".to_owned();
    let outcome = import_snapshot(&conn, &mut again, true).unwrap();

    assert_eq!(outcome.replaced.len(), 1);
    assert_eq!(
        items(&conn).unwrap().len(),
        1,
        "cipher_id 是主键，不该长出第二行"
    );
    let row = item(&conn, "c-1").unwrap();
    assert_eq!(row.revision_date, "rev-2");
    assert_eq!(row.fingerprint, "SHA256:changed");
    assert_eq!(row.key_id, id, "还是同一行钥匙");
}

/// 删掉钥匙 = 来历行跟着走（`ON DELETE CASCADE`）：导入池里没有"孤儿"这种状态。
#[test]
fn deleting_the_key_takes_the_provenance_with_it() {
    let (_dir, conn) = new_vault("bw-import-cascade");
    import_snapshot(&conn, &mut [incoming("c-1", "key", "rev-1")], false).unwrap();
    let id = keys::find_by_name(&conn, "key").unwrap().unwrap();

    keys::delete_key(&conn, id).unwrap();
    assert!(items(&conn).unwrap().is_empty(), "来历行该随钥匙一起消失");
}

/// 反向：只抹掉来历（`forget`）时**钥匙留着** —— "这条快照不再有用了，但钥匙留着自己用"。
#[test]
fn forgetting_the_record_keeps_the_key() {
    let (_dir, conn) = new_vault("bw-import-forget");
    import_snapshot(&conn, &mut [incoming("c-1", "key", "rev-1")], false).unwrap();

    akasha_lib::store::pools::bw_items::forget(&conn, "c-1").unwrap();
    assert!(items(&conn).unwrap().is_empty());
    assert!(
        keys::find_by_name(&conn, "key").unwrap().is_some(),
        "钥匙不该被顺手删掉"
    );
}
