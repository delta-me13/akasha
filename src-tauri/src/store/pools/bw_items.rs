//! **Bitwarden 导入池**（`scope.md` §7，plan 0903）：导入进来的那几行**来历**。
//!
//! ## 它为什么不是第五套池
//!
//! 四套池是"用户可增删改查的配置"，而这里的行**由导入产生**：`bw` 说有什么，这里就记什么。
//! 用户能做的是"把某一条导入进来"和"把某一行的钥匙删掉"——后者由 `ON DELETE CASCADE`
//! 顺手完成（[`crate::store::schema`] 的 `DDL_V3` 写了理由）。所以这个模块只做两件事：
//! **落一批快照**（私钥进 [`super::keys`]、来历进这张表）与**读回来**。
//!
//! ## 为什么私钥落进 `keys` 而不是这里
//!
//! `scope.md` §7 要求"导入后可用该密钥建立 SSH 连接"，而主机引用钥匙的唯一方式是
//! `hosts.key_id` → `keys.id`（`crate::store::pools::hosts`）。私钥若住在另一张表里，
//! 连接那条路就得知道两种钥匙 —— 那是把"哪把钥匙"这件事复制成两份。
//!
//! 于是导入是**快照**（`scope.md` §7 的"与本地密钥池之间没有任何同步机制"）：
//! 私钥的副本进 `keys`，来历记在这里。重新导入是一次显式的用户动作（`overwrite`），
//! 没有任何自动回流。
//!
//! ## 同名怎么办
//!
//! 与 `~/.ssh/config` 导入同一口径（[`super::import`]）：**默认不动**已有的同名行，
//! `overwrite` 才整行替换。另一条同名是**这一批里自己撞了** —— 上游允许两个条目同名，
//! 而池里的 `name` 是 `UNIQUE`，那种情况必须报出来（[`Skip::DuplicateName`]）：
//! 静默让后者盖掉前者，等于用户的两把钥匙里少了一把而没有任何痕迹。

use std::collections::{BTreeMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use super::constrained;
use super::keys::{self, Key, NewKey, PrivateKey};
use crate::store::StoreError;

/// 池名。写死成常量而不是让调用方拼字符串：错误消息里的池名必须与实际表名一致。
const POOL: &str = "bw_items";

/// 要导入的一条 **SSH key 快照**（还没有落库）。
///
/// 三个上游字段（`cipher_id` / `revision_date` / `fingerprint`）都是**原样文本**：
/// 我们不解释 UUID、不解析时间戳 —— 需要的只是"是不是同一条、变过没有"这两个比较。
pub struct Incoming {
    /// 上游那一条的 id（UUID 文本）。刷新判据按它比对。
    pub cipher_id: String,
    /// 上游条目名 —— 它就是池里的 `keys.name`。
    pub name: String,
    /// 上游的 `revisionDate`（ISO 8601 文本，原样）。
    pub revision_date: String,
    /// 上游报的 `SHA256:…`（完整性自检的参照，plan 0904）。
    pub fingerprint: String,
    /// 公钥（`sshKey.publicKey`，原样）。
    pub public_key: String,
    /// 私钥本体。它进 `keys` 时不过普通堆（[`PrivateKey`] 自己保证）。
    pub private: PrivateKey,
}

/// 导入池里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub cipher_id: String,
    pub name: String,
    pub revision_date: String,
    pub fingerprint: String,
    /// 这份快照落在 `keys` 的哪一行。
    pub key_id: i64,
}

/// 落库的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// 新进来的。
    pub created: Vec<Row>,
    /// 被 `overwrite` 换掉的。
    pub replaced: Vec<Row>,
    /// 看见了、但**没动**的。
    pub skipped: Vec<(String, Skip)>,
}

/// 池里的一行（过报告用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: i64,
    pub name: String,
}

/// 没动这一条的原因。**三个取值对应三个不同的下一步动作**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// 池里已经有同名的行了（想换掉它就带 `overwrite` 再来一次）。
    Exists,
    /// **这一批里**已经有同名的了。改上游的条目名，或者分开导 —— `overwrite` 也救不了：
    /// 让它盖掉前一条，用户的两把钥匙就少了一把。
    DuplicateName,
    /// 池里那一行**已经归另外一条上游条目了**。上游允许两个条目同名，而池里的名字是
    /// `UNIQUE`、一条钥匙也只能有一条来历 —— 让后来者顶掉前者，等于把先导入的那条
    /// 悄悄从"有来历"变成"没来历"。改上游的条目名，或者先删掉池里那一行。
    Claimed,
}

