//! `Passphrase` —— 口令在进程里的**唯一形态**（ADR-0002 D5 的落地）。
//!
//! D5 说的是六件事："只以字节缓冲存在于内存、不进 `String`、不进配置文件、不进环境变量、
//! 不作为命令行参数、永远不进日志"。这些话写在文档里守不住（下一个人加一个
//! `#[derive(Debug)]` 就漏了），所以这里把它们变成**类型事实**：
//!
//! | D5 的那一句 | 由什么保证 |
//! |---|---|
//! | 空口令在应用层被拒绝 | 空值**造不出来** —— [`Passphrase::new`] 是唯一的构造口 |
//! | 永远不进日志 | 没有 `Debug` / `Display`（`memsafe::Secret` 本身就不实现，于是 `{:?}` 是**编译错误**而不是打码） |
//! | 不进配置文件 / 环境变量 / 命令行 | 本 crate 没有配置、`env`、argv 的读取口（`cargo tree` 可证，见 plan 0402 验收命令） |
//! | 只以字节缓冲存在、不进 `String` | 字节直接写进下面那**一页受保护内存**，从不经过 `String` |
//!
//! ## 内存里怎么护住它（`memsafe`，plan 0406 / 0403）
//!
//! 口令放在 [`memsafe::Secret`] 的**一整页受保护内存**里，而不是普通堆上。这件事的
//! 实现搬去了 [`crate::protected`]（0403 起私钥也要用它，两份实现必然漂移），
//! 实测与边界见那个模块与 ADR-0002 §7.2 / D13。这里只留口令特有的两条：
//!
//! 1. **空口令不可表示**（D5）—— 不是"打开函数里有个 if"，而是造不出空值。
//!    空 key 送进 `sqlite3_key()` 的后果是**得到一个明文库**（D5 / §7 实测）；
//! 2. **上限 256 字节**（[`MAX_LEN`]）—— 超了是**明确报错**，不是截断。
//!
//! 还有一条与安全无关但要认的：**`mlock` 失败会让构造直接失败**（上游没有降级路径）——
//! 也就是说 `Passphrase::new` 返回 `Err` 时，解锁是**不能进行**的，而不是"悄悄不锁"。
//! 这个取舍属于 D5，已记入 ADR-0002 §10。
//!
//! 把字节交出去的唯一入口是 [`Passphrase::expose`]，它是 `pub(crate)`：口令能流向哪里，
//! 在本 crate 里数得出来（`open` / `create` 各一处），reviewer 一眼看得见。

use crate::StoreError;
use crate::protected::{Exposed, PageError, Protected};

/// 口令的字节上限 —— 也就是受保护页的大小。
///
/// 256 是"够用且不必权衡"的取值：用户输入的口令、粘贴的 64 位十六进制密钥都在里面，
/// 而每多一字节就多一字节被 `mlock` 的内存（整页粒度，4 KiB）。超了是**明确报错**，
/// 不是截断 —— 截断会让"口令错了"变成一件没人能解释的事。
pub const MAX_LEN: usize = 256;

/// `sqlite3_key` 的长度参数是 `c_int`。256 远小于它，这条断言把"不会被截断"钉在编译期，
/// 于是 `apply_key` 里那条 `try_from` 的失败分支是**逻辑上不可达**的兜底。
const _: () = assert!(MAX_LEN <= i32::MAX as usize);

/// 用户口令。**空值不可表示**（ADR-0002 D5）。
///
/// 刻意不实现 `Clone`：口令的副本只有一个来源，多一个副本就得回答"为什么要多这一个"。
/// 需要多次使用时传 `&mut`（`open` / `create` 都是 `&mut Passphrase`）——
/// `&mut` 是刻意的：**读它是一次需要独占的提权动作**，交出引用就等于把守卫交给调用方。
pub struct Passphrase {
    page: Protected<MAX_LEN>,
}

impl Passphrase {
    /// 从字节造一个口令。**空字节是错误**，不是"没设口令"。
    ///
    /// 这一条为什么必须在类型上而不是在打开函数里：实测（ADR-0002 §7）空 key 送进
    /// `sqlite3_key()` 会让连接**退化成明文库**，而此后每一步都"成功" ——
    /// 一个能被误传的空值，迟早会被误传。造不出来就不会传错。
    ///
    /// 成功时**源缓冲已被擦零**（[`Protected::new`] 那一侧做的）。
    pub fn new(bytes: Vec<u8>) -> Result<Self, StoreError> {
        if bytes.is_empty() {
            return Err(StoreError::EmptyPassphrase);
        }
        let page = Protected::new(bytes).map_err(as_passphrase_error)?;
        Ok(Self { page })
    }

    /// 临时取得口令字节。返回的守卫 drop 时向 OS 交还权限（Unix 上是回到 `PROT_NONE`）。
    ///
    /// `pub(crate)` 是刻意的：这是口令流出本模块的**唯一**通道，而且只对本 crate 开放。
    pub(crate) fn expose(&mut self) -> Result<Exposed<'_, MAX_LEN>, StoreError> {
        self.page.expose().map_err(as_passphrase_error)
    }
}

/// 把 [`PageError`] 翻成口令这一侧的说法。
///
/// 翻译写成函数而不是散在两处 `match`：新增一个 [`PageError`] 变体时，这里会**编译失败**，
/// 而散在两处的话很可能只有一处被想起来。
fn as_passphrase_error(err: PageError) -> StoreError {
    match err {
        PageError::TooLong { max } => StoreError::PassphraseTooLong { max },
        PageError::Memory(err) => StoreError::MemoryProtection(err),
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
    fn bytes_come_back_byte_for_byte() {
        // 任意字节都是合法口令（D5 的理由之一：口令可以不是 UTF-8）。非 UTF-8 与内嵌 NUL
        // 都必须原样回来 —— 这也正是 D4 坚持走 C API（显式长度）而不是 `PRAGMA key = '…'` 的原因。
        for bytes in [
            vec![b'x'],
            b"correct horse battery staple".to_vec(),
            vec![0x00],
            vec![0xff, 0xfe, 0x00, 0x27],
            vec![b'z'; MAX_LEN],
        ] {
            let expected = bytes.clone();
            let mut passphrase = Passphrase::new(bytes).unwrap();
            assert_eq!(&*passphrase.expose().unwrap(), &expected[..]);
        }
    }

    #[test]
    fn one_byte_over_the_page_is_refused_not_truncated() {
        // 截断会让"口令错了"变成一件没人能解释的事，所以这里是明确报错。
        //
        // ⚠️ 这里写不出 `.unwrap_err()`：`Passphrase` 没有 `Debug`，而 `unwrap_err` 要求
        // `Ok` 一侧可打印 —— 这个**编译错误本身就是**"口令打不进日志"那条保证的证据
        // （`memsafe::Secret` 不给 `Debug`，我们也不给）。
        match Passphrase::new(vec![b'z'; MAX_LEN + 1]) {
            Err(StoreError::PassphraseTooLong { max }) => assert_eq!(max, MAX_LEN),
            Err(other) => panic!("错误类型不对：{other:?}"),
            Ok(_) => panic!("超过一页的口令不该造得出来"),
        }
    }
}
