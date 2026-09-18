//! **serial 配置池**：端口 / 波特率 / 数据位 / 停止位 / 校验 / 流控（`scope.md` §3）。
//!
//! ⚠️ **`port` 是这里唯一像绝对路径的字段，而它不是我们的文件位置**：
//! Unix 上它就是 `/dev/ttyUSB0` 这种**设备名**，Windows 上是 `COM3`。P2 要求的是
//! "库里不存**我们的**绝对路径"（`portable.md` §2 第 2 条）—— 存数据目录、密钥文件路径
//! 那类东西，搬走文件夹后会**静默失效**。设备名换台机器本来就可能不存在，那不是"搬家
//! 之后才失效"，所以不在这条要求的射程里。这条界线由 `tests/no_absolute_paths.rs` 守着
//! （判据是"**列名**里不许出现 path/dir/file" + "任何值都不含我们的数据目录"）。
//!
//! 取值域写在 DDL 的 `CHECK` 里（`baud > 0`、`data_bits IN (5,6,7,8)`…）而不是这里：
//! 波特率**不**限制成"标准档位"—— 现实里有非标准波特率的设备，把它挡在外面等于让用户
//! 换个工具连。校验位与流控是枚举，所以两边都有。

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{constrained, read_enum, touched};
use crate::store::StoreError;

/// 池名。写死成常量而不是让调用方拼字符串：错误消息里的池名必须与实际表名一致。
const POOL: &str = "serials";

/// 校验位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

impl Parity {
    pub const ALL: [Self; 3] = [Self::None, Self::Even, Self::Odd];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Even => "even",
            Self::Odd => "odd",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|parity| parity.as_str() == raw)
    }
}

/// 流控。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    None,
    /// XON/XOFF（软件流控）。
    Software,
    /// RTS/CTS（硬件流控）。
    Hardware,
}

impl Flow {
    pub const ALL: [Self; 3] = [Self::None, Self::Software, Self::Hardware];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Software => "software",
            Self::Hardware => "hardware",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|flow| flow.as_str() == raw)
    }
}

/// serial 配置池里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Serial {
    pub id: i64,
    pub name: String,
    /// 操作系统给的设备名（见模块文档）。
    pub port: String,
    pub baud: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: Parity,
    pub flow: Flow,
}

/// 还没入库的一行（**没有 id**）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSerial {
    pub name: String,
    pub port: String,
    pub baud: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: Parity,
    pub flow: Flow,
}

/// 插入一行，返回新的 `id`。
pub fn insert_serial(conn: &Connection, new: &NewSerial) -> Result<i64, StoreError> {
    let rows = constrained(
        POOL,
        conn.execute(
            "INSERT INTO serials (name, port, baud, data_bits, stop_bits, parity, flow)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new.name,
                new.port,
                new.baud,
                new.data_bits,
                new.stop_bits,
                new.parity.as_str(),
                new.flow.as_str()
            ],
        ),
    )?;
    debug_assert_eq!(rows, 1, "INSERT 只该影响一行");
    Ok(conn.last_insert_rowid())
}

/// 读一行。
pub fn serial(conn: &Connection, id: i64) -> Result<Serial, StoreError> {
    conn.query_row("SELECT * FROM serials WHERE id = ?1", [id], from_row)
        .optional()?
        .ok_or(StoreError::NoSuchRow { pool: POOL, id })
}

/// 全部 serial 配置，按名字排序（顺序确定，才谈得上"可断言的输出"）。
pub fn serials(conn: &Connection) -> Result<Vec<Serial>, StoreError> {
    let mut stmt = conn.prepare("SELECT * FROM serials ORDER BY name")?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 全量替换一行。
pub fn update_serial(conn: &Connection, serial: &Serial) -> Result<(), StoreError> {
    let rows = constrained(
        POOL,
        conn.execute(
            "UPDATE serials SET name = ?1, port = ?2, baud = ?3, data_bits = ?4,
                    stop_bits = ?5, parity = ?6, flow = ?7
             WHERE id = ?8",
            params![
                serial.name,
                serial.port,
                serial.baud,
                serial.data_bits,
                serial.stop_bits,
                serial.parity.as_str(),
                serial.flow.as_str(),
                serial.id
            ],
        ),
    )?;
    touched(POOL, serial.id, rows)
}

/// 删一行。没有别的池引用 serial 配置，所以这里不会有外键冲突。
pub fn delete_serial(conn: &Connection, id: i64) -> Result<(), StoreError> {
    let rows = constrained(
        POOL,
        conn.execute("DELETE FROM serials WHERE id = ?1", [id]),
    )?;
    touched(POOL, id, rows)
}

/// 按**列名**读一行。
fn from_row(row: &Row<'_>) -> rusqlite::Result<Serial> {
    Ok(Serial {
        id: row.get("id")?,
        name: row.get("name")?,
        port: row.get("port")?,
        baud: row.get("baud")?,
        data_bits: row.get("data_bits")?,
        stop_bits: row.get("stop_bits")?,
        parity: read_enum(row.get("parity")?, "parity", Parity::parse)?,
        flow: read_enum(row.get("flow")?, "flow", Flow::parse)?,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn every_value_round_trips_through_its_text_form() {
        for parity in Parity::ALL {
            assert_eq!(Parity::parse(parity.as_str()), Some(parity));
        }
        for flow in Flow::ALL {
            assert_eq!(Flow::parse(flow.as_str()), Some(flow));
        }
    }

    #[test]
    fn an_unknown_value_is_not_a_default() {
        assert_eq!(Parity::parse("mark"), None);
        assert_eq!(Flow::parse("rts"), None, "取值是 'hardware'，不是引脚名");
    }
}
