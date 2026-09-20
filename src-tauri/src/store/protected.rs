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
//! 与 [`crate::store::Passphrase`] 那条"只对本 crate 开一个口"的通道**要求不同**。
//! 于是选择是：① 在本模块再抄一份（两份必然漂移，理由是上面那段）；
//! ② 把**原语**公开，让每个用途各自决定自己的暴露面 —— 取 ②。
//! ⚠️ 公开的是"受保护的一页"这件事，不是"谁都能读口令"：[`crate::store::Passphrase`] 的
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
//! 见 [`crate::store::pools::keys`]）—— 而那条检查比这里的更早、更有意义，
//! 因为超长的私钥**根本不该进库**。

use std::ops::Deref;
use std::sync::atomic::{AtomicU8, Ordering};

use memsafe::{MemSafeRead, Secret};

/// 构造 / 读取受保护页可能出的两种错。
///
/// 刻意**不是** [`crate::store::StoreError`]：这一层不知道自己在护着的是口令还是私钥，
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

// ── Windows：进程能锁住多少页的额度 ────────────────────────────────────────

/// Windows 上抬高之后的**最小工作集**——它同时是一个进程能锁住的页数的上限。
///
/// 取值理由：SQLCipher 的 `cipher_memory_security` 会给它自己的每一次分配调 `VirtualLock`，
/// 而它的页缓存默认就是 2 MB（每个连接），再加上导出 / 还原那条 `ATTACH` 路径的临时分配，
/// 同一时刻可能有数个 MB 处于锁定状态。16 MiB 给足余量；而它对常驻内存的下限没有实际影响，
/// 本进程的常态驻留远高于它。
#[cfg(windows)]
const MIN_WORKING_SET: usize = 16 * 1024 * 1024;

/// 抬高之后的**最大工作集**。能锁住多少页只由最小值决定，这一项是为了满足 API 的两条形状
/// 约束：它必须 ≥ 最小值，且必须小于“可用页数 − 512 页”。取 256 MiB —— 向下远高于本应用的
/// 常态驻留（否则内存紧张时内存管理器会开始裁剪它），向上远低于任何能跑 WebView2 的机器。
#[cfg(windows)]
const MAX_WORKING_SET: usize = 256 * 1024 * 1024;

// `kernel32` 的两个入口。手写这两行而不是引入 `windows-sys`：只用到两个函数，
// 而新增一个依赖要走 `just deny` 的许可证与来源门禁，代价不成比例。
#[cfg(windows)]
#[allow(unsafe_code)] // 本模块在 store/ 内（no-unsafe-outside-store.yml）；理由见下
#[allow(non_snake_case)] // 这两个名字是 Win32 的导出名，改名就链接不上了
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn SetProcessWorkingSetSize(process: *mut std::ffi::c_void, min: usize, max: usize) -> i32;
}

/// 把本进程的最小 / 最大工作集抬到上面两个常量。**只调一次**，失败只记一条 `warn`。
///
/// 为什么必须做（问题 #167）：`VirtualLock` 的文档写明“一个进程能锁住的页数 = 它的最小工作集
/// 减去一点开销”，而默认只有 50 页（200 KiB）。SQLCipher 的 `cipher_memory_security = ON`
/// 与这里的受保护页**共用同一份额度**，于是“库解锁着 + 读一把私钥”会拿到
/// `ERROR_WORKING_SET_QUOTA`；同一份文档给出的处置就是这一条。
///
/// 为什么放在这个模块：它是唯一构造受保护页的地方，而它一定早于任何连接被打开
/// （`create` / `open` / `to_encrypted` 都要先有一个 [`crate::store::Passphrase`]），
/// 所以抬额度必然发生在 SQLCipher 那一边开始上锁之前。
///
/// 为什么失败不阻断：这是进程级的调优，不是本次操作的前置条件。真的锁不住页时
/// [`Protected::new`] 会把它报成 [`PageError::Memory`] —— 判据在那里，不在这里。
#[cfg(windows)]
#[allow(unsafe_code)] // 同上
fn ensure_working_set() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: ① `GetCurrentProcess()` 返回一个**伪句柄**：它是常量、在任何线程上都有效、
        // 不需要关闭，且天然带 `PROCESS_SET_QUOTA`（那是对自己的句柄）；② 两个尺寸参数都落在
        // 文档允许的范围里（最小值 > 0 且 ≤ 最大值；最大值 ≥ 13 页且远小于“可用页数 − 512 页”）；
        // ③ 这个调用只改本进程的工作集上下限，不碰任何指针或内存内容，失败也只是返回 0。
        let raised = unsafe {
            SetProcessWorkingSetSize(GetCurrentProcess(), MIN_WORKING_SET, MAX_WORKING_SET)
        };
        if raised == 0 {
            // 拿不到错误码就不写这个字段（`AGENTS.md` §3.4：字段值不得虚构）。
            match std::io::Error::last_os_error().raw_os_error() {
                Some(code) => tracing::warn!(code, "process working set not raised"),
                None => tracing::warn!("process working set not raised"),
            }
        }
    });
}

/// 其他平台：能锁多少页由 `RLIMIT_MEMLOCK` 一类决定，没有对应的进程级旋钮。
#[cfg(not(windows))]
fn ensure_working_set() {}

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
        ensure_working_set();
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
    /// 那个方法只会永远返回 `false`。与 [`crate::store::pools::keys::PrivateKey::byte_len`] 同一口径。
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

    /// Windows 上“一个进程能锁住多少页”= **它的最小工作集**减去一点开销（`VirtualLock` 的
    /// 文档），而默认只有 50 页（200 KiB）。SQLCipher 的 `cipher_memory_security = ON`
    /// 会给它自己的每一次分配调 `VirtualLock`，与这里的受保护页**共用同一份额度** ——
    /// 于是“库解锁着 + 读一把私钥”这条路会拿到 `ERROR_WORKING_SET_QUOTA`（问题 #167）。
    ///
    /// 这条用例钉住的正是那个额度够不够：连续建 32 个 16 KiB 的页（= 512 KiB）必须全部成功。
    /// 没有抬额度时这里只能建起 11 个（176 KiB / 16 KiB），第 12 个就失败。
    ///
    /// ⚠️ 只在 Windows 上执行：其他平台上“能锁多少”由 `RLIMIT_MEMLOCK` 决定，与这条判据无关。
    #[cfg(windows)]
    #[test]
    fn windows_holds_many_protected_pages_at_once() {
        let pages: Vec<Protected<16384>> = (0..32)
            .map(|_| Protected::<16384>::new(vec![0x5a; 32]).unwrap())
            .collect();
        assert_eq!(pages.len(), 32, "32 个受保护页（512 KiB）应当同时建得起来");
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
