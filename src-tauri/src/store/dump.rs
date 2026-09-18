//! **库里有什么** —— 可见性工具（plan 0404）。
//!
//! 它回答的是"我配的东西还在不在"与"这个库是不是我以为是的那一个"，用在诊断、导出前的
//! 自查、以及测试里"导出前后内容一致"那条判据的比对基准（比对**结构化数据**，
//! 不是比对文件字节 —— 页布局不是我们要承诺的东西）。
//!
//! ## 为什么它**结构上**不含机密
//!
//! 私钥不在 [`Key`] 里：要它得单独走 [`crate::store::pools::keys::private_key`]（出来即进受保护页）。
//! 这条设计是 0403 定的（"列出密钥不该顺手把每把私钥都读进 16 KiB 的 `mlock`"），
//! 到了 dump 这一层就成了**免费的性质**：想把私钥打出来，得先改类型。
//!
//! 这不是洁癖：dump 的天花板是"贴进 issue / 日志"，而库文件本身是密文 ——
//! 一个顺手泄密的 dump 会把整库的安全边界从"文件是密文"降到"用户没贴过 dump"。
//!
//! ## 文本形态是**给人的**，不是协议
//!
//! [`Dump::to_text`] 的输出没有格式承诺：它给人和日志看。要程序读，用 [`Dump`] 的字段
//! （将来 IPC 那条路会把它序列化成 JSON，那是 app 层的事）。

use std::fmt::Write as _;

use rusqlite::Connection;

use crate::store::StoreError;
use crate::store::pools::forwards::{self, Direction, Forward};
use crate::store::pools::hosts::{self, Host};
use crate::store::pools::keys::{self, Key};
use crate::store::pools::serial::{self, Parity, Serial};

/// 库里有什么：格式版本 + 四套池的全部行（**都不含机密**）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dump {
    /// `PRAGMA user_version` —— 读的是库里的真值，不是 [`crate::store::FORMAT_VERSION`]
    /// （诊断要回答的是"这个文件是什么"，不是"这个程序期望什么"）。
    pub format_version: i64,
    /// 密钥池的行。**不含私钥本体**（见模块文档）。
    pub keys: Vec<Key>,
    pub hosts: Vec<Host>,
    pub serials: Vec<Serial>,
    pub forwards: Vec<Forward>,
}

/// 读出库里全部内容。
///
/// 只读：不写、不建表、不改任何状态；四套池各自的排序由池模块定（顺序确定才谈得上比对）。
pub fn dump(conn: &Connection) -> Result<Dump, StoreError> {
    Ok(Dump {
        format_version: conn.query_row("PRAGMA user_version", [], |row| row.get(0))?,
        keys: keys::keys(conn)?,
        hosts: hosts::hosts(conn)?,
        serials: serial::serials(conn)?,
        forwards: forwards::forwards(conn)?,
    })
}

impl Dump {
    /// 四套池的行数，顺序是密钥 / 主机 / 串口 / 转发。
    ///
    /// 单独给一个小函数，是因为"这个库是空的吗"是最常问的那一句 ——
    /// 而它不该逼调用方去数四个 `Vec`。
    pub fn row_counts(&self) -> [usize; 4] {
        [
            self.keys.len(),
            self.hosts.len(),
            self.serials.len(),
            self.forwards.len(),
        ]
    }

    /// 给人看的形态。
    ///
    /// 列出来的都是**可以公开展示**的东西：名称、地址、端口、枚举取值、**公开**密钥。
    /// 私钥写不出来（[`Key`] 里没有它）。
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "format: v{}", self.format_version);

        let _ = writeln!(out, "keys: {}", self.keys.len());
        for key in &self.keys {
            let _ = writeln!(
                out,
                "  [{}] {} pubkey={} comment={}",
                key.id,
                key.name,
                quoted_unless_empty(&key.public_key),
                optional(&key.comment),
            );
        }

        let _ = writeln!(out, "hosts: {}", self.hosts.len());
        for host in &self.hosts {
            let _ = writeln!(
                out,
                "  [{}] {} {}:{} user={} auth={} key={} jump={}",
                host.id,
                host.name,
                host.host,
                host.port,
                host.user,
                host.auth.as_str(),
                optional_id(host.key_id),
                optional_id(host.jump_id),
            );
        }

        let _ = writeln!(out, "serials: {}", self.serials.len());
        for serial in &self.serials {
            let _ = writeln!(
                out,
                "  [{}] {} {} {} {}{}{} flow={}",
                serial.id,
                serial.name,
                serial.port,
                serial.baud,
                serial.data_bits,
                parity_letter(serial.parity),
                serial.stop_bits,
                serial.flow.as_str(),
            );
        }

        let _ = writeln!(out, "forwards: {}", self.forwards.len());
        for forward in &self.forwards {
            let _ = writeln!(
                out,
                "  [{}] {} {} {}:{} -> {} host={} autostart={}",
                forward.id,
                forward.name,
                forward.direction.as_str(),
                forward.bind_host,
                forward.bind_port,
                target(forward),
                forward.host_id,
                if forward.autostart { "yes" } else { "no" },
            );
        }

        out
    }
}

/// `-` 表示没有；有就加引号（文本里可能带空格，不加引号读不出边界）。
fn optional(value: &Option<String>) -> String {
    match value {
        Some(text) => format!("{text:?}"),
        None => "-".to_string(),
    }
}

/// 公开密钥为空是实现允许的（`public_key` 有 `DEFAULT ''`）：空就写 `-`，不写一对空引号。
fn quoted_unless_empty(value: &str) -> String {
    if value.is_empty() {
        "-".to_string()
    } else {
        format!("{value:?}")
    }
}

/// 外键：没有就写 `-`（`Some(-1)` 这种事在库里不可能发生，外键拦着）。
fn optional_id(id: Option<i64>) -> String {
    match id {
        Some(id) => id.to_string(),
        None => "-".to_string(),
    }
}

/// `dynamic` 没有目标（DDL 的 `CHECK` 保证"有方向就有目标"是双向成立的）。
fn target(forward: &Forward) -> String {
    match (&forward.target_host, forward.target_port) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        _ => match forward.direction {
            Direction::Dynamic => "- (client chooses)".to_string(),
            _ => "-".to_string(),
        },
    }
}

/// `8N1` 里的那个字母。
fn parity_letter(parity: Parity) -> char {
    match parity {
        Parity::None => 'N',
        Parity::Even => 'E',
        Parity::Odd => 'O',
    }
}
