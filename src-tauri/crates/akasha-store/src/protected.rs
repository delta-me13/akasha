//! **受保护的一页内存** —— 口令与私钥共用的那一份实现（ADR-0002 D13）。
//!
//! 0406 之后口令是这么放的，0403 起私钥也是。两处抄两份的代价不是"多写 40 行"，
//! 而是**两份会各自漂移**：一处加了"读回失败要当错误"，另一处没有 —— 于是泄漏点
//! 就出现在没人再看的那一份里。所以提成泛型：`Protected<N>` 是那个 N 字节的受保护页，
//! 口令是 `N = 256`，私钥是 `N = 16384`。
//!
//! 上游 `memsafe` 做的那些系统调用见 `memsafe` 的文档与 ADR-0002 §7.2（实测）；
//! 这里只管两件事：**长度上限**与**把字节交出去的那一次提权**。
//!
//! ## 为什么这个模块是 `pub`（plan 0502）
//!
//! 第三个用途来了：SSH 的**内存凭据缓存**（ADR-0003 D8）也要把口令放进受保护页。
//! 它是**另一种**机密 —— 口令在这里要能被 SSH 栈读出来送进握手，
//! 与 [`crate::Passphrase`] 那条"只对本 crate 开一个口"的通道**要求不同**。
//! 于是选择是：① 在本模块再抄一份（两份必然漂移，理由是上面那段）；
//! ② 把**原语**公开，让每个用途各自决定自己的暴露面 —— 取 ②。
//! ⚠️ 公开的是"受保护的一页"这件事，不是"谁都能读口令"：[`crate::Passphrase`] 的
//! `expose` 仍然是 `pub(crate)`，一个字节都没多给。
//!
//! **只往这里放机密。** 它每次构造都要一整页 `mlock` + 两个 `madvise`，
//! 用来装普通数据是纯粹的浪费，也会让"`VmLck` 涨了就是有机密"这条判据失去意义。
//!
//! ## 为什么 `Secret<[u8; N]>` 而不是 `Secret<Vec<u8>>`
//!
//! 后者只保护 `Vec` 的 24 字节头，真正的字节还在普通堆上（上游文档把这条叫
//! "the `MemSafe<Vec<u8>>` pitfall"）。用 `[u8; N]` 意味着**字节按类型就落在那一页里**，
//! 不存在"忘了搬进去"这种写法。
//!
//! ## 超长为什么是错误而不是截断
//!
//! 截断会让"口令错了"/"密钥对不上"变成一件没人能解释的事。代价是：**调用方必须在
//! 秘密进入这条路径之前就能算出它有多长**（私钥那边因此把上限也放在了写入侧，
//! 见 [`crate::pools::keys`]）—— 而那条检查比这里的更早、更有意义，
//! 因为超长的私钥**根本不该进库**。

use std::ops::Deref;
use std::sync::atomic::{AtomicU8, Ordering};

use memsafe::{MemSafeRead, Secret};

/// 构造 / 读取受保护页可能出的两种错。
///
/// 刻意**不是** [`crate::StoreError`]：这一层不知道自己在护着的是口令还是私钥，
/// 而"太长"那句话对这两者的说法不同（`PassphraseTooLong` / `SecretTooLong`）。
/// 各自的模块把它翻成自己的错误 —— 翻译只有两处，且都是穷尽 `match`。
#[derive(Debug)]
pub enum PageError {
    /// 比这一页还长。**不截断**。
    TooLong { max: usize },
    /// 要不到一块能锁住的页（`mmap` / `mlock` 被拒）。上游没有降级路径：
    /// 这条错误意味着"不能进行"，而不是"悄悄不锁"。
    Memory(memsafe::error::MemoryError),
}

