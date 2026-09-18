//! **ADR-0002 D13 的判据表 —— 私钥那一份**（plan 0403 是它第一次被逐条重验）。
//!
//! D13 的规则是"内存里长住的机密统一经 `memsafe`，但**每新增一个用途要按那张判据表重验一遍**" ——
//! 因为"用了 memsafe"这句话在 `N` 变了、生命期变了之后就不等价于"口令那套判据仍然成立"。
//! 这里和口令那一份（`passphrase_contract.rs` §6）的差别是具体的：
//!
//! | | 口令 | 私钥 |
//! |---|---|---|
//! | `N` | 256（一页里的 256 字节） | 16384（4 页） |
//! | 生命期 | 解锁路径上一小段 | 从库里读出来到连接用完 |
//! | "用"的入口 | `create` / `open` | `keys::private_key` |
//!
//! 判据仍是那四条：**① `VmLck` 涨；② 那一页静止态没有任何权限；③ `VmFlags` 有 `dd`；
//! ④ `VmFlags` 有 `wf`**。另加一条这一层特有的：**读完还回去**（`munmap`）——
//! 私钥不该在进程里留驻，而"页一直挂着"是看不出错的。
//!
//! ⚠️ 这些判据**只在 Linux 上成立**（`dd` / `wf` 是 Linux 的 `madvise`，
//! `VmFlags` 是 `/proc` 的东西）。Windows（静止态只读）与 macOS（没有 `dd`/`wf`）
//! 的差异记在 D13 的不足表里，不是"覆盖不到所以不提"。
//!
//! **测不了的那条照实记**：`memsafe` 会把调用方那个 `Vec<u8>` 源缓冲擦零，但那一份在
//! `from_bytes` 里就被 `mem::forget` 了，外面拿不到、也就断言不了 ——
//! 它属于"依赖上游实现"，与口令那一侧同一句话（D13 的不足表）。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "store_common/mod.rs"]
mod common;

use akasha_lib::store::keys;

// 三条判据都只在 Linux 上成立（见文件头），所以 `new_vault` 与另两个一起门控 ——
// 不门控时 macOS 上会多一条"未使用的 import"，而 `just clippy` 带 `-D warnings`。
#[cfg(target_os = "linux")]
use common::{locked_kb, new_vault, noaccess_pages};

/// 私钥那一页的四条判据 + 一条"用完还回去"。
#[cfg(target_os = "linux")]
#[test]
fn the_private_key_page_is_locked_and_excluded_from_core_dumps() {
    let (_dir, conn) = new_vault("key-page-protection");

    // 先把数据填好。插入时那个 `PrivateKey` 用完即 drop（页还回去），
    // 所以下面取"之前"快照时进程里不该还挂着上一把钥匙的页 ——
    // 否则"多出来的那一页"就说不清是谁的。
    let pem = vec![b'k'; 4096];
    let id = {
        let mut key = keys::PrivateKey::new(pem).unwrap();
        keys::insert_key(
            &conn,
            &keys::NewKey {
                name: "protected".into(),
                public_key: String::new(),
                comment: None,
            },
            &mut key,
        )
        .unwrap()
    };

    let pages_before = noaccess_pages();
    let locked_before = locked_kb();

    // 真跑一遍**使用路径**（从库里读出来），不是只构造一个 `PrivateKey`：
    // 这样"下面那一页属于这把钥匙"不是推测。
    let mut key = keys::private_key(&conn, id).unwrap();

    let pages_after = noaccess_pages();
    let locked_after = locked_kb();

    let ours: Vec<_> = pages_after
        .iter()
        .filter(|page| !pages_before.contains(page))
        .collect();
    for (start, end, flags) in &ours {
        println!(
            "私钥的受保护页：{start:#x}-{end:#x}（{} 字节）VmFlags=[{flags}]",
            end - start
        );
    }
    println!("VmLck：{locked_before} kB → {locked_after} kB");
    assert_eq!(
        ours.len(),
        1,
        "读私钥前后应当只多出一页「静止不可读」的匿名内存，实际 {}",
        ours.len()
    );

    // 内容先对一遍：判据是"护住了**这把钥匙**"，不是"护住了一块匿名内存"。
    assert_eq!(key.byte_len(), 4096);
    assert_eq!(&*key.expose().unwrap(), vec![b'k'; 4096].as_slice());

    // ① 静止态不可读（`---p`），而且**大小就是 `N`** —— 口令那页是 4096，这一页是 16384。
    //    这条正是 D13 要求"重验"的原因：`N` 不同，"那一页"就不是同一个东西。
    assert_eq!(
        ours[0].1 - ours[0].0,
        keys::MAX_PEM_LEN as u64,
        "受保护页应当正好是 N 字节（16 KiB = 4 页）"
    );

    // ② mlock：进程级 VmLck 必须涨够这一页（16 KiB）
    assert!(
        locked_after >= locked_before + (keys::MAX_PEM_LEN / 1024) as u64,
        "私钥那一页没有 mlock —— 它可能被换进 swap：{locked_before} → {locked_after} kB"
    );

    // ③ 不进 core dump（`dd`）④ fork 时清零（`wf`）
    let flags = &ours[0].2;
    assert!(
        flags.split_whitespace().any(|f| f == "dd"),
        "私钥那一页没有 MADV_DONTDUMP —— 它会被写进 core dump：{flags}"
    );
    assert!(
        flags.split_whitespace().any(|f| f == "wf"),
        "私钥那一页没有 MADV_WIPEONFORK —— fork 出来的子进程会继承它：{flags}"
    );

    // ⑤ 用完还回去：私钥不是口令，它**按需读**、用完就该消失。
    //    "页一直挂着"这种错在其它四条判据下是绿的 —— 只有这一条看得见。
    drop(key);
    assert_eq!(
        noaccess_pages().len(),
        pages_before.len(),
        "读完的私钥页应当被 munmap 还回 OS —— 它不该在进程里留驻"
    );
}

/// `N` 的**取值依据**的可执行版本：一次读只锁 `N` 字节，而 `N` 是常量、与 PEM 多长无关。
///
/// 它不是安全判据，是"这个数为什么是 16 KiB"的两个边界 —— 放在 `const` 块里是刻意的：
/// 改坏了**编译不过**，而不是"某天有人跑测试才发现"。
#[test]
fn the_page_is_sized_for_real_keys() {
    // 常见形态里的极大值：RSA 4096 的 PEM ≈ 3.3 KB（ed25519 只有几百字节）。
    // 留几倍余量是刻意的，但"余量"要是有限的 —— 无限大就等于没有上限。
    const RSA_4096_PEM: usize = 3400;
    const {
        assert!(
            keys::MAX_PEM_LEN >= RSA_4096_PEM * 4,
            "MAX_PEM_LEN 对 RSA 4096（≈3.3 KB）的余量不足"
        )
    };
    // 上界同样有判据：N 是**每把钥匙**的 mlock 量，与 PEM 实际长度无关。
    const { assert!(keys::MAX_PEM_LEN <= 65536, "MAX_PEM_LEN 太大了") };

    println!(
        "MAX_PEM_LEN = {} 字节（RSA 4096 的 PEM ≈ {RSA_4096_PEM} 字节）",
        keys::MAX_PEM_LEN
    );
}
