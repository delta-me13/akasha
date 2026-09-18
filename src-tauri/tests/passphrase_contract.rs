//! plan 0402 的契约测试：**打开与新建是两条路**，以及口令真的被钉住了。
//!
//! 这一份守的是**我们的**行为（`sqlcipher_contract.rs` 守的是上游的）。核心那条来自一次
//! 意外的实测：一个不存在（或 0 字节）的文件，**用任何口令都能"打开"** ——
//! 库里没有任何东西可解，KDF 根本没跑。0401 的 `open()` 同时管新建与打开，于是
//! "打开"在新建这条路上等于没验证口令，还把这把错口令当成了创建口令。
//!
//! 所以这里的第一条断言就是：`create` 之后，**只有那一把口令**能开。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::fs;
use std::path::{Path, PathBuf};

use akasha_lib::store::{
    FORMAT_VERSION, Passphrase, STORE_FILE_NAME, StoreError, create, open, vault_path,
};

const PASSPHRASE: &[u8] = b"correct horse battery staple";
const OTHER: &[u8] = b"correct horse battery stapl";

fn pass(bytes: &[u8]) -> Passphrase {
    Passphrase::new(bytes.to_vec()).unwrap()
}

/// 每例一个干净目录，产物留在 `target/store-passphrase/` 供人工复核。
fn fixture_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/store-passphrase")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

// ── 1. 判据：新建的库从第一刻起只认那一把口令 ────────────────────────────────

#[test]
fn a_fresh_vault_only_accepts_the_passphrase_it_was_created_with() {
    let db = fixture_dir("only-one-passphrase").join(STORE_FILE_NAME);

    {
        let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
        // 一句话就够：`create` 那次写（建表 + `user_version`）就是文件的**第一次写页**，
        // 盐与密钥校验值就落在这里 —— 在那之前文件是 0 字节，谁的"口令"都成立。
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            FORMAT_VERSION
        );
    }

    let bytes = fs::read(&db).unwrap();
    println!("create 之后 = {} 字节", bytes.len());
    // 判据是"**实体化了**"，不是一个具体字节数：plan 0403 起 `create` 还要建 v1 的四张表，
    // 于是它从 0402 时的一页（4096）长成 9 页（36864，实测）。钉死数字只会让每次改表都红一次；
    // 真正的判据是"整页"（页是 SQLCipher 的写单位，半个页不该出现在文件里）。
    assert!(!bytes.is_empty(), "建库应当把文件实体化");
    assert_eq!(bytes.len() % 4096, 0, "库文件不是整页：{}", bytes.len());
    assert!(
        !bytes.starts_with(b"SQLite format 3"),
        "库头是明文 SQLite 魔数 —— 这个库没加密"
    );

    // 正确口令：开 + 能读
    let conn = open(&db, &mut pass(PASSPHRASE)).unwrap();
    conn.execute_batch("CREATE TABLE probe(x); INSERT INTO probe VALUES (1);")
        .unwrap();
    drop(conn);
    let conn = open(&db, &mut pass(PASSPHRASE)).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM probe", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);

    // 差一个字节的口令就不行 —— 这条在 0401 的写法下（新建即"打开"）根本测不出来
    let err = open(&db, &mut pass(OTHER)).unwrap_err();
    assert!(matches!(err, StoreError::NotADatabase), "实际是 {err:?}");
}

// ── 2. 不存在 / 0 字节 → NoVault（而不是"用任意口令建一个"）──────────────────

#[test]
fn open_refuses_a_vault_that_does_not_exist_yet() {
    let dir = fixture_dir("no-vault");

    let missing = dir.join(STORE_FILE_NAME);
    let err = open(&missing, &mut pass(PASSPHRASE)).unwrap_err();
    assert!(
        matches!(&err, StoreError::NoVault(path) if path == &missing),
        "实际是 {err:?}"
    );
    assert!(!missing.exists(), "open 不该在磁盘上留下任何东西");

    // 0 字节的残留文件同样算"没建过"。它是**最危险**的那种输入：
    // 换作 0401 的 open()，这里会成功、并且把临时口令当成创建口令。
    let empty = dir.join("leftover.db");
    fs::write(&empty, b"").unwrap();
    let err = open(&empty, &mut pass(OTHER)).unwrap_err();
    assert!(matches!(err, StoreError::NoVault(_)), "实际是 {err:?}");
    assert_eq!(fs::metadata(&empty).unwrap().len(), 0, "open 不该写它");
}

// ── 3. `create` 永不覆盖一个已有内容的库 ────────────────────────────────────