/// 一批快照落库：私钥进 `keys`、来历进 `bw_items`。**一次事务**（任何一条失败整批回滚）。
///
/// `overwrite` = `false`（默认）：同名已存在就**不动它**，只在结果里列出来。
pub fn import_snapshot(
    conn: &Connection,
    items: &mut [Incoming],
    overwrite: bool,
) -> Result<Outcome, StoreError> {
    let tx = conn.unchecked_transaction()?;
    let mut by_name: BTreeMap<String, i64> = keys::keys(&tx)?
        .into_iter()
        .map(|row| (row.name, row.id))
        .collect();
    let mut outcome = Outcome::default();
    // 这一批里出现过的名字。与 `by_name` 分开：后者在插入之后也会变，
    // 分不出"池里本来就有"与"这一批刚刚进来的"。
    let mut seen: HashSet<String> = HashSet::new();

    for item in items.iter_mut() {
        if !seen.insert(item.name.clone()) {
            outcome
                .skipped
                .push((item.name.clone(), Skip::DuplicateName));
            continue;
        }
        match by_name.get(&item.name).copied() {
            Some(_) if !overwrite => {
                outcome.skipped.push((item.name.clone(), Skip::Exists));
            }
            Some(id) => {
                // 名字对上了，但那一行可能已经归**另一条**上游条目。`bw_items.key_id` 是
                // `UNIQUE`（一把钥匙只有一条来历），所以这里不能直接顶掉 —— 那会让先导入的
                // 那一条从"有来历"变成"没来历"，而用户不会看到任何提示。
                if let Some(owner) = owner_of(&tx, id)?
                    && owner != item.cipher_id
                {
                    outcome.skipped.push((item.name.clone(), Skip::Claimed));
                    continue;
                }
                // 只换"上游说了算"的那两样：私钥与公钥。**备注留着** ——
                // 它是用户的，不是上游的，换一把私钥没有理由抹掉它。
                let current = keys::key(&tx, id)?;
                keys::set_private_key(&tx, id, &mut item.private)?;
                keys::update_key(
                    &tx,
                    &Key {
                        id,
                        name: item.name.clone(),
                        public_key: item.public_key.clone(),
                        comment: current.comment,
                    },
                )?;
                record(&tx, item, id)?;
                outcome.replaced.push(Row {
                    id,
                    name: item.name.clone(),
                });
            }
            None => {
                let id = keys::insert_key(
                    &tx,
                    &NewKey {
                        name: item.name.clone(),
                        public_key: item.public_key.clone(),
                        comment: None,
                    },
                    &mut item.private,
                )?;
                by_name.insert(item.name.clone(), id);
                record(&tx, item, id)?;
                outcome.created.push(Row {
                    id,
                    name: item.name.clone(),
                });
            }
        }
    }

    tx.commit()?;
    Ok(outcome)
}

/// 这一行钥匙现在归哪条上游条目（没有来历行就是 `None`）。
fn owner_of(conn: &Connection, key_id: i64) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT cipher_id FROM bw_items WHERE key_id = ?1",
            [key_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// 写一行来历。同一条目再导一次是**更新**（`cipher_id` 是主键），不是第二条。
fn record(conn: &Connection, item: &Incoming, key_id: i64) -> Result<(), StoreError> {
    constrained(
        POOL,
        conn.execute(
            "INSERT INTO bw_items (cipher_id, name, revision_date, fingerprint, key_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (cipher_id) DO UPDATE SET
                 name = excluded.name,
                 revision_date = excluded.revision_date,
                 fingerprint = excluded.fingerprint,
                 key_id = excluded.key_id",
            params![
                item.cipher_id,
                item.name,
                item.revision_date,
                item.fingerprint,
                key_id
            ],
        ),
    )?;
    Ok(())
}

/// 导入池的全部行，按名字排序（顺序确定，才谈得上"可断言的输出"）。
pub fn items(conn: &Connection) -> Result<Vec<Item>, StoreError> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY name"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 某一条（按上游的 id）。
pub fn item(conn: &Connection, cipher_id: &str) -> Result<Item, StoreError> {
    conn.query_row(
        &format!("{SELECT} WHERE cipher_id = ?1"),
        [cipher_id],
        from_row,
    )
    .optional()?
    .ok_or(StoreError::NoSuchRow {
        pool: POOL,
        // `cipher_id` 是文本而不是行 id，这里只能报 0 —— 消息里那句话仍然说得清是哪一条。
        id: 0,
    })
}

/// 抹掉一条来历（**不删私钥**）。
///
/// 用在"这条快照不再有用了，但钥匙留着自己用"的场合。删钥匙走 [`keys::delete_key`]，
/// 那时这一行由 `ON DELETE CASCADE` 自动消失。
///
/// 不用 [`touched`]：它把"影响 0 行"翻成 [`StoreError::NoSuchRow`]，而那个变体要的是
/// **行 id**，这张表的主键是上游的 `cipher_id`（文本）—— 报一个 `id: 0` 出来只会误导。
/// "那条来历本来就不在"在这里也不是错误。
pub fn forget(conn: &Connection, cipher_id: &str) -> Result<(), StoreError> {
    constrained(
        POOL,
        conn.execute("DELETE FROM bw_items WHERE cipher_id = ?1", [cipher_id]),
    )?;
    Ok(())
}

/// 读行用 `SELECT *` + **按列名取值**（与其它池同一条理由：列清单只有 DDL 一份）。
const SELECT: &str = "SELECT * FROM bw_items";

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Item> {
    Ok(Item {
        cipher_id: row.get("cipher_id")?,
        name: row.get("name")?,
        revision_date: row.get("revision_date")?,
        fingerprint: row.get("fingerprint")?,
        key_id: row.get("key_id")?,
    })
}
