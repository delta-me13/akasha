//! **密钥池**（本地）：私钥 / 公钥 / 备注 / 关联主机（`scope.md` §3）。
//!
//! 两件事与其它三池不同，都在这个模块里：
//!
//! 1. **私钥本体是机密**：它进库是 BLOB，**出库直接进受保护页**（ADR-0002 D13），
//!    从不以 `String` / `Vec<u8>` 的形态交给调用方（[`private_key`]）。
//! 2. **每一行都必须有私钥**：公钥可以不填（能从私钥推，或者用户懒得填），
//!    但"没有私钥的私钥行"是个自相矛盾的记录 —— 它在连接时才暴露（"这把钥匙怎么是空的"），
//!    所以在**构造**这一层就拒绝：空值造不出 [`PrivateKey`]。
//!
//! ## 上限 16 KiB 是怎么定的
//!
//! 受保护页的大小是 `N`（编译期常量），不是运行时可以随便挑的，所以要先回答
//! "私钥最长能有多长"。实测过的形态：RSA 4096 的 PEM ≈ 3.3 KB、ed25519 ≈ 400 B、
//! OpenSSH 新格式同量级、`openssh-key-v1` 带一段 comment 也不到 4 KB。
//! 取 **16384 = 4 KiB × 4 = 16 KiB 页 × 1**：两种常见页大小下都正好是整页，
//! 离"常见私钥"有一个数量级的余量。
//!
//! 代价照实记：`N` 是常量，所以**每读一把私钥就锁 16 KiB**（哪怕这把密钥只有 400 字节）。
//! 密钥是按需读的、数量是"用户有几把钥匙"级别，所以这个浪费换来的好处是
//! "上限写在一个地方、判据能重验一遍"（D13 对每个新用途的要求）。
//!
//! **写入侧先拒**：超过一页的私钥**根本进不了库** —— 否则库里会留下一条以后读不出来的
//! 记录，而那个错误要到连接时才出现。[`crate::store::schema`] 的 `private_pem BLOB NOT NULL`
//! 拦不住长度，所以这条由 [`PrivateKey::new`] 保证。

use std::ops::Deref;

use rusqlite::{Connection, OptionalExtension, params};

use super::hosts::Host;
use super::{constrained, touched};
use crate::store::StoreError;
use crate::store::protected::{PageError, Protected};

/// 池名。写死成常量而不是让调用方拼字符串：错误消息里的池名必须与实际表名一致。
const POOL: &str = "keys";

/// 私钥 PEM 的字节上限 —— 也就是受保护页的大小。取值理由见模块文档。
pub const MAX_PEM_LEN: usize = 16384;

/// 一把私钥。**只能从字节造、只能经 [`PrivateKey::expose`] 读**。
///
/// 刻意不实现 `Clone` / `Debug` / `Display`（与 [`crate::store::Passphrase`] 同一条理由）：
/// 副本只有一个来源，而打不出来是**编译错误**而不是打码。
pub struct PrivateKey {
    page: Protected<MAX_PEM_LEN>,
}

impl PrivateKey {
    /// 把 PEM 字节搬进受保护页。**空字节是错误**（见模块文档第 2 条）。
    ///
    /// 成功时源缓冲已被擦零（[`Protected::new`] 那一侧做的）。
    pub fn new(pem: Vec<u8>) -> Result<Self, StoreError> {
        if pem.is_empty() {
            return Err(StoreError::EmptyPrivateKey);
        }
        let page = Protected::new(pem).map_err(as_key_error)?;
        Ok(Self { page })
    }

    /// PEM 的字节数（不是页大小）。
    ///
    /// 名字里带 `byte_` 是刻意的：`len()` 这个名字在 Rust 里属于"容器"，于是
    /// `clippy::len_without_is_empty` 会要求配一个 `is_empty` —— 而**空私钥在这里
    /// 造不出来**（[`PrivateKey::new`] 拒绝空字节），那个方法只会永远返回 `false`。
    /// 提供一个永远说假话的方法，比换一个名字坏。
    pub fn byte_len(&self) -> usize {
        self.page.byte_len()
    }

    /// 返回一个**提权窗口**：读到的东西只在守卫活着时有效，drop 即降权（Unix 上回到 `PROT_NONE`）。
    ///
    /// 返回 `impl Deref<Target = [u8]>` 而不是那个守卫类型，是为了让守卫类型留在本 crate 里 ——
    /// 调用方能读、不能命名、更不能把它存起来当普通切片用（那就等于把提权窗口延长到守卫之外）。
    pub fn expose(&mut self) -> Result<impl Deref<Target = [u8]> + '_, StoreError> {
        self.page.expose().map_err(as_key_error)
    }
}

/// 把 [`PageError`] 翻成密钥这一侧的说法（与口令那一侧的 [`crate::store::passphrase`] 对称）。
fn as_key_error(err: PageError) -> StoreError {
    match err {
        PageError::TooLong { max } => StoreError::SecretTooLong { max },
        PageError::Memory(err) => StoreError::MemoryProtection(err),
    }
}

/// 密钥池里的一行 —— **不含私钥本体**。
///
/// 列出密钥时不该顺手把每把私钥都读进受保护页（那是 N × 16 KiB 的 `mlock`，而且
/// 每一次都是"本可以不做"的提权）。要私钥就单独 [`private_key`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub id: i64,
    pub name: String,
    pub public_key: String,
    pub comment: Option<String>,
}

