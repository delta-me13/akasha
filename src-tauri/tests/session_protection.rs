//! session key 那一页**真的被锁住了吗**（ADR-0002 D13 的判据表，按 plan 0902 的用途重验一次）。
//!
//! `AGENTS.md` §3.4 对"新用途"有硬要求：`memsafe` 的防护有明确边界，所以**新增一个用途就要
//! 按 D13 那张判据表重验一遍**，不得只说"已使用 memsafe"。口令（plan 0406）与私钥（plan 0403）
//! 各自验过；session key 是第三个用途，它由 ADR-0007 D7 引进来 —— 这个文件就是它那一次的读数。
//!
//! 三条在本文件里可断言的性质（都是上游 `memsafe` 的承诺，而承诺要由我们核）：
//!
//! | 性质 | 怎么看得见 |
//! |---|---|
//! | 那一页**已 mlock**（静止时在 `VmLck` 里） | 建之前 / 建之后 / 丢掉之后各读一次 `/proc/self/status` |
//! | **静止不可读**（`PROT_NONE`） | `smaps` 里一个 `---p` 段 |
//! | **不进 core dump** | 同一段的 `VmFlags` 同时含 `dd` 与 `wf` |
//!
//! ⚠️ **"那一页是我们的"不靠猜**：建 session 前后各取一次快照，**多出来的那一个**才是它 ——
//! 直接按总量断言会被同进程里别的分配（甚至别的测试线程的栈守卫页）搅乱，问题 #129 就是这么
//! 来的。这里只比"多出来的那一段"的**性质**，不比总量。
//!
//! ⚠️ 只在 Linux 上跑：`VmLck` 与 `smaps` 是 Linux 的接口（Windows 上 `memsafe` 的静止只读
//! 那一档本来就不成立，见 D13 的表）。

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::fs;

use akasha_lib::bw::Session;

/// 这份 session key 的长度**不是**重点，取文档里那个例子的形状。
const TOKEN: &str = "5PBYGU+5yt3RHcCjoeJKx/wByU34vokGRZjXpSH7Ylo8w==";

/// 进程级 `VmLck`（kB）。
fn locked_kb() -> u64 {
    let text = fs::read_to_string("/proc/self/status").unwrap();
    text.lines()
        .find_map(|line| line.strip_prefix("VmLck:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

/// 一页「静止不可读」的匿名内存：`(起始, 结束, VmFlags)`。只取 `---p` 且 ≤ 8 KiB 的段 ——
/// 我们那页是 4 KiB，而线程栈的 guard page 是几十 MB，一次就分开了（问题 #129 的处置）。
///
/// `/proc/self/smaps` 的段头形如 `7f1f…-7f1f… ---p 00000000 00:00 0`，属性行**缩进**且是
/// `键: 值` —— 解析时别忘 `trim_start()`（与 `passphrase_contract.rs` 同一份实现，
/// 那边栽过一次，这边不再犯）。
fn noaccess_pages() -> Vec<(u64, u64, String)> {
    let text = fs::read_to_string("/proc/self/smaps").unwrap();
    let mut out: Vec<(u64, u64, String)> = Vec::new();
    let mut current: Option<usize> = None;

    for line in text.lines() {
        let is_header = !line.starts_with(char::is_whitespace)
            && line
                .split_whitespace()
                .next()
                .is_some_and(|range| range.contains('-'));
        if is_header {
            let mut fields = line.split_whitespace();
            let range = fields.next().unwrap_or_default();
            let perms = fields.next().unwrap_or_default();
            let mut ends = range.split('-');
            let start = u64::from_str_radix(ends.next().unwrap_or("0"), 16).unwrap_or(0);
            let end = u64::from_str_radix(ends.next().unwrap_or("0"), 16).unwrap_or(0);
            current = if perms == "---p" && end - start <= 8192 {
                out.push((start, end, String::new()));
                Some(out.len() - 1)
            } else {
                None
            };
        } else if let Some(i) = current
            && let Some(rest) = line.trim_start().strip_prefix("VmFlags:")
        {
            out[i].2 = rest.trim().to_string();
        }
    }
    out
}

#[test]
fn the_session_key_page_is_locked_and_excluded_from_core_dumps() {
    let locked_before = locked_kb();
    let regions_before = noaccess_pages();

    let mut session = Session::from_output(TOKEN.as_bytes().to_vec()).expect("建一个 session");
    let locked_after = locked_kb();
    let regions_after = noaccess_pages();

    println!(
        "VmLck：{locked_before} → {locked_after} kB；不可读段 {} → {}",
        regions_before.len(),
        regions_after.len()
    );

    assert!(
        locked_after >= locked_before + 4,
        "session key 那一页没有被 mlock：{locked_before} → {locked_after} kB"
    );

    // 多出来的那一段就是它：静止不可读，且带 `dd`（不进 core dump）与 `wf`（不落 swap）。
    let fresh: Vec<_> = regions_after
        .iter()
        .filter(|region| !regions_before.contains(region))
        .collect();
    assert_eq!(
        fresh.len(),
        1,
        "建一个 session 应当只多出一个不可读段，实际多出 {} 个：{fresh:?}",
        fresh.len()
    );
    let (_start, _end, flags) = fresh[0];
    assert!(flags.contains("dd"), "那一页不该进 core dump：{flags}");
    assert!(flags.contains("wf"), "那一页不该被换出去：{flags}");

    // 取值仍逐字节相同（保护不等于读不出来 —— 读它要走一次提权窗口）。
    assert_eq!(&*session.expose().unwrap(), TOKEN.as_bytes());

    // 丢掉之后**回到起点**：这一条同时证明"它不是普通堆上的东西"。
    drop(session);
    let locked_after_drop = locked_kb();
    println!("丢掉之后 VmLck = {locked_after_drop} kB（起点 {locked_before}）");
    assert_eq!(
        locked_after_drop, locked_before,
        "丢掉 session 之后那一页必须还回 OS"
    );
}
