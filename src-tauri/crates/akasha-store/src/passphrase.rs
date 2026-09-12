//! `Passphrase` —— 口令在进程里的**唯一形态**（ADR-0002 D5 的落地）。
//!
//! D5 说的是六件事："只以字节缓冲存在于内存、不进 `String`、不进配置文件、不进环境变量、
//! 不作为命令行参数、永远不进日志"。这些话写在文档里守不住（下一个人加一个
//! `#[derive(Debug)]` 就漏了），所以这里把它们变成**类型事实**：
//!
//! | D5 的那一句 | 由什么保证 |
//! |---|---|
//! | 空口令在应用层被拒绝 | 空值**造不出来** —— [`Passphrase::new`] 是唯一的构造口 |
//! | 永远不进日志 | 没有 `Display` / `Serialize`；`Debug` 只打 `<redacted>` |
//! | 不进配置文件 / 环境变量 / 命令行 | 本 crate 没有配置、`env`、argv 的读取口（`cargo tree` 可证，见 plan 0402 验收命令） |
//! | 只以字节缓冲存在、不进 `String` | 持有 `Vec<u8>` —— `String` 会强制 UTF-8，还会在堆上多留一份 |
//!
//! 把字节交出去的唯一入口是 [`Passphrase::expose`]，它是 `pub(crate)`：口令能流向哪里，
//! 在本 crate 里数得出来（`open` / `create` 各一处），reviewer 一眼看得见。
//! ⚠️ 将来的 IPC 边界（plan 0403）必然先拿到一个 TS 字符串 —— 那一份副本不在我们手里，
//! 我们只能保证**进来之后**只有这一个形态。这条边界照实写在这里，不假装不存在。
//!
//! **明确不做：内存擦除**（`zeroize` 那类）。库一解锁，私钥与解密后的页就同样躺在内存里，
//! 单独擦掉这一个输入缓冲不改变威胁模型 —— 真正的边界是"文件是密文"（D5 / D12）。
//! 做了的是 `cipher_memory_security`（擦 SQLCipher 自己分配的那一份，见 `lib.rs`）。
//! 不做是因为它**换不来安全**，不是因为忘了。

use std::fmt;

use crate::StoreError;

/// 用户口令。**空值不可表示**（ADR-0002 D5）。
///
/// 刻意不实现 `Clone`：口令的副本只有一个来源，多一个副本就得回答"为什么要多这一个"。
/// 需要多次使用时传引用（`open` / `create` 都是 `&Passphrase`）。
pub struct Passphrase(Vec<u8>);

impl Passphrase {
    /// 从字节造一个口令。**空字节是错误**，不是"没设口令"。
    ///
    /// 这一条为什么必须在类型上而不是在打开函数里：实测（ADR-0002 §7）空 key 送进
    /// `sqlite3_key()` 会让连接**退化成明文库**，而此后每一步都"成功" ——
    /// 一个能被误传的空值，迟早会被误传。造不出来就不会传错。
    pub fn new(bytes: Vec<u8>) -> Result<Self, StoreError> {
        if bytes.is_empty() {
            return Err(StoreError::EmptyPassphrase);
        }
        Ok(Self(bytes))
    }

    /// 把字节交给 `sqlite3_key()`。`pub(crate)` 是刻意的：这是口令流出本模块的**唯一**通道，
    /// 而且只对本 crate 开放（外部代码只能把 `&Passphrase` 交给 `open` / `create`）。
    pub(crate) fn expose(&self) -> &[u8] {
        &self.0
    }
}

/// 手写而不是 `derive`：`derive` 会把口令逐字节打进日志/panic 消息/错误链。
///
/// **不打长度**：长度也是信息，而它对诊断没有价值（唯一会被拒的情形是"空"，而空值
/// 根本走不到打印这一步）。
impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn empty_is_not_a_passphrase() {
        assert!(matches!(
            Passphrase::new(Vec::new()),
            Err(StoreError::EmptyPassphrase)
        ));
    }

    #[test]
    fn debug_never_prints_the_passphrase() {
        let secret = "hunter2-should-not-appear";
        let passphrase = Passphrase::new(secret.as_bytes().to_vec()).unwrap();
        let printed = format!("{passphrase:?} {passphrase:#?}");
        assert!(
            !printed.contains(secret),
            "口令被打进了 Debug 输出：{printed}"
        );
        assert!(printed.contains("redacted"), "应当明说被隐去了：{printed}");
    }

    #[test]
    fn any_byte_sequence_is_a_valid_passphrase() {
        // D5 的理由之一：口令可以是任意字节。非 UTF-8 与内嵌 NUL 都必须是合法口令 ——
        // 这也正是 D4 坚持走 C API（显式长度）而不是 `PRAGMA key = '…'` 的原因。
        for bytes in [vec![0x00], vec![0xff, 0xfe, 0x00, 0x27], vec![b'x'; 64]] {
            let len = bytes.len();
            assert_eq!(Passphrase::new(bytes).unwrap().expose().len(), len);
        }
    }
}
