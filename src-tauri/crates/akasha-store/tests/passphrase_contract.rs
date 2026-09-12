//! plan 0402 的契约测试：**打开与新建是两条路**，以及口令真的被钉住了。
//!
//! 这一份守的是**我们的**行为（`sqlcipher_contract.rs` 守的是上游的）。核心那条来自一次
//! 意外的实测：一个不存在（或 0 字节）的文件，**用任何口令都能"打开"** ——
//! 库里没有任何东西可解，KDF 根本没跑。0401 的 `open()` 同时管新建与打开，于是
//! "打开"在新建这条路上等于没验证口令，还把这把错口令当成了创建口令。
//!
//! 所以这里的第一条断言就是：`create` 之后，**只有那一把口令**能开。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::fs;
use std::path::{Path, PathBuf};

use akasha_store::{
    FORMAT_VERSION, Passphrase, STORE_FILE_NAME, StoreError, create, open, vault_path,
};

const PASSPHRASE: &[u8] = b"correct horse battery staple";
const OTHER: &[u8] = b"correct horse battery stapl";

fn pass(bytes: &[u8]) -> Passphrase {
    Passphrase::new(bytes.to_vec()).unwrap()
}

/// 每例一个干净目录，产物留在 `target/store-passphrase/` 供人工复核。
fn fixture_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/store-passphrase")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

// ── 1. 判据：新建的库从第一刻起只认那一把口令 ────────────────────────────────

#[test]
fn a_fresh_vault_only_accepts_the_passphrase_it_was_created_with() {
    let db = fixture_dir("only-one-passphrase").join(STORE_FILE_NAME);

    {
        let conn = create(&db, &pass(PASSPHRASE)).unwrap();
        // 一句话就够：`create` 写下的 `user_version` 是文件的第一页，
        // 盐与密钥校验值就落在这里 —— 在那之前文件是 0 字节，谁的"口令"都成立。
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            FORMAT_VERSION
        );
    }

    let bytes = fs::read(&db).unwrap();
    println!("create 之后 = {} 字节", bytes.len());
    assert_eq!(bytes.len(), 4096, "建库应当把文件实体化（一页头）");
    assert!(
        !bytes.starts_with(b"SQLite format 3"),
        "库头是明文 SQLite 魔数 —— 这个库没加密"
    );

    // 正确口令：开 + 能读
    let conn = open(&db, &pass(PASSPHRASE)).unwrap();
    conn.execute_batch("CREATE TABLE probe(x); INSERT INTO probe VALUES (1);")
        .unwrap();
    drop(conn);
    let conn = open(&db, &pass(PASSPHRASE)).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM probe", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);

    // 差一个字节的口令就不行 —— 这条在 0401 的写法下（新建即"打开"）根本测不出来
    let err = open(&db, &pass(OTHER)).unwrap_err();
    assert!(matches!(err, StoreError::NotADatabase), "实际是 {err:?}");
}

// ── 2. 不存在 / 0 字节 → NoVault（而不是"用任意口令建一个"）──────────────────

#[test]
fn open_refuses_a_vault_that_does_not_exist_yet() {
    let dir = fixture_dir("no-vault");

    let missing = dir.join(STORE_FILE_NAME);
    let err = open(&missing, &pass(PASSPHRASE)).unwrap_err();
    assert!(
        matches!(&err, StoreError::NoVault(path) if path == &missing),
        "实际是 {err:?}"
    );
    assert!(!missing.exists(), "open 不该在磁盘上留下任何东西");

    // 0 字节的残留文件同样算"没建过"。它是**最危险**的那种输入：
    // 换作 0401 的 open()，这里会成功、并且把临时口令当成创建口令。
    let empty = dir.join("leftover.db");
    fs::write(&empty, b"").unwrap();
    let err = open(&empty, &pass(OTHER)).unwrap_err();
    assert!(matches!(err, StoreError::NoVault(_)), "实际是 {err:?}");
    assert_eq!(fs::metadata(&empty).unwrap().len(), 0, "open 不该写它");
}

// ── 3. `create` 永不覆盖一个已有内容的库 ────────────────────────────────────

#[test]
fn create_never_overwrites_a_vault() {
    let db = fixture_dir("never-overwrite").join(STORE_FILE_NAME);
    {
        let conn = create(&db, &pass(PASSPHRASE)).unwrap();
        conn.execute_batch("CREATE TABLE probe(x); INSERT INTO probe VALUES (1);")
            .unwrap();
    }
    let before = fs::read(&db).unwrap();

    // 换一把口令去"建"同一个路径：必须是错误，而不是把用户的库清掉。
    let err = create(&db, &pass(OTHER)).unwrap_err();
    assert!(matches!(err, StoreError::VaultExists(_)), "实际是 {err:?}");
    assert_eq!(fs::read(&db).unwrap(), before, "库文件被动过了");

    // 而且原口令照旧能开（覆盖型的实现常常在"报错前"已经写坏文件）
    open(&db, &pass(PASSPHRASE)).unwrap();
}

#[test]
fn create_accepts_a_zero_byte_leftover() {
    // 0 字节 = 还没有密钥落在这条路径上（见 lib.rs 的 file_len），所以它是"可以建"的位置。
    // 若这里判 VaultExists，用户会陷入"删不掉也建不了"的死角。
    let db = fixture_dir("zero-byte-leftover").join(STORE_FILE_NAME);
    fs::write(&db, b"").unwrap();

    let conn = create(&db, &pass(PASSPHRASE)).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        FORMAT_VERSION
    );
    assert_eq!(fs::metadata(&db).unwrap().len(), 4096);
}

// ── 4. D7：版本是唯一的格式权威，打开时校验 ─────────────────────────────────

#[test]
fn open_rejects_a_version_it_does_not_know() {
    let dir = fixture_dir("version-check");

    for found in [FORMAT_VERSION + 1, 0] {
        let db = dir.join(format!("v{found}.db"));
        let conn = create(&db, &pass(PASSPHRASE)).unwrap();
        conn.pragma_update(None, "user_version", found).unwrap();
        drop(conn);

        let err = open(&db, &pass(PASSPHRASE)).unwrap_err();
        assert!(
            matches!(err, StoreError::UnsupportedVersion { found: f } if f == found),
            "版本 {found} 的实际结果是 {err:?}"
        );
    }

    // `< 1` 拒绝而不是"当成待迁移"：v1 之前没有版本，非空文件里出现 0 说明它不是本程序的库。
    // 接着往下走，就是"能开但内容不对"的入口（ADR-0002 §10 记了这条措辞改动）。
    let db = dir.join("v2-still-ok.db");
    let conn = create(&db, &pass(PASSPHRASE)).unwrap();
    drop(conn);
    open(&db, &pass(PASSPHRASE)).unwrap(); // 对照：版本对的时候当然能开
}

// ── 5. 落点：库与 `config.json` 同目录（D1），文件名叫 `akasha.db` ───────────

#[test]
fn vault_path_is_the_store_file_in_the_given_directory() {
    let dir = Path::new("/tmp/some-data-dir");
    assert_eq!(
        vault_path(dir),
        Path::new("/tmp/some-data-dir/akasha.db"),
        "D1：库文件名与落点是磁盘格式的一部分，改了等于迁移用户数据"
    );
    assert_eq!(vault_path(dir).file_name().unwrap(), STORE_FILE_NAME);
}
