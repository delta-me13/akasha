//! 集成测试共用的脚手架。**不是测试目标**：`cargo` 只把 `tests/*.rs` 当目标，
//! `tests/common/mod.rs` 是被各个目标 `mod common;` 引进来的普通模块。
//!
//! `#![allow(dead_code)]` 是必须的：每个目标各取所需，用不到的辅助函数在
//! `-D warnings` 的 clippy 下会直接让门禁红 —— 而那不是"有死代码"，只是"这个目标没用它"。

#![allow(dead_code)]
#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::fs;
use std::path::{Path, PathBuf};

use akasha_store::{Passphrase, STORE_FILE_NAME, create, open, vault_path};
use rusqlite::Connection;
use rusqlite::types::ValueRef;

/// 全部用例共用的口令。
pub const PASSPHRASE: &[u8] = b"correct horse battery staple";

/// 把字节包成口令。`Passphrase` 刻意不实现 `Clone`，所以每个用例各自造一份。
pub fn pass(bytes: &[u8]) -> Passphrase {
    Passphrase::new(bytes.to_vec()).unwrap()
}

/// 每例一个干净目录；产物留在 `target/store-pools/` 下，**故意不删** ——
/// 判据是关于磁盘上的字节的，要能拿一个真实文件手工复核。
pub fn fixture_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/store-pools")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// 建一个解锁好的空库（v1 的四张表，还没有数据），返回 (数据目录, 连接)。
///
/// 返回目录是刻意的：P2 的判据要检查"库里的值有没有提到**我们的**目录"。
pub fn new_vault(name: &str) -> (PathBuf, Connection) {
    let dir = fixture_dir(name);
    let db = vault_path(&dir);
    let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
    (dir, conn)
}

/// 重新打开一个已经存在的库。
pub fn reopen(db: &Path) -> Connection {
    open(db, &mut pass(PASSPHRASE)).unwrap()
}

/// 库文件名常量（各用例都要拼路径）。
pub const FILE_NAME: &str = STORE_FILE_NAME;

/// 一张表的列名，**按 DDL 里的顺序**（`PRAGMA table_info` 的第 2 列）。
pub fn columns_of(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

/// 库里的每一个值：`(表, 列, 值)`。
///
/// 不预设任何列 —— 表清单来自 `sqlite_master`、列清单来自 `PRAGMA table_info`，
/// 所以**将来加的列也在检查范围内**。这正是 P2 那条要求需要的东西："任何位置都不许
/// 出现我们的绝对路径"，而不是"我们记得检查的那几列里没有"。
pub fn all_values(conn: &Connection) -> Vec<(String, String, String)> {
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();

    let mut out = Vec::new();
    for table in tables {
        for column in columns_of(conn, &table) {
            let mut stmt = conn
                .prepare(&format!("SELECT \"{column}\" FROM \"{table}\""))
                .unwrap();
            let values = stmt
                .query_map([], |row| {
                    Ok(match row.get_ref(0)? {
                        ValueRef::Null => None,
                        ValueRef::Integer(i) => Some(i.to_string()),
                        ValueRef::Real(f) => Some(f.to_string()),
                        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
                        // BLOB 也要看内容：路径当然可以被存成一列 BLOB
                        ValueRef::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
                    })
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            for value in values.into_iter().flatten() {
                out.push((table.clone(), column.clone(), value));
            }
        }
    }
    out
}

/// 一页「静止不可读」的匿名内存：`(起始, 结束, VmFlags)`。只取 `---p` 且 ≤ 32 KiB 的段 ——
/// 我们的受保护页最大 16 KiB，而线程栈的 guard page 是几十 MB，一次就分开了。
///
/// `/proc/self/smaps` 的段头形如 `7f1f…-7f1f… ---p 00000000 00:00 0`，属性行**缩进**且是
/// `键: 值` —— 解析时别忘 `trim_start()`（`passphrase_contract.rs` 的第一版就栽在这里）。
#[cfg(target_os = "linux")]
pub fn noaccess_pages() -> Vec<(u64, u64, String)> {
    let text = fs::read_to_string("/proc/self/smaps").unwrap();
    let mut out: Vec<(u64, u64, String)> = Vec::new();
    let mut current: Option<usize> = None;

    for line in text.lines() {
        let is_header = !line.starts_with(char::is_whitespace)
            && line
                .split_whitespace()
                .next()
                .is_some_and(|range| range.contains('-'));
        if is_header {
            let mut fields = line.split_whitespace();
            let range = fields.next().unwrap_or_default();
            let perms = fields.next().unwrap_or_default();
            let mut ends = range.split('-');
            let start = u64::from_str_radix(ends.next().unwrap_or("0"), 16).unwrap_or(0);
            let end = u64::from_str_radix(ends.next().unwrap_or("0"), 16).unwrap_or(0);
            current = if perms == "---p" && end - start <= 32768 {
                out.push((start, end, String::new()));
                Some(out.len() - 1)
            } else {
                None
            };
        } else if let Some(i) = current
            && let Some(rest) = line.trim_start().strip_prefix("VmFlags:")
        {
            out[i].2 = rest.trim().to_string();
        }
    }
    out
}

/// `/proc/self/status` 的进程级 `VmLck`（kB）—— 全部已 `mlock` 的内存。
#[cfg(target_os = "linux")]
pub fn locked_kb() -> u64 {
    let text = fs::read_to_string("/proc/self/status").unwrap();
    text.lines()
        .find_map(|line| line.strip_prefix("VmLck:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}