#[test]
fn create_never_overwrites_a_vault() {
    let db = fixture_dir("never-overwrite").join(STORE_FILE_NAME);
    {
        let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
        conn.execute_batch("CREATE TABLE probe(x); INSERT INTO probe VALUES (1);")
            .unwrap();
    }
    let before = fs::read(&db).unwrap();

    // 换一把口令去"建"同一个路径：必须是错误，而不是把用户的库清掉。
    let err = create(&db, &mut pass(OTHER)).unwrap_err();
    assert!(matches!(err, StoreError::VaultExists(_)), "实际是 {err:?}");
    assert_eq!(fs::read(&db).unwrap(), before, "库文件被动过了");

    // 而且原口令照旧能开（覆盖型的实现常常在"报错前"已经写坏文件）
    open(&db, &mut pass(PASSPHRASE)).unwrap();
}

#[test]
fn create_accepts_a_zero_byte_leftover() {
    // 0 字节 = 还没有密钥落在这条路径上（见 lib.rs 的 file_len），所以它是"可以建"的位置。
    // 若这里判 VaultExists，用户会陷入"删不掉也建不了"的死角。
    let db = fixture_dir("zero-byte-leftover").join(STORE_FILE_NAME);
    fs::write(&db, b"").unwrap();

    let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        FORMAT_VERSION
    );
    assert!(
        fs::metadata(&db).unwrap().len() > 0,
        "0 字节的残留位置应当变成一个真库"
    );
}

// ── 4. D7：版本是唯一的格式权威，打开时校验 ─────────────────────────────────

#[test]
fn open_rejects_a_version_it_does_not_know() {
    let dir = fixture_dir("version-check");

    for found in [FORMAT_VERSION + 1, 0] {
        let db = dir.join(format!("v{found}.db"));
        let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
        conn.pragma_update(None, "user_version", found).unwrap();
        drop(conn);

        let err = open(&db, &mut pass(PASSPHRASE)).unwrap_err();
        assert!(
            matches!(err, StoreError::UnsupportedVersion { found: f } if f == found),
            "版本 {found} 的实际结果是 {err:?}"
        );
    }

    // `< 1` 拒绝而不是"当成待迁移"：v1 之前没有版本，非空文件里出现 0 说明它不是本程序的库。
    // 接着往下走，就是"能开但内容不对"的入口（ADR-0002 §10 记了这条措辞改动）。
    let db = dir.join("v2-still-ok.db");
    let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
    drop(conn);
    open(&db, &mut pass(PASSPHRASE)).unwrap(); // 对照：版本对的时候当然能开
}

// ── 5. 落点：库与 `config.json` 同目录（D1），文件名叫 `akasha.db` ───────────

#[test]
fn vault_path_is_the_store_file_in_the_given_directory() {
    let dir = Path::new("/tmp/some-data-dir");
    assert_eq!(
        vault_path(dir),
        Path::new("/tmp/some-data-dir/akasha.db"),
        "D1：库文件名与落点是磁盘格式的一部分，改了等于迁移用户数据"
    );
    assert_eq!(vault_path(dir).file_name().unwrap(), STORE_FILE_NAME);
}

// ── 6. plan 0406：口令在内存里被护住（受保护页 / mlock / 不进 core dump）──────

/// 一页「静止不可读」的匿名内存：`(起始, 结束, VmFlags)`。只取 `---p` 且 ≤ 8 KiB 的段 ——
/// 我们那页是 4 KiB，而线程栈的 guard page 是几十 MB，一次就分开了。
///
/// `/proc/self/smaps` 的段头形如 `7f1f…-7f1f… ---p 00000000 00:00 0`，属性行**缩进**且是
/// `键: 值` —— 解析时别忘 `trim_start()`（第一版就栽在这里，`strip_prefix` 永远是 `None`）。
#[cfg(target_os = "linux")]
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

