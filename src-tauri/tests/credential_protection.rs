//! ADR-0002 D13 对**每一个新用途**的要求：那张判据表要在新的生命周期下**重验一遍**
//! （plan 0406 是第一次、plan 0407 是第三次 —— 这是第四次：SSH 的凭据缓存）。
//!
//! 为什么不能只说"用了 memsafe"：`N` 不同、生命期不同，"这一页是我们的"这个识别方式
//! 与验证点都会变。这次的新东西是**生命期**：口令不再只活在解锁路径的一小段里，
//! 而是**缓存起来、活到库锁定或进程退出**。
//!
//! 于是这里验两条（另两条是同一个原语的属性，由 `akasha-store` 的用例钉着 ——
//! 这里用的是**同一个** `Protected<256>`，没有第二份实现）：
//!
//! 1. **`VmLck` 涨**：每一条缓存真的把一页锁进内存；
//! 2. **静止态那一页没有任何权限位**（`---p` 映射数跟着涨），并且
//!    **清空之后回落到起点** —— 这一条是"释放时真的还回去了"。

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use akasha_lib::ssh::{CacheKey, Credential, CredentialCache, CredentialKind, SshTarget};

/// 这个进程现在锁了多少 kB（`/proc/self/status` 的 `VmLck`）。
fn locked_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|line| {
            let rest = line.strip_prefix("VmLck:")?;
            rest.split_whitespace().next()?.parse().ok()
        })
        .expect("VmLck 不在 /proc/self/status 里")
}

/// 现在有多少个"没有任何权限位"的映射（受保护页的静止态就长这样）。
fn prot_none_pages() -> usize {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    maps.lines().filter(|line| line.contains(" ---p ")).count()
}

fn key(index: usize) -> CacheKey {
    CacheKey::new(
        SshTarget::new("example.invalid", 22, format!("user{index}")),
        CredentialKind::LoginPassword,
    )
}

/// 一条缓存的凭据：**锁住 → 静止态无权限 → 清空后还回去**。
#[test]
fn a_cached_credential_is_locked_while_it_lives_and_released_after() {
    const COUNT: usize = 8;

    let baseline_locked = locked_kb();
    let baseline_pages = prot_none_pages();
    {
        let cache = CredentialCache::new();
        for index in 0..COUNT {
            cache.insert(key(index), Credential::new(vec![b'x'; 32]).unwrap());
        }
        assert_eq!(cache.len(), COUNT);

        let during_locked = locked_kb();
        let during_pages = prot_none_pages();
        // 正对照：**必须比基线高**，否则"没有泄漏"与"这一页根本没被锁"是同一条绿
        // （`docs/STATUS.md` 问题 #87）。一页 = 4 kB。
        assert!(
            during_locked >= baseline_locked + 4 * COUNT as u64,
            "{COUNT} 条凭据应当至少多锁 {} kB（{baseline_locked} → {during_locked}）",
            4 * COUNT
        );
        assert!(
            during_pages >= baseline_pages + COUNT,
            "每一条凭据都该有一个静止态无权限的页（{baseline_pages} → {during_pages}）"
        );

        cache.clear();
    }

    // 缓存连同它里面的 `Arc` 一起 drop 之后，两样都要**回到起点**。
    assert_eq!(locked_kb(), baseline_locked, "清空并释放之后不该还锁着内存");
    assert_eq!(
        prot_none_pages(),
        baseline_pages,
        "清空并释放之后不该还留着受保护的页"
    );
}

/// 空口令造不出来 —— 与 `Passphrase` 同一条理由（能被误传的空值迟早会被误传）。
#[test]
fn an_empty_credential_is_not_representable() {
    assert!(Credential::new(Vec::new()).is_err());
}