impl std::fmt::Display for PageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong { max } => write!(f, "超过一页（上限 {max} 字节）"),
            Self::Memory(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PageError {}

/// 一整页受保护内存，装着"最多 `N` 字节"的机密与它的实际长度。
pub struct Protected<const N: usize> {
    secret: Secret<N>,
    /// 实际长度。**页里剩下的字节是 0**，所以长度必须单独记 —— 一条长度不是机密，
    /// 而它挡住的正是"为了知道几字节而把秘密读一遍"。
    len: usize,
}

impl<const N: usize> Protected<N> {
    /// 把 `bytes` 搬进受保护页。成功时**源缓冲已被擦零**（上游做的）。
    pub fn new(bytes: Vec<u8>) -> Result<Self, PageError> {
        let mut bytes = bytes;
        if bytes.len() > N {
            // ⚠️ 这条路上 `bytes` 还是明文，而它马上要被 drop（= 释放一块存着秘密的
            // 普通堆内存）。所以自己先擦一遍。上游只在**成功**路径上擦源缓冲。
            wipe(&mut bytes);
            return Err(PageError::TooLong { max: N });
        }
        let len = bytes.len();

        // 失败时上游把源缓冲还回来：长度问题上面已经排除，所以这条只可能是
        // "要不到一块能锁住的页"。
        let secret =
            Secret::<N>::from_bytes(bytes).map_err(|(_bytes, err)| PageError::Memory(err))?;
        Ok(Self { secret, len })
    }

    /// 实际长度（不是页大小）。
    ///
    /// 名字带 `byte_` 而不是 `len()`：`len()` 在 Rust 里属于"容器"，于是 clippy 会要求
    /// 配一个 `is_empty` —— 而**空机密在这里造不出来**（空口令 / 空私钥都在上游被拒），
    /// 那个方法只会永远返回 `false`。与 [`crate::pools::keys::PrivateKey::byte_len`] 同一口径。
    pub fn byte_len(&self) -> usize {
        self.len
    }

    /// 临时取得字节。返回的守卫 drop 时向 OS 交还权限（Unix 上回到 `PROT_NONE`）。
    pub fn expose(&mut self) -> Result<Exposed<'_, N>, PageError> {
        let len = self.len;
        let guard = self.secret.read().map_err(PageError::Memory)?;
        Ok(Exposed { guard, len })
    }
}

/// [`Protected::expose`] 的返回值：一个**提权窗口**。
///
/// 它 derefs 成 `&[u8]`（秘密本体）；一 drop 权限就交还 OS。刻意不实现 `Debug` /
/// `Display` / `AsRef<[u8]>`：前者让它打不出来，后者会诱使调用方把 `&[u8]` 存起来 ——
/// 那就等于把提权窗口延长到守卫的生命期之外。
pub struct Exposed<'a, const N: usize> {
    guard: MemSafeRead<'a, [u8; N]>,
    len: usize,
}

impl<const N: usize> Deref for Exposed<'_, N> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.guard[..self.len]
    }
}

/// 擦零一个普通堆缓冲。
///
/// 用**原子存储**而不是 `bytes.fill(0)`：后者可以被编译器整个优化掉（那块内存马上
/// 就没人读了），于是"擦过"只发生在源码里。原子写是**可观察**的，编译器删不掉。
///
/// 这样就不必为了一次擦零引入一处 `unsafe`（`AGENTS.md` §3.4：`unsafe` 只许出现在
/// 本 crate 的那一个单点，放宽它等于改架构）。
fn wipe(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        AtomicU8::from_mut(byte).store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    /// 只在这一个用例里用的小页：`N` 是编译期常量，所以每个用例各自选一个 N。
    const SMALL: usize = 8;

    #[test]
    fn bytes_come_back_byte_for_byte() {
        for bytes in [vec![0x00], vec![0xff; SMALL], b"abcd".to_vec()] {
            let expected = bytes.clone();
            let mut page = Protected::<SMALL>::new(bytes).unwrap();
            assert_eq!(page.byte_len(), expected.len());
            assert_eq!(&*page.expose().unwrap(), &expected[..]);
        }
    }

    #[test]
    fn the_page_refuses_more_than_it_can_hold() {
        match Protected::<SMALL>::new(vec![b'z'; SMALL + 1]) {
            Err(PageError::TooLong { max }) => assert_eq!(max, SMALL),
            _ => panic!("超过一页的机密不该造得出来"),
        }
    }

    #[test]
    fn wiping_is_observable() {
        // 不能断言"编译器没优化掉"，但能断言这条函数的**语义**：进来什么出去就全零。
        // 真正的保证来自"原子存储不可被删"这条语言事实，写在 `wipe` 的注释里。
        let mut bytes = b"a secret here".to_vec();
        let len = bytes.len();
        wipe(&mut bytes);
        assert_eq!(bytes, vec![0u8; len], "擦零之后不该留下任何原文");
    }
}
