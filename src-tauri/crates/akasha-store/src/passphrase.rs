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
//! | 只以字节缓冲存在、不进 `String` | 字节直接写进**受保护页**（下节），从不经过 `String` |
//!
//! ## 内存里怎么护住它（`memsafe`，plan 0406）
//!
//! 口令放在 [`memsafe::Secret`] 的**一整页受保护内存**里，而不是普通堆上。实测与上游源码
//! 确认它做了这几件事：
//!
//! - Unix 上静止态是 **`PROT_NONE`**（读也不行）：要看得走 [`Passphrase::expose`] 拿一个
//!   临时提权的守卫，守卫一 drop 就降回去；
//! - **`mlock`**（不进 swap），Linux 上另有 **`MADV_DONTDUMP`**（不进 core dump）
//!   与 `MADV_WIPEONFORK`；
//! - 释放时**逐字节 volatile 写零**再 `munmap`（编译器不能把它优化掉）；
//! - 源缓冲（调用方那个 `Vec<u8>`）在拷贝进受保护页之后也被 **volatile 擦零**
//!   —— 这是普通 `zeroize` 用法最容易漏掉的一份副本。
//!
//! ⚠️ **它挡不住的四件事，照实记**（免得把"有防护"读成"没风险"）：
//!
//! 1. **Windows 上静止态是 `PAGE_READONLY`**（可读、不可写），不是 Unix 那样的全禁 ——
//!    只有 Windows 的运行期语义与 Unix 不同，别把 Linux 的结论搬过去；
//! 2. **macOS 没有 `MADV_DONTDUMP` / `WIPEONFORK`**（上游是 `cfg(target_os = "linux")`），
//!    所以"不进 core dump"这一条只在 Linux 上成立；
//! 3. **拿到本进程中任意代码执行权的人仍能读到它**：`PROT_NONE` 挡的是"越界读 / 误格式化 /
//!    顺手 dump"，不是"对手已经能在你的进程里跑代码"；
//! 4. **`Passphrase::new` 之前的副本不归我们管**（IPC 反序列化的缓冲、前端那侧的字符串）。
//!    我们能保证的是"进来之后只有这一个形态"，不是"从按键那一刻起就只有一份"。
//!
//! 还有一条与安全无关但要认的：**`mlock` 失败会让构造直接失败**（上游没有降级路径）——
//! 也就是说 `Passphrase::new` 返回 `Err` 时，解锁是**不能进行**的，而不是"悄悄不锁"。
//! 这个取舍属于 D5，已记入 ADR-0002 §10。
//!
//! 把字节交出去的唯一入口是 [`Passphrase::expose`]，它是 `pub(crate)`：口令能流向哪里，
//! 在本 crate 里数得出来（`open` / `create` 各一处），reviewer 一眼看得见。

use std::ops::Deref;

use memsafe::{MemSafeRead, Secret};

use crate::StoreError;

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
    /// 受保护页。`Secret<[u8; N]>` 而非 `Secret<Vec<u8>>` 是有意的：
    /// 后者只保护 `Vec` 的 24 字节头，真正的字节还在普通堆上（上游文档把这条叫
    /// "the `MemSafe<Vec<u8>>` pitfall"）。
    secret: Secret<MAX_LEN>,
    /// 实际长度。**页里剩下的字节是 0**，所以长度必须单独记 —— 一条长度不是机密，
    /// 而它挡住的正是"为了知道几字节而把口令读一遍"。
    len: usize,
}

impl Passphrase {
    /// 从字节造一个口令。**空字节是错误**，不是"没设口令"。
    ///
    /// 这一条为什么必须在类型上而不是在打开函数里：实测（ADR-0002 §7）空 key 送进
    /// `sqlite3_key()` 会让连接**退化成明文库**，而此后每一步都"成功" ——
    /// 一个能被误传的空值，迟早会被误传。造不出来就不会传错。
    ///
    /// 成功时**源缓冲已被擦零**（`memsafe` 在拷贝之后做的，见模块文档）。
    pub fn new(bytes: Vec<u8>) -> Result<Self, StoreError> {
        if bytes.is_empty() {
            return Err(StoreError::EmptyPassphrase);
        }
        if bytes.len() > MAX_LEN {
            return Err(StoreError::PassphraseTooLong { max: MAX_LEN });
        }
        let len = bytes.len();

        // 失败时上游把源缓冲还回来（长度不符那一路）或已擦零（内存保护失败那一路）；
        // 两种都不该出现在这里 —— 上面两条已经排除了长度问题，所以这条错误只可能是
        // "要不到一块能锁住的页"。
        let secret =
            Secret::<MAX_LEN>::from_bytes(bytes).map_err(|(_bytes, err)| StoreError::from(err))?;
        Ok(Self { secret, len })
    }

    /// 临时取得口令字节。返回的守卫 drop 时向 OS 交还权限（Unix 上是回到 `PROT_NONE`）。
    ///
    /// `pub(crate)` 是刻意的：这是口令流出本模块的**唯一**通道，而且只对本 crate 开放。
    pub(crate) fn expose(&mut self) -> Result<Exposed<'_>, StoreError> {
        let len = self.len;
        let guard = self.secret.read()?;
        Ok(Exposed { guard, len })
    }
}

/// [`Passphrase::expose`] 的返回值：一个**提权窗口**。
///
/// 它 derefs 成 `&[u8]`（口令本体）；它一 drop，权限就交还 OS。刻意不实现 `Debug` /
/// `Display` / `AsRef<[u8]>`：前者让它打不出来，后者会诱使调用方把 `&[u8]` 存起来 ——
/// 那就等于把提权窗口延长到守卫的生命期之外。
pub(crate) struct Exposed<'a> {
    guard: MemSafeRead<'a, [u8; MAX_LEN]>,
    len: usize,
}

impl Deref for Exposed<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.guard[..self.len]
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
