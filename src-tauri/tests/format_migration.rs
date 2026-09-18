//! **格式迁移**（plan 0503）：v1 的库开完之后是 v2，且**一条数据都没少**。
//!
//! 这是本仓库的第一次迁移，所以这一份同时是"以后照抄"的样板：`DDL_V1` 造历史 →
//! `open` 升上来 → 逐个断言"版本、表、**内容**"。少任何一条，红的都只是"文件打得开"，
//! 而"打得开但内容不对"正是 ADR-0002 D7 要防的东西。
//!
//! ⚠️ 它守的另一半是**不迁移**：只读地消费一个导出件时（[`akasha_lib::store::export::restore`]、
//! 明文那条路），来源文件**一个字节都不许变**。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "store_common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};

use akasha_lib::store::{
    DDL_V1, DDL_V2, FORMAT_VERSION, StoreError, export, hosts, open, open_unmigrated, vault_path,
};
use common::{PASSPHRASE, columns_of, fixture_dir, pass};
use rusqlite::{Connection, ffi};

/// 还原用的另一个口令（ADR-0002 D6：导出件不得与来源同口令，所以还原也一样）。
const OTHER: &[u8] = b"a different correct horse battery staple";

/// 造一个**真正的 v1 库**：v1 的冻结 DDL + `user_version = 1` + 一条 host。
///
/// 这里直接走 `sqlite3_key` 与 `DDL_V1`，而不是借今天的 `create`：
/// 后者只会写出当前格式。**v1 的定义只有 `DDL_V1` 一处**（ADR-0002 D7），
/// 所以历史库就得从它造出来 —— 这份测试也因此是"那段 DDL 真的能建库"的证据。
#[allow(unsafe_code)] // 本 crate 是唯一允许碰 ffi 的地方（no-unsafe-outside-store.yml）
fn create_v1(dir: &Path) -> PathBuf {
    let db = vault_path(dir);
    let conn = Connection::open(&db).unwrap();
    key(&conn);

    conn.execute_batch(DDL_V1).unwrap();
    conn.execute_batch(
        "INSERT INTO hosts (name, host, port, user, auth)
         VALUES ('web', 'example.com', 22, 'root', 'agent')",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    db
}

/// 造一个**真正的 v2 库**：v1 + v2 的冻结定义（`DDL_V2` 至今没改过，所以它就是 v2 的定义）
/// + 一条 host + 一条 known_hosts，`user_version = 2`。
///
/// 为什么要第二个历史版本：v3 只在 v2 之上加表，而"从 v2 升上来"与"从 v1 升上来"走的
/// 是**同一段循环的两步** —— 只有 v1 那一份的话，"`tables_of(2)` 那一档还在不在"没人验
/// （少了它，真 v2 库会被当成不认识的版本直接拒绝）。
#[allow(unsafe_code)] // 本 crate 是唯一允许碰 ffi 的地方（no-unsafe-outside-store.yml）
fn create_v2(dir: &Path) -> PathBuf {
    let db = vault_path(dir);
    let conn = Connection::open(&db).unwrap();
    key(&conn);

    conn.execute_batch(DDL_V1).unwrap();
    conn.execute_batch(DDL_V2).unwrap();
    conn.execute_batch(
        "INSERT INTO hosts (name, host, port, user, auth)
         VALUES ('web', 'example.com', 22, 'root', 'agent');
         INSERT INTO known_hosts (host, port, key_type, key_blob, fingerprint)
         VALUES ('example.com', 22, 'ssh-ed25519', X'0001', 'SHA256:v2');",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 2).unwrap();
    db
}

/// 把口令经 C API 送进一个**裸**连接（v1 时代的 `create` 就是这么建库的）。
#[allow(unsafe_code)]
fn key(conn: &Connection) {
    // SAFETY: ① `conn.handle()` 在本连接存活期间有效，而 `conn` 在调用点活着（借用而非复制）；
    // ② 指针与长度在本次调用期间有效（`PASSPHRASE` 是 `'static` 字节字面量），且
    // `sqlite3_key` 只读它 —— 它把密钥复制进自己的缓冲区，调用返回后不持有这个指针；
    // ③ 两处调用都发生在任何**读库**的语句之前（ADR-0002 D4 的顺序要求）。
    let rc = unsafe {
        ffi::sqlite3_key(
            conn.handle(),
            PASSPHRASE.as_ptr().cast(),
            PASSPHRASE.len() as i32,
        )
    };
    assert_eq!(rc, ffi::SQLITE_OK, "送密钥失败");
}

/// 库里写的版本号，**裸读**（连 `open` 的门都不进）。
///
/// 为什么不用 [`open_unmigrated`]：那扇门本来就拒"版本 0"与"缺表"的库，
/// 而这几条用例恰恰要断言**被拒之后版本号一个字节没动** —— 从被拒的门后面看是看不到的。
fn raw_version(db: &Path) -> i64 {
    let conn = Connection::open(db).unwrap();
    key(&conn);
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

// ── 1. 升级：版本、表、**内容** ─────────────────────────────────────────────

#[test]
fn a_v2_vault_is_upgraded_to_v3_and_keeps_its_rows() {
    let dir = fixture_dir("migrate-v2-to-v3");
    let db = create_v2(&dir);
    assert_eq!(raw_version(&db), 2, "前提：造出来的确实是 v2");

    let conn = open(&db, &mut pass(PASSPHRASE)).unwrap();
    let found: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(found, FORMAT_VERSION);

    // v3 加的那张表在，且形状就是 DDL 说的那样。
    assert_eq!(
        columns_of(&conn, "bw_items"),
        [
            "cipher_id",
            "name",
            "revision_date",
            "fingerprint",
            "key_id"
        ]
    );

    // **内容一条不少**：v2 的 known_hosts 与更早那条 host 都还在。
    assert_eq!(hosts::hosts(&conn).unwrap().len(), 1);
    let keys = conn
        .query_row("SELECT count(*) FROM known_hosts", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    assert_eq!(keys, 1, "v2 那条 known_hosts 必须还在");

    drop(conn);
    assert_eq!(raw_version(&db), FORMAT_VERSION);
}

#[test]
fn a_v1_vault_is_upgraded_on_open_and_keeps_its_rows() {
    let dir = fixture_dir("migrate-v1-to-v2");
    let db = create_v1(&dir);
    assert_eq!(raw_version(&db), 1, "前提：造出来的确实是 v1");

    let conn = open(&db, &mut pass(PASSPHRASE)).unwrap();

    let found: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        found, FORMAT_VERSION,
        "开完之后应当是当前格式（{FORMAT_VERSION}）"
    );

    // 新表在
    assert_eq!(
        columns_of(&conn, "known_hosts"),
        ["id", "host", "port", "key_type", "key_blob", "fingerprint"]
    );

    // **内容一条不少** —— 只看"打得开"是不够的：迁移要是把行弄丢了，上面两条照样过。
    let rows = hosts::hosts(&conn).unwrap();
    assert_eq!(rows.len(), 1, "v1 里那条 host 必须还在");
    assert_eq!(rows[0].host, "example.com");
    assert_eq!(rows[0].user, "root");

    // 盘上也确实是 v2 了（换个打开方式再读一次，不靠上面那个连接的缓存）
    drop(conn);
    assert_eq!(raw_version(&db), FORMAT_VERSION);
}

#[test]
fn opening_a_current_vault_again_writes_nothing() {
    let dir = fixture_dir("migrate-idempotent");
    let db = create_v1(&dir);

    // 第一次：升级（要写）
    drop(open(&db, &mut pass(PASSPHRASE)).unwrap());
    let after_upgrade = fs::read(&db).unwrap();

    // 第二次：已经是当前格式 → **一个字节都不该动**。
    // 这条判据是"迁移只在需要时发生"的机器可查形态：如果 `open` 每次都重跑迁移，
    // 时间久了库会莫名其妙地变大（页面被反复重写），而没人能解释为什么。
    drop(open(&db, &mut pass(PASSPHRASE)).unwrap());
    assert_eq!(
        fs::read(&db).unwrap(),
        after_upgrade,
        "再开一次当前格式的库不该改文件"
    );
}

#[test]
fn version_zero_is_still_refused() {
    let dir = fixture_dir("migrate-version-zero");
    let db = create_v1(&dir);
    {
        // 0 = "v1 之前没有版本" → 它不是本程序的库（D7：不把来路不明的文件当待迁移）。
        let (conn, _) = open_unmigrated(&db, &mut pass(PASSPHRASE)).unwrap();
        conn.pragma_update(None, "user_version", 0).unwrap();
    }
    let err = open(&db, &mut pass(PASSPHRASE)).unwrap_err();
    assert!(
        matches!(err, StoreError::UnsupportedVersion { found: 0 }),
        "{err:?}"
    );
    assert_eq!(raw_version(&db), 0, "拒绝之后版本号不该被动过");
}

#[test]
fn a_v1_vault_missing_a_table_is_refused_instead_of_migrated() {
    let dir = fixture_dir("migrate-broken-v1");
    let db = create_v1(&dir);
    {
        let conn = Connection::open(&db).unwrap();
        key(&conn);
        conn.execute_batch("DROP TABLE keys").unwrap();
    }

    let err = open(&db, &mut pass(PASSPHRASE)).unwrap_err();
    assert!(
        matches!(err, StoreError::MissingTable { table: "keys" }),
        "缺表的 v1 库不是「待迁移」，是坏的 —— 迁移要**先**按旧版本确认形状：{err:?}"
    );
    assert_eq!(raw_version(&db), 1, "拒绝之后不该留下半迁移状态");
}

// ── 2. 消费历史导出件：**升的是副本，来源一个字节不动** ──────────────────────

#[test]
fn restoring_a_v1_export_upgrades_the_copy_and_leaves_the_source_alone() {
    let dir = fixture_dir("migrate-restore-v1");
    let source = create_v1(&dir);
    let before = fs::read(&source).unwrap();
    let dest = dir.join("restored.db");

    export::restore(&source, &mut pass(PASSPHRASE), &dest, &mut pass(OTHER)).unwrap();

    // 目标：当前格式 + 内容在
    let conn = open(&dest, &mut pass(OTHER)).unwrap();
    let found: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(found, FORMAT_VERSION, "还原出来的库要是当前格式");
    assert_eq!(hosts::hosts(&conn).unwrap().len(), 1);

    // 来源：v1，且**逐字节没变**（它是用户的产物，可能在只读介质上）
    drop(conn);
    assert_eq!(fs::read(&source).unwrap(), before, "还原不该改写导出件本身");
    assert_eq!(raw_version(&source), 1, "也不该顺手把它升上来");
}

#[test]
fn restoring_a_v1_plaintext_export_also_upgrades_the_copy() {
    let dir = fixture_dir("migrate-restore-v1-plain");
    let source = dir.join("akasha-export-plain.db");
    {
        // 明文导出件就是"没有加密的库"：同一段 DDL + 同一个版本号（D6）。
        let conn = Connection::open(&source).unwrap();
        conn.execute_batch(DDL_V1).unwrap();
        conn.execute_batch(
            "INSERT INTO hosts (name, host, port, user, auth)
             VALUES ('web', 'example.com', 22, 'root', 'agent')",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }
    let before = fs::read(&source).unwrap();
    let dest = dir.join("restored.db");

    export::restore_plaintext(&source, &dest, &mut pass(OTHER)).unwrap();

    let conn = open(&dest, &mut pass(OTHER)).unwrap();
    let found: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(found, FORMAT_VERSION);
    assert_eq!(hosts::hosts(&conn).unwrap().len(), 1);

    drop(conn);
    assert_eq!(fs::read(&source).unwrap(), before, "明文件同样不该被改写");
}
