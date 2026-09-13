//! 四套池的 CRUD（plan 0403）：**密钥** / **ssh 配置** / **serial 配置** / **端口转发规则**。
//!
//! 库是**唯一真相源**，不是缓存（`scope.md` §3）。所以这一层的形状是"直接把行拿出来 /
//! 放回去"，没有内存副本、没有脏标记、没有"稍后写回"。
//!
//! ⚠️ **例外一处的措辞**：[`known_hosts`] 是**缓存**（plan 0503）—— 它确实随库走
//! （可搬迁的数据目录），但它不是用户配置的东西，而是我们替他记住的主机密钥。
//! 放在同一个模块里是因为它同样是"库里的行 + 同样的不变量写法"，不是因为它是第 5 套池。
//!
//! ## 三条贯穿四套池的约定
//!
//! 1. **`New*` 与入库后的 `*` 分开**：未入库的行没有 `id`，这件事是类型事实而不是
//!    `Option<i64>`（`Option` 会把"没有 id 时怎么办"这个问题复制到每个调用点）。
//! 2. **不变量写到库的 `CHECK` / 外键上**，而不是只写在这里的校验函数里（见
//!    [`crate::schema`]）：库自己拦下来的东西，绕过任何代码路径都拦得住。
//!    这一层负责的是**把 sqlite 的失败翻成人能读的说法**（[`StoreError::Conflict`]）。
//! 3. **枚举在库里是文本**（`'publickey'` 这种），读回来时**不认识就报错**、不给默认值 ——
//!    默认值会让"库里有一个本程序不认识的取值"变成一件静默的事。
//!
//! 四套池的字段集与理由见 `docs/scope.md` §3；这里只实现，不重新论证。

pub mod forwards;
pub mod hosts;
pub mod keys;
pub mod known_hosts;
pub mod serial;

use rusqlite::types::Type;

use crate::StoreError;

/// 跳板链的深度上限。
///
/// 链本身**不该**成环（写入时挡着，见 [`hosts`]），所以这个数只是"万一库里真被改出环来，
/// 不要死循环"的兜底。取 32 不是为了限制用户配置跳板跳板跳板 —— 那种链本来就该在
/// UI 上被看得见，而不是悄悄连出去。
const MAX_JUMP_DEPTH: usize = 32;

/// 执行一条写语句，并把 sqlite 的失败翻成池的说法。
///
/// 三种失败各归各的（`docs/plans/0403` 的"设计要点"第 3 条）：
/// `UNIQUE` 重名、`CHECK` 不变量、`FOREIGN KEY` 还被引用 —— 都是
/// [`StoreError::Conflict`]，因为**用户的下一步动作是同一个**：改这一行，或者先去掉引用它的东西。
/// 分开成三个变体只会让调用方多写三个分支，却提供不了不同的动作。
fn constrained<T>(pool: &'static str, result: rusqlite::Result<T>) -> Result<T, StoreError> {
    result.map_err(|err| match &err {
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StoreError::Conflict {
                pool,
                detail: err.to_string(),
            }
        }
        _ => StoreError::Sqlite(err),
    })
}

/// `UPDATE` / `DELETE` 影响 0 行 = **那一行不在**（不是"值没变"：sqlite 把"匹配到但写回同样的值"
/// 也算作已修改）。所以这个判据不会误报。
fn touched(pool: &'static str, id: i64, rows: usize) -> Result<(), StoreError> {
    if rows == 0 {
        Err(StoreError::NoSuchRow { pool, id })
    } else {
        Ok(())
    }
}

/// 把一列文本翻成枚举：不认识就**报错**。
///
/// 列号用 `usize::MAX`（上游 `rusqlite` 对"列号不知道"用的就是这个哨兵，
/// 且它的 `Display` 遇到这个值就只打内层消息）—— 这个错误里有用的是那句话，
/// 不是列号。
fn read_enum<T>(
    raw: String,
    column: &'static str,
    parse: fn(&str) -> Option<T>,
) -> rusqlite::Result<T> {
    parse(&raw).ok_or_else(|| conversion(format!("unknown {column} value {raw:?}")))
}

/// 构造一条"列值无法转换"的 sqlite 错误（读路径上唯一的用法：枚举取值不认识）。
fn conversion(message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(usize::MAX, Type::Text, message.into())
}

/// 每个连接都要开一次：`PRAGMA foreign_keys` 是**连接级**的，且**默认关闭** ——
/// DDL 里写了 `REFERENCES` 不等于它会生效。不开的表现不是报错，而是静默的
/// "引用完整性看着没问题"（删掉还在被引用的密钥会成功，留下悬空的 `key_id`）。
///
/// 由 `tests/schema_contract.rs` 读回来断言（"我们以为开了" 不等于 "真开了"）。
/// 它不读库，所以放在 [`crate::unlock`] 里不违反 D4 的顺序要求。
pub(crate) fn enable_foreign_keys(conn: &rusqlite::Connection) -> Result<(), StoreError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn an_unknown_value_is_an_error_not_a_default() {
        // 三个枚举各自的 `parse` 在池模块里断言；这里断言**翻译**那一步：
        // 认不出来就必须是错误，而不是被吞成某个默认取值。
        let err = read_enum::<u8>("nope".to_string(), "direction", |_| None).unwrap_err();
        assert!(
            err.to_string().contains("unknown direction value \"nope\""),
            "错误消息要说出是哪一列、什么值：{err}"
        );
    }

    #[test]
    fn a_known_value_comes_back() {
        let value = read_enum("ok".to_string(), "direction", |raw| {
            (raw == "ok").then_some(7)
        })
        .unwrap();
        assert_eq!(value, 7);
    }

    #[test]
    fn zero_rows_touched_is_a_missing_row() {
        assert!(matches!(
            touched("hosts", 3, 0),
            Err(StoreError::NoSuchRow {
                pool: "hosts",
                id: 3
            })
        ));
        assert!(touched("hosts", 3, 1).is_ok());
    }
}
