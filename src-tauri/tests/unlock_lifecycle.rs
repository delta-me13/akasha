//! **解锁 / 锁定的生命周期里，进程内存里到底还剩什么**（plan 0407）。
//!
//! 判据来自 ROADMAP：「解锁 → 读一次池 → **锁定之后进程里不留机密**（`VmLck` 回落到
//! 解锁前的水平）」。`VmLck` 只是其中**看得见**的那一半 —— 这一层真正要回答的是
//! "口令与派生密钥还在不在内存里"，所以除了 `VmLck` 与 `---p` 段数，这里还把
//! **整个进程内存扫一遍**找那两串字节。
//!
//! ## 扫描器的三条纪律（第一版栽过两次，都记在这里）
//!
//! 1. **必须也扫 `---p` 的段**：口令那一页静止态是 `PROT_NONE`，只看 `r` 开头的段会把
//!    **唯一真正该被看见的那一页**漏掉（`/proc/self/mem` 的读走 `FOLL_FORCE`，
//!    `---p` 照样读得出来 —— 同 `passphrase_contract.rs` 那条边界测试）；
//! 2. **读缓冲复用，并且用完就擦**：缓冲本身就在进程内存里，装着刚读过的那些字节 ——
//!    不擦的话第二次扫描会把"上一次的拷贝"当成新命中（第一版就是这么越扫越多的）；
//! 3. **针是真随机的**（`/dev/urandom`）：运行期算出来的固定序列会跟内存里别的东西撞上，
//!    "基线"里就冒出十几处命中，那就分不出"我们的缓冲区"与"别处"了。
//!
//! ## "没有留下"这类断言必须有**正对照**
//!
//! 这是问题 #87 的第二次适用：`during > before` 那一条是这套用例的**正对照** ——
//! 少了它，"锁上之后 0 命中"与"扫描器根本没在工作"是同一条绿。
//!
//! ⚠️ 扫描的段有**上限**（`CAP`，8 MiB）：超大段（线程栈、映射进来的大文件）跳过。
//! 这不会让判据落空 —— 正对照要求"解锁期间扫得到"，扫不到就是红。
//!
//! ⚠️ 判据只在 Linux 上成立（`/proc` 的 `smaps` / `status` / `mem`）。
//! Windows 与 macOS 的差异记在 ADR-0002 D13 的不足表里，不是"覆盖不到所以不提"。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "store_common/mod.rs"]
mod common;

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::io::Read;
    use std::os::unix::fs::FileExt;
    use std::process::Command;

    use akasha_lib::store::{Passphrase, create, open, vault_path};

    use akasha_lib::common;

    /// 每次从段里读多大一块。**块之间留 `needle.len() - 1` 字节的重叠**，
    /// 所以跨块边界的命中不会被漏掉，也不会被数两遍（完整命中装不进那点重叠里）。
    pub(super) const CHUNK: usize = 1 << 20;

    /// 一根**真随机**的针：它在这台机器的内存里只可能出现在我们自己放的地方。
    pub(super) fn needle() -> Vec<u8> {
        let mut raw = vec![0u8; 24];
        fs::File::open("/dev/urandom")
            .unwrap()
            .read_exact(&mut raw)
            .unwrap();
        raw
    }

    /// 擦零一块缓冲。原子写，编译器删不掉 —— 与 `protected::wipe` 同一个理由。
    fn wipe(buf: &mut [u8]) {
        use std::sync::atomic::{AtomicU8, Ordering};
        for byte in buf.iter_mut() {
            AtomicU8::from_mut(byte).store(0, Ordering::Relaxed);
        }
    }

    /// 扫一遍**匿名**段（`buf` 是复用的一块，见 `CHUNK`）。
    ///
    /// 只扫匿名段：机密只可能落在我们自己分配的内存里（堆、受保护页、SQLCipher 自己的
    /// 分配区），而映射进来的可执行文件与库动辄几百 MB、只读，扫它们既慢又不可能有东西。
    ///
    /// ⚠️ **不给"大段"设上限**：第一版按 8 MiB 截断，结果 SQLCipher 的 codec 副本
    /// 正好落在一个更大的段里 —— 扫描器"什么都没扫到"，而用例照样绿。
    /// 漏扫的代价由正对照（解锁期间必须多扫到）兜住；这条解释留着，别再往回加截断。
    fn scan_with(needle: &[u8], buf: &mut [u8]) -> Vec<(u64, String, u64)> {
        let maps = fs::read_to_string("/proc/self/maps").unwrap();
        let mem = fs::File::open("/proc/self/mem").unwrap();
        let mut hits = Vec::new();

        for line in maps.lines() {
            let mut fields = line.split_whitespace();
            let range = fields.next().unwrap_or_default();
            let perms = fields.next().unwrap_or_default().to_string();
            // 跳过**文件支撑**的段：`start-end perms offset dev inode path` 里第 6 段是路径。
            // `[heap]` / `[stack]` 这类内核起的名字要扫（方括号不是路径）。
            if let Some(path) = fields.nth(3)
                && !path.starts_with('[')
            {
                continue;
            }
            let mut ends = range.split('-');
            let start = u64::from_str_radix(ends.next().unwrap_or("0"), 16).unwrap_or(0);
            let end = u64::from_str_radix(ends.next().unwrap_or("0"), 16).unwrap_or(0);
            if end <= start || start == 0 {
                continue;
            }

            let overlap = (needle.len() - 1) as u64;
            let mut offset = start;
            while offset < end {
                let len = CHUNK.min((end - offset) as usize);
                let slice = &mut buf[..len];
                // 读不出来就跳过这一块：`---p` 的页在内核不给时是 EIO，
                // 而"扫不到"由正对照兜住。
                let Ok(read) = mem.read_at(slice, offset) else {
                    break;
                };
                if read == 0 {
                    break;
                }
                for (i, window) in slice[..read].windows(needle.len()).enumerate() {
                    if window == needle {
                        hits.push((offset + i as u64, perms.clone(), start));
                    }
                }
                wipe(slice);
                offset += (read as u64).saturating_sub(overlap).max(1);
            }
        }
        hits
    }

    /// 打印命中，格式统一（地址 + 段 + 段内偏移）。
    pub(super) fn show(label: &str, hits: &[(u64, String, u64)]) {
        println!("{label}：命中 {} 次", hits.len());
        for (addr, perms, region) in hits {
            println!(
                "   {addr:#x}（段 {region:#x} +{:#x} {perms}）",
                addr - region
            );
        }
    }

    /// SQLCipher 4 的默认派生参数算出来的密钥：PBKDF2-HMAC-SHA512、256000 轮、32 字节，
    /// 盐 = 库文件前 16 字节（ADR-0002 §7 的磁盘事实）。
    ///
    /// 用 `openssl` CLI 而不是自己实现 PBKDF2：这里要**独立于被测代码**地算出那把密钥 ——
    /// 用被测代码算，测的就成了"它等于它自己"。
    pub(super) fn derived_key(passphrase: &[u8], salt: &[u8]) -> Option<Vec<u8>> {
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let out = Command::new("openssl")
            .args([
                "kdf",
                "-keylen",
                "32",
                "-kdfopt",
                "digest:SHA512",
                "-kdfopt",
                &format!("hexpass:{}", hex(passphrase)),
                "-kdfopt",
                &format!("hexsalt:{}", hex(salt)),
                "-kdfopt",
                "iter:256000",
                "PBKDF2",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8(out.stdout).ok()?;
        let hex: String = text.trim().chars().filter(|c| *c != ':').collect();
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
            .collect()
    }

    /// 判据 ①：`VmLck` 走一个来回，`---p` 段数也回得来。
    ///
    /// 三步各自都有"涨"的断言，所以这条用例不会因为"什么都没发生"而变绿：
    /// 口令那一页 +4 KiB、SQLCipher 自己的分配 +几十 KiB、丢掉之后**一步不少地回去**。
    #[test]
    fn the_locked_memory_walks_back_to_where_it_started() {
        let dir = common::fixture_dir("unlock-lifecycle-locked-memory");
        let db = vault_path(&dir);

        let locked_before = common::locked_kb();
        let pages_before = common::noaccess_pages().len();

        let mut passphrase = Passphrase::new(vec![b'p'; 32]).unwrap();
        let locked_passphrase_only = common::locked_kb();

        let conn = create(&db, &mut passphrase).unwrap();
        let locked_with_connection = common::locked_kb();

        drop(conn);
        let locked_after_connection = common::locked_kb();

        drop(passphrase);
        let locked_after_everything = common::locked_kb();
        let pages_after = common::noaccess_pages().len();

        println!(
            "VmLck：起 {locked_before} → 只有口令 {locked_passphrase_only} → \
             解锁中 {locked_with_connection} → 丢掉连接 {locked_after_connection} → \
             丢掉口令 {locked_after_everything} kB"
        );

        assert!(
            locked_passphrase_only >= locked_before + 4,
            "口令那一页没有 mlock：{locked_before} → {locked_passphrase_only} kB"
        );
        assert!(
            locked_with_connection > locked_passphrase_only,
            "解锁之后 `VmLck` 没有涨 —— SQLCipher 的 `cipher_memory_security` 会给每次分配上锁：\
             {locked_passphrase_only} → {locked_with_connection} kB"
        );
        assert_eq!(
            locked_after_connection, locked_passphrase_only,
            "丢掉连接之后应当只剩下口令那一页"
        );
        assert_eq!(
            locked_after_everything, locked_before,
            "锁定之后 `VmLck` 必须回到起点（ROADMAP 的判据）"
        );
        assert_eq!(pages_after, pages_before, "受保护页应当被 munmap 还回 OS");
    }

    /// 判据 ②：锁定之后**口令在进程内存里一处不剩**，而解锁期间多出来的那一处
    /// 正好是**受保护页**。
    ///
    /// 基线里那 1 处命中是本用例自己那根针（`pass`）—— 所以断言写成"锁上之后回到基线"，
    /// 而不是"命中 0 次"：后者会把"我们自己那份"也当成泄漏。
    ///
    /// ⚠️ 这条用例量到的第二件事**推翻了一个想当然**：解锁期间口令**只多出一处**
    /// （就是那一页 `---p`）。`sqlite3_key` 拿到的就是我们那一页的指针，而 SQLCipher
    /// **不留口令本体** —— 它手里只有派生密钥（下面那条用例量到了）。所以"口令的副本清单"
    /// 里根本没有"SQLCipher 内部那一份"这一行。
    #[test]
    fn no_copy_of_the_passphrase_outlives_the_unlock_window() {
        let pass = needle();
        let dir = common::fixture_dir("unlock-lifecycle-passphrase-copies");
        let db = vault_path(&dir);
        let mut buf = vec![0u8; CHUNK];

        let before = scan_with(&pass, &mut buf);
        show("① 解锁前", &before);

        let mut passphrase = Passphrase::new(pass.clone()).unwrap();
        let conn = create(&db, &mut passphrase).unwrap();
        let during = scan_with(&pass, &mut buf);
        show("② 解锁中", &during);

        drop(conn);
        let after_connection = scan_with(&pass, &mut buf);
        show("③ 丢掉连接", &after_connection);

        drop(passphrase);
        let after_everything = scan_with(&pass, &mut buf);
        show("④ 锁定之后", &after_everything);

        // 正对照：解锁期间必须比基线**多**扫到 —— 否则"锁上之后回到基线"什么都没证明。
        let extra: Vec<_> = during
            .iter()
            .filter(|hit| !before.iter().any(|base| base.0 == hit.0))
            .collect();
        assert_eq!(
            extra.len(),
            1,
            "解锁期间应当只多出一处口令（那一页受保护内存），实际多出 {} 处：{extra:?}",
            extra.len()
        );
        assert_eq!(
            extra[0].1, "---p",
            "多出来的那一处不是受保护页（静止不可读）—— 那是别的东西留了一份口令"
        );
        // 丢连接**不该**让口令的副本数变化：SQLCipher 不留口令本体（见上面那段）。
        assert_eq!(
            after_connection.len(),
            during.len(),
            "丢掉连接之后口令的副本数变了 —— 说明连接期间还另有别的副本"
        );
        assert_eq!(
            after_everything.len(),
            before.len(),
            "锁定之后进程里还有口令的副本（基线 {} 处 → 现在 {} 处）",
            before.len(),
            after_everything.len()
        );
        assert!(
            after_everything.iter().all(|hit| hit.1 != "---p"),
            "锁定之后那一页还在：{:?}",
            after_everything
        );
    }

    /// 判据 ③：**派生密钥**也一处不剩（那才是 SQLCipher 真正用来解页的东西）。
    ///
    /// 这条比口令那条更值钱：密钥是 `sqlite3_key` 派生出来的，我们从来不持有它 ——
    /// 它在不在内存里，完全取决于 `cipher_memory_security` 有没有在释放时擦零。
    /// 基线那 1 处命中同样是本用例自己算出来的那份。
    ///
    /// `openssl` CLI 不在时**显式跳过并写明原因**（不静默通过）：这条判据的独立性
    /// 就建立在"用别的工具算出同一把密钥"上。
    #[test]
    fn no_copy_of_the_derived_key_outlives_the_connection() {
        let pass = needle();
        let dir = common::fixture_dir("unlock-lifecycle-derived-key");
        let db = vault_path(&dir);
        let mut buf = vec![0u8; CHUNK];

        let mut passphrase = Passphrase::new(pass.clone()).unwrap();
        let conn = create(&db, &mut passphrase).unwrap();
        let salt = fs::read(&db).unwrap()[..16].to_vec();
        let Some(key) = derived_key(&pass, &salt) else {
            eprintln!(
                "跳过：这台机器上没有可用的 `openssl` CLI —— 派生密钥要独立算出来才谈得上对照"
            );
            return;
        };

        let before = scan_with(&key, &mut buf);
        show("① 解锁中", &before);
        drop(conn);
        let after = scan_with(&key, &mut buf);
        show("② 丢掉连接", &after);

        // 正对照：解锁期间 SQLCipher 必须持有那把密钥（我们自己也持有一份）。
        assert!(
            before.len() >= 2,
            "解锁期间只扫到 {} 处派生密钥 —— 至少我们自己那份 + SQLCipher 那份，扫描器可能没在工作",
            before.len()
        );
        assert_eq!(
            after.len(),
            1,
            "丢掉连接之后还剩 {} 处派生密钥（只该剩本用例自己算出来的那一份）—— \
             `cipher_memory_security` 的擦零没有生效？",
            after.len()
        );
    }

    /// 判据 ④（口令那一侧的**另一条路**）：重新 `open` 一个已经存在的库也要走同一条生命周期。
    ///
    /// 只测 `create` 是不够的：`open` 是**真实使用**里那条路（建库只有一次），
    /// 而它多一段"校验 `user_version` 与四张表"。
    #[test]
    fn opening_an_existing_vault_also_returns_the_memory() {
        let dir = common::fixture_dir("unlock-lifecycle-reopen");
        let db = vault_path(&dir);

        let mut passphrase = Passphrase::new(vec![b'q'; 32]).unwrap();
        drop(create(&db, &mut passphrase).unwrap());

        let locked_before = common::locked_kb();
        let conn = open(&db, &mut passphrase).unwrap();
        let locked_open = common::locked_kb();
        drop(conn);
        let locked_after = common::locked_kb();
        drop(passphrase);

        println!("重新打开的 VmLck：{locked_before} → {locked_open} → {locked_after} kB");
        assert!(
            locked_open > locked_before,
            "重新打开没有多锁住任何内存 —— 那说明连接根本没被建起来"
        );
        assert_eq!(
            locked_after, locked_before,
            "丢掉连接之后 `VmLck` 必须回到打开前"
        );
    }
}
