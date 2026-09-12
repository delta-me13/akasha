//! plan 0402 的**判据**测试：口令不以任何形式出现在磁盘上。
//!
//! 判据来自 ROADMAP 阶段 4：*口令不以任何形式出现在磁盘上（配置文件、日志、临时文件都不行）*。
//! 它是一条**关于磁盘位与字节**的声明，所以只能靠"扫真实文件"来验，不能靠读代码点头。
//!
//! 因此这里用的是一个**一眼能认出来的标记口令**，跑完一遍真实的 create + unlock + 解锁失败，
//! 然后递归扫数据目录里每一个文件。fixture 留在 `target/store-passphrase/`（不是临时目录）：
//! 判据要能被人工复核 —— `grep -rl` 一下就行，见 plan 0402 的「验收命令」。
//!
//! ⚠️ **扫描器本身也要被验证**：一个坏掉的扫描器会给出"0 命中"这个最令人安心的答案。
//! 所以旁边有一个对照目录，里面放一个**含标记的文件**，扫描器必须找到它。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段

use std::fs;
use std::path::{Path, PathBuf};

use akasha_store::{Passphrase, StoreError, create, open, vault_path};

/// 标记口令：不可能碰巧出现在别的地方，grep 得到。
const MARKER: &[u8] = b"akasha-passphrase-marker-9d0f4c7b1e5a";
const OTHER: &[u8] = b"a-different-passphrase";
const SECRET: &str =
    "-----BEGIN OPENSSH PRIVATE KEY-----\nleak-probe\n-----END OPENSSH PRIVATE KEY-----\n";

fn pass(bytes: &[u8]) -> Passphrase {
    Passphrase::new(bytes.to_vec()).unwrap()
}

fn fixture_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/store-passphrase")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// 递归找出 `dir` 下**内容里含 `needle`** 的文件。找不到就是空表。
fn files_containing(dir: &Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if fs::read(&path)
                .unwrap()
                .windows(needle.len())
                .any(|w| w == needle)
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

// ── 判据：扫真实文件，0 命中 ────────────────────────────────────────────────

#[test]
fn passphrase_never_reaches_the_data_directory() {
    let dir = fixture_dir("vault");
    let db = vault_path(&dir);

    // ① 建库（口令在这一刻进入 SQLCipher 的 KDF）
    {
        let conn = create(&db, &pass(MARKER)).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE key_pool(secret TEXT); INSERT INTO key_pool VALUES ('{SECRET}');"
        ))
        .unwrap();
    }
    // ② 正常解锁一次
    {
        let conn = open(&db, &pass(MARKER)).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM key_pool", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
    // ③ 解锁**失败**一次 —— 错误路径是秘密最容易漏进日志的地方
    let err = open(&db, &pass(OTHER)).unwrap_err();
    assert!(matches!(err, StoreError::NotADatabase), "实际是 {err:?}");

    // 先确认扫描对象存在、非空：否则"0 命中"可能只是因为没有东西可扫
    let bytes = fs::read(&db).unwrap();
    assert!(!bytes.is_empty(), "库文件是空的，这条测试失去意义");
    println!("库 = {}（{} 字节）", db.display(), bytes.len());

    let leaked = files_containing(&dir, MARKER);
    assert!(
        leaked.is_empty(),
        "口令出现在磁盘上：{leaked:?}（判据：ROADMAP 阶段 4）"
    );
    // 顺带：库里的私钥同样不该以明文出现（0401 判据 ②，这里换个目录再确认一次）
    assert!(
        files_containing(&dir, SECRET.as_bytes()).is_empty(),
        "明文私钥出现在数据目录里"
    );
}

// ── 对照：扫描器必须真的能搜到东西 ──────────────────────────────────────────

#[test]
fn the_scanner_finds_a_marker_when_one_is_really_there() {
    let dir = fixture_dir("control");
    let leak = dir.join("leak.txt");
    fs::write(
        &leak,
        format!("this file contains {}\n", String::from_utf8_lossy(MARKER)),
    )
    .unwrap();

    // 一个永远返回空表的扫描器会让上面那条判据变成一句空话 —— 这里把它钉住。
    let found = files_containing(&dir, MARKER);
    assert_eq!(found, vec![leak], "扫描器没找到真实存在的标记");
    assert!(
        files_containing(&dir, b"not-present-anywhere").is_empty(),
        "扫描器把所有文件都当成命中"
    );
}