/// 还没入库的一行（**没有 id**）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewKey {
    pub name: String,
    pub public_key: String,
    pub comment: Option<String>,
}

/// 插入一行，返回新的 `id`。
pub fn insert_key(
    conn: &Connection,
    new: &NewKey,
    private: &mut PrivateKey,
) -> Result<i64, StoreError> {
    let pem = private.expose()?;
    let rows = constrained(
        POOL,
        conn.execute(
            "INSERT INTO keys (name, private_pem, public_key, comment) VALUES (?1, ?2, ?3, ?4)",
            params![new.name, &*pem, new.public_key, new.comment],
        ),
    )?;
    debug_assert_eq!(rows, 1, "INSERT 只该影响一行");
    Ok(conn.last_insert_rowid())
}

/// 读一行（**不含私钥**）。
pub fn key(conn: &Connection, id: i64) -> Result<Key, StoreError> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)
        .optional()?
        .ok_or(StoreError::NoSuchRow { pool: POOL, id })
}

/// 全部密钥，按名字排序（顺序确定，才谈得上"可断言的输出"）。
pub fn keys(conn: &Connection) -> Result<Vec<Key>, StoreError> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY name"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 按**名字**找一行（找不到是 `None`，不是错误）。
///
/// 谁在用这条：`~/.ssh/config` 导入时，`IdentityFile` 的 basename 与池里的名字**逐字符
/// 相同**就把 `key_id` 接上（plan 0903）。它是"看起来像同名"而不是"猜一个"：名字是
/// 池里的 `UNIQUE` 列，匹配不上时行为与不加这条规则时**完全一致**。
pub fn find_by_name(conn: &Connection, name: &str) -> Result<Option<i64>, StoreError> {
    Ok(conn
        .query_row("SELECT id FROM keys WHERE name = ?1", [name], |row| {
            row.get(0)
        })
        .optional()?)
}

/// 读某一把私钥 —— **出库即进受保护页**（ADR-0002 D13）。
///
/// ⚠️ 这份私钥在自己的生命期里经过了两块普通内存，照实记：
/// ① SQLCipher 内部的行缓冲（它走 SQLCipher 自己的安全分配器 —— 我们每个连接都先开
/// `cipher_memory_security`，见 [`crate::store::unlock`]）；② 从行缓冲拷出来的那个 `Vec<u8>`，
/// 它在 [`PrivateKey::new`] 里被**擦零**。擦不掉的只有 ①，而它不归我们管。
pub fn private_key(conn: &Connection, id: i64) -> Result<PrivateKey, StoreError> {
    let pem: Vec<u8> = conn
        .query_row("SELECT private_pem FROM keys WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional()?
        .ok_or(StoreError::NoSuchRow { pool: POOL, id })?;
    PrivateKey::new(pem)
}

/// 换掉某一把私钥（换密钥时用；改名字用 [`update_key`]）。
pub fn set_private_key(
    conn: &Connection,
    id: i64,
    private: &mut PrivateKey,
) -> Result<(), StoreError> {
    let pem = private.expose()?;
    let rows = constrained(
        POOL,
        conn.execute(
            "UPDATE keys SET private_pem = ?1 WHERE id = ?2",
            params![&*pem, id],
        ),
    )?;
    touched(POOL, id, rows)
}

/// 全量替换一行的元数据（私钥不在里面 —— 它有 [`set_private_key`]）。
pub fn update_key(conn: &Connection, key: &Key) -> Result<(), StoreError> {
    let rows = constrained(
        POOL,
        conn.execute(
            "UPDATE keys SET name = ?1, public_key = ?2, comment = ?3 WHERE id = ?4",
            params![key.name, key.public_key, key.comment, key.id],
        ),
    )?;
    touched(POOL, key.id, rows)
}

/// 删一行。**还在被 host 引用时删不掉**（外键 `ON DELETE RESTRICT` → [`StoreError::Conflict`]）：
/// 那是明确报错，而不是把 host 的 `key_id` 悄悄清空。
pub fn delete_key(conn: &Connection, id: i64) -> Result<(), StoreError> {
    let rows = constrained(POOL, conn.execute("DELETE FROM keys WHERE id = ?1", [id]))?;
    touched(POOL, id, rows)
}

/// 用了这把密钥的主机（`scope.md` §3 密钥池那一行的"关联主机"）——
/// 关系存在 host 那一侧（`hosts.key_id`），因为 `~/.ssh/config` 就是这么表达的。
pub fn hosts_using_key(conn: &Connection, key_id: i64) -> Result<Vec<Host>, StoreError> {
    let mut stmt = conn.prepare("SELECT * FROM hosts WHERE key_id = ?1 ORDER BY name")?;
    let rows = stmt.query_map([key_id], super::hosts::from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 读行用 `SELECT *` + **按列名取值**：列清单只有 DDL 一份，读的地方不会再抄一份
/// （抄的那份迟早与 DDL 不一致，而"字段串了"这种错很贵）。`STRICT` 表 + 按名取值
/// 让 `SELECT *` 没有它那两条经典风险（顺序敏感 / 多出来的列进内存）。
const SELECT: &str = "SELECT * FROM keys";

/// 按**列名**读一行。缺列或改名会在读的那一刻报错，而不是悄悄换个值。
fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Key> {
    Ok(Key {
        id: row.get("id")?,
        name: row.get("name")?,
        public_key: row.get("public_key")?,
        comment: row.get("comment")?,
    })
}