/// `/proc/self/status` 的进程级 `VmLck`（kB）—— 全部已 `mlock` 的内存。
#[cfg(target_os = "linux")]
fn locked_kb() -> u64 {
    let text = fs::read_to_string("/proc/self/status").unwrap();
    text.lines()
        .find_map(|line| line.strip_prefix("VmLck:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

/// 判据：口令那一页**静止时不可读**（`PROT_NONE`）、**已 mlock**、**不进 core dump**。
///
/// 这三条都是上游 `memsafe` 的承诺，而承诺要由我们来核 —— 它是年轻的小库（plan 0406），
/// 把安全属性押在它身上就必须有一条能红的测试：上游换了实现、或者这台机器上 `mlock`
/// 悄悄失败，这里要看得见。
///
/// "那一页是我们的"不靠猜：**建口令前后各取一次快照，多出来的那一页就是它**。
#[cfg(target_os = "linux")]
#[test]
fn the_passphrase_page_is_locked_and_excluded_from_core_dumps() {
    let dir = fixture_dir("memory-protection");
    let db = vault_path(&dir);

    let pages_before = noaccess_pages();
    let locked_before = locked_kb();

    let mut passphrase = pass(b"smaps-probe-passphrase");
    // 真跑一遍解锁路径：这样"下面那一页属于这个口令"不是推测
    // （`expose()` 是 `pub(crate)`，集成测试本来就够不着它，正好走公开路径）。
    create(&db, &mut passphrase).unwrap();

    let pages_after = noaccess_pages();
    let locked_after = locked_kb();

    let ours: Vec<_> = pages_after
        .iter()
        .filter(|page| !pages_before.contains(page))
        .collect();
    for (start, end, flags) in &ours {
        println!(
            "受保护页：{start:#x}-{end:#x}（{} 字节）VmFlags=[{flags}]",
            end - start
        );
    }
    assert_eq!(
        ours.len(),
        1,
        "建口令前后应当只多出一页「静止不可读」的匿名内存，实际 {}",
        ours.len()
    );

    // ① 静止态不可读：`PROT_NONE` 也意味着"直接去读会 SIGSEGV"，所以只能看 maps 的权限位
    assert_eq!(ours[0].1 - ours[0].0, 4096, "它应当正好一页");

    // ② mlock：进程级 VmLck 必须涨（这一页 4 KiB）
    println!("VmLck：{locked_before} kB → {locked_after} kB");
    assert!(
        locked_after >= locked_before + 4,
        "口令那一页没有 mlock —— 它可能被换进 swap：{locked_before} → {locked_after} kB"
    );

    // ③ 不进 core dump（Linux 的 MADV_DONTDUMP = `dd`）与 fork 时清零（`wf`）
    let flags = &ours[0].2;
    assert!(
        flags.split_whitespace().any(|f| f == "dd"),
        "口令那一页没有 MADV_DONTDUMP —— 它会被写进 core dump：{flags}"
    );
    assert!(
        flags.split_whitespace().any(|f| f == "wf"),
        "口令那一页没有 MADV_WIPEONFORK —— fork 出来的子进程会继承它：{flags}"
    );
}

/// **记录当前的边界**：`PROT_NONE` 挡不住 `/proc/self/mem`。
///
/// `/proc/<pid>/mem` 的读走 `FOLL_FORCE`，**绕过页保护** —— 上面那条 `---p` 的页照样读得出来
/// （实测：整页 4096 字节连口令原样返回）。所以这一条不是"我们期望的安全属性"，
/// 而是"别把它说大"的证据：`memsafe` 挡的是**越界读、误格式化、core dump、swap、fork**
/// 这些**意外**泄露，挡不住"已经能在你进程里跑代码的人"（那本来也不是这一层能解决的）。
///
/// ⚠️ 如果哪天这条**开始失败**（读不出来了），说明上游或内核把这一层做硬了 ——
/// 那时该回来重写模块文档与 ADR §10 的那段措辞，而不是把测试删掉。
#[cfg(target_os = "linux")]
#[test]
fn proc_self_mem_still_bypasses_page_protections() {
    use std::os::unix::fs::FileExt;

    let dir = fixture_dir("memory-protection-boundary");
    let db = vault_path(&dir);

    let pages_before = noaccess_pages();
    let mut passphrase = pass(b"boundary-probe");
    create(&db, &mut passphrase).unwrap();
    let ours: Vec<_> = noaccess_pages()
        .into_iter()
        .filter(|page| !pages_before.contains(page))
        .collect();
    let (start, end, _) = ours.first().expect("应当多出一页受保护内存").clone();

    let mem = fs::File::open("/proc/self/mem").unwrap();
    let mut buf = vec![0u8; (end - start) as usize];
    let read = mem.read_at(&mut buf, start);
    println!("/proc/self/mem 读那一页：{read:?}");
    assert!(
        read.is_ok(),
        "读不出来了 —— 保护变强了，回来重写 §10 与模块文档里「边界」那段"
    );
    assert!(
        buf.windows(8).any(|w| w == b"boundary"),
        "读到了那一页，但里面没有我们的口令 —— 那读的可能是别的页"
    );
}

/// 口令**不能**出现在错误里。`Passphrase` 没有 `Debug`（打不出来是编译错误），但错误类型
/// 本身是给日志与人看的 —— 这条守的是"哪天有人往 `StoreError` 里塞一个带口令的变体"。
/// 解锁失败（口令错）是最容易出这种事的一条路：那正是要把口令回显给用户"再试一次"的时刻。
#[test]
fn errors_never_carry_the_passphrase() {
    let db = fixture_dir("errors-no-leak").join(STORE_FILE_NAME);
    let marker = b"marker-only-in-my-head-4711";
    create(&db, &mut pass(marker)).unwrap();

    let err = open(&db, &mut pass(b"marker-only-in-my-head-4712")).unwrap_err();
    let rendered = format!("{err} {err:?}");
    assert!(
        !rendered.contains("marker-only-in-my-head"),
        "错误文本里带出了口令：{rendered}"
    );
}
