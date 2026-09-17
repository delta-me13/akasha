//! SQLCipher **契约测试** —— ADR-0002 §7 的实测清单落地处。
//!
//! 这里守的不是"我们的代码有没有写错"，而是**上游库的行为**：默认参数集、
//! 错误口令的报错形态、空 key 的后果、`sqlcipher_export` 传不传 `user_version`、
//! `rekey` 后盐变不变。它们在 ADR-0002 里是设计依据 —— 上游换版本时这些测试该红，
//! 而不是让 ADR 里的一段推断悄悄失效。
//!
//! 其中两条就是 ROADMAP 的判据本身：**错误口令打不开库**、**`.db` 里搜不到明文密钥**。
//!
//! fixture 故意落在 `target/store-contract/`（不是 tempdir）：判据 ② 是关于磁盘位与字节的
//! **安全声明**，必须能拿一个真实文件手工 `grep` 复核 —— 见 plan 0401 的「验收命令」。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use akasha_store::{Passphrase, StoreError, create, open};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, ffi};

/// 契约用的口令与"私钥"内容。内容是**一眼能认出来的字符串**，
/// 这样"库里 grep 不到"才是一条能被人复核的判据。
const PASSPHRASE: &[u8] = b"correct horse battery staple";
const NEW_PASSPHRASE: &[u8] = b"another correct horse battery staple";
const SECRET: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nakasha-contract-fixture\n-----END OPENSSH PRIVATE KEY-----\n";

/// 把字节包成口令。`Passphrase` 刻意不实现 `Clone`，所以每个用例各自造一份 ——
/// 口令的副本只有一个来源，多一个就得回答"为什么要多这一个"（ADR-0002 D5）。
fn pass(bytes: &[u8]) -> Passphrase {
    Passphrase::new(bytes.to_vec()).unwrap()
}

/// 每例一个干净目录；`target/store-contract/` 下的产物**故意留着**给人工复核。
fn fixture_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/store-contract")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn render(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).to_string(),
        ValueRef::Blob(b) => format!("<{} bytes>", b.len()),
    }
}

/// 把一条 `PRAGMA` 的全部列与行摊成文本 —— `cipher_settings` 的形状（多列一行 /
/// 两列多行）随上游版本变过，硬编码列名会让测试在无关的地方红。
///
/// 取不到就返回失败说明而不是 panic：那样断言失败时**看得到上游的原话**。
fn dump_pragma(conn: &Connection, pragma: &str) -> String {
    let mut stmt = match conn.prepare(pragma) {
        Ok(stmt) => stmt,
        Err(err) => return format!("<{pragma} 取不到: {err}>"),
    };
    let cols: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut rows = match stmt.query([]) {
        Ok(rows) => rows,
        Err(err) => return format!("<{pragma} 查不动: {err}>"),
    };
    let mut out = String::new();
    while let Some(row) = rows.next().unwrap() {
        for (i, col) in cols.iter().enumerate() {
            let cell = row
                .get_ref(i)
                .map(render)
                .unwrap_or_else(|err| format!("<{err}>"));
            out.push_str(&format!("  {col} = {cell}\n"));
        }
    }
    out
}

/// 取一条单列文本 PRAGMA 的取值。
fn text_pragma(conn: &Connection, pragma: &str) -> String {
    conn.query_row(pragma, [], |row| row.get(0)).unwrap()
}

/// 字节 → 小写十六进制（`cipher_salt` 的格式）。
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ── 1. 参数集就是 ADR-0002 D2 写的那一套 ─────────────────────────────────────

#[test]
fn cipher_settings_reports_adr_parameters() {
    let db = fixture_dir("cipher-settings").join("akasha.db");
    let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();

    let settings = dump_pragma(&conn, "PRAGMA cipher_settings");
    println!("PRAGMA cipher_settings:\n{settings}");

    // D2 的"不设任何参数"只有在**默认值恰好是这一套**时才等于"冻结了这套参数"，
    // 所以真正要断言的是这几个取值本身。
    assert!(
        settings.contains("kdf_iter = 256000"),
        "kdf_iter 不再是 256000，D2/D3 需要重新论证:\n{settings}"
    );
    assert!(
        settings.contains("cipher_page_size = 4096"),
        "cipher_page_size 不再是 4096:\n{settings}"
    );
    assert!(
        settings.contains("cipher_hmac_algorithm = HMAC_SHA512"),
        "每页 HMAC 不再是 SHA512:\n{settings}"
    );
    assert!(
        settings.contains("cipher_kdf_algorithm = PBKDF2_HMAC_SHA512"),
        "KDF 不再是 PBKDF2-HMAC-SHA512:\n{settings}"
    );

    // D8：日志模式是默认的 DELETE（不是 WAL）—— 用 WAL 会多出 `-wal` / `-shm`，
    // 而"复制单个 .db 即完整"（P2）正是靠没有它们成立的。
    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal.to_lowercase(), "delete", "D8：不该是 WAL");
}

// ── 2. `sqlite3_key` 来自 `rusqlite::ffi`，且链的确实是 SQLCipher ────────────

#[test]
fn cipher_version_is_sqlcipher_4() {
    let db = fixture_dir("cipher-version").join("akasha.db");
    let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();

    // 能读到 cipher_version 本身就说明链的是 SQLCipher 而不是裸 SQLite
    // （裸 SQLite 上这个 PRAGMA 报错）。ADR-0002 §7 要求记下实际版本与构建出的 OpenSSL。
    let version: String = conn
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .unwrap();
    println!("cipher_version = {version}");
    println!("{}", dump_pragma(&conn, "PRAGMA cipher_provider"));
    println!("{}", dump_pragma(&conn, "PRAGMA cipher_provider_version"));

    assert!(
        version.starts_with('4'),
        "ADR-0002 的全套结论建立在 SQLCipher 4 上，实际是：{version}"
    );

    // 反向确认："能开"不是因为库根本没加密 —— 一个非库文件用同一把口令照样打不开
    let not_a_db = fixture_dir("not-a-database").join("plain.db");
    fs::write(&not_a_db, b"SQLite format 3\0not really a database").unwrap();
    assert!(matches!(
        open(&not_a_db, &mut pass(PASSPHRASE)).unwrap_err(),
        StoreError::NotADatabase
    ));
}

// ── 3. 判据 ①：错误口令打不开 ───────────────────────────────────────────────

#[test]
fn wrong_passphrase_cannot_open() {
    let dir = fixture_dir("wrong-passphrase");
    let db = dir.join("akasha.db");

    {
        let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
        conn.execute_batch("CREATE TABLE key_pool(secret TEXT);")
            .unwrap();
    }

    // 先记下**上游的原话**：ADR-0002 §7 要的是"确切的报错形态"，
    // 而 `StoreError::NotADatabase` 是我们归一化之后的说法。
    let raw = Connection::open(&db).unwrap();
    let raw_err = raw
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_err();
    println!("raw error = {raw_err:?}");
    drop(raw);

    let err = open(&db, &mut pass(b"wrong passphrase")).unwrap_err();
    println!("StoreError = {err}");
    assert!(matches!(err, StoreError::NotADatabase), "实际是 {err:?}");

    // 正确口令仍然打得开 —— 否则这条判据可能只是因为"这个文件本来就打不开"
    open(&db, &mut pass(PASSPHRASE)).unwrap();
}

// ── 4. 空口令：应用层拦下，且理由不是猜测 ──────────────────────────────────

#[test]
fn empty_passphrase_refused_before_touching_file() {
    let dir = fixture_dir("empty-passphrase");
    let db = dir.join("akasha.db");

    // plan 0402：这道校验从 `open` 里的一个分支升级成**类型不变量** ——
    // 空口令不是"调用时被拒绝"，而是**根本造不出来**，所以没有哪条调用路径能忘记检查它。
    assert!(matches!(
        Passphrase::new(Vec::new()),
        Err(StoreError::EmptyPassphrase)
    ));
    // "不碰文件"因此是更强的意思：这条路径**走不到文件系统**
    assert!(!db.exists(), "空口令不该在磁盘上留下任何东西");

    // 反面（否则上面那条断言可能只是"构造函数永远失败"）：非空造得出来，`create` 也认它
    let _conn = create(&db, &mut pass(b"x")).unwrap();
    assert!(db.exists());
}

#[test]
#[allow(unsafe_code)] // 本 crate 是唯一允许碰 ffi 的地方，见 scripts/ast-grep/rules/no-unsafe-outside-store.yml
fn empty_key_really_disables_encryption() {
    let dir = fixture_dir("empty-key-raw");
    let db = dir.join("akasha.db");

    // 绕开应用层校验，直接照 SQLCipher 的语义走一次空 key。
    // 这一例存在的唯一目的：证明 D5 的理由是**事实**，而不是"文档说要小心"。
    //
    // 实测到的机制（与"空 key 悄悄关掉加密"这个说法不太一样，见 ADR-0002 §10）：
    // `sqlite3_key_v2` 在 `nKey == 0` 时**直接返回 SQLITE_ERROR**，根本不挂 codec
    // —— 于是连接退化成**明文 sqlite**。危险之处正在这里：**只看返回值而不中断**
    // （`let _ = sqlite3_key(...)`）就会得到一个明文库，而后面每一步都"正常成功"。
    let conn = Connection::open(&db).unwrap();
    // SAFETY: 同 `akasha_store::apply_key` —— 活句柄、本次调用期间有效的指针、长度 0。
    let rc = unsafe { ffi::sqlite3_key(conn.handle(), b"".as_ptr().cast(), 0) };
    println!(
        "sqlite3_key(empty) rc = {rc}（SQLITE_ERROR = {}）",
        ffi::SQLITE_ERROR
    );
    assert_eq!(rc, ffi::SQLITE_ERROR, "空 key 不是静默接受，而是报错");

    // 报错之后连接**照样能用** —— 这就是"必须检查返回值"的原因
    conn.execute_batch(&format!(
        "CREATE TABLE key_pool(secret TEXT); INSERT INTO key_pool VALUES ('{SECRET}');"
    ))
    .unwrap();
    drop(conn);

    let bytes = fs::read(&db).unwrap();
    assert!(
        contains(&bytes, SECRET.as_bytes()),
        "预期空 key = 未加密（明文可读）—— 若这里不再成立，D5 的论证要重写"
    );
}

// ── 5. 导出：D6 的容器与 D7 的版本字段 ─────────────────────────────────────

#[test]
fn plaintext_export_roundtrip_and_user_version_not_copied() {
    let dir = fixture_dir("plaintext-export");
    let db = dir.join("akasha.db");
    let out = dir.join("export-plain.db");

    let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE key_pool(secret TEXT); INSERT INTO key_pool VALUES ('{SECRET}');"
    ))
    .unwrap();
    // D7：`user_version` 是格式版本的唯一权威；导出**不传递**它，
    // 所以导出时必须**显式**再写一次 —— 这一例就是那条"必须"的证据。
    conn.pragma_update(None, "user_version", 7).unwrap();

    conn.execute_batch(&format!(
        "ATTACH DATABASE '{}' AS export KEY '';",
        out.display()
    ))
    .unwrap();
    conn.query_row("SELECT sqlcipher_export('export')", [], |_| Ok(()))
        .unwrap();
    conn.execute_batch("DETACH DATABASE export;").unwrap();

    // 明文导出可以用**裸 SQLite** 打开（不设任何 key）：这正是"明文"的含义，
    // 也是 plan 0404 的确认门槛要拦的那个东西。
    let plain = Connection::open(&out).unwrap();
    let secret: String = plain
        .query_row("SELECT secret FROM key_pool", [], |r| r.get(0))
        .unwrap();
    assert_eq!(secret, SECRET);

    let exported_version: i64 = plain
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    println!("exported user_version = {exported_version}（源库是 7）");
    assert_eq!(
        exported_version, 0,
        "sqlcipher_export 不传递 user_version（D7 的前提）"
    );

    let source_version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(source_version, 7, "导出不该动源库的版本字段");
}

// ── 6. 判据 ②：`.db` 里搜不到明文密钥 ──────────────────────────────────────

#[test]
fn plaintext_secret_not_found_in_db_file() {
    let dir = fixture_dir("plaintext-grep");
    let db = dir.join("akasha.db");

    {
        let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE key_pool(secret TEXT); INSERT INTO key_pool VALUES ('{SECRET}');"
        ))
        .unwrap();
    }

    let bytes = fs::read(&db).unwrap();
    println!("store file = {}（{} 字节）", db.display(), bytes.len());
    println!("header = {:?}", String::from_utf8_lossy(&bytes[..16]));

    assert!(
        !contains(&bytes, SECRET.as_bytes()),
        "明文私钥出现在库文件里"
    );
    assert!(
        !contains(&bytes, b"BEGIN OPENSSH PRIVATE KEY"),
        "明文私钥头出现在库文件里"
    );
    assert!(
        !bytes.starts_with(b"SQLite format 3"),
        "库头是 SQLite 魔数 —— 说明这个库根本没加密"
    );
}

// ── 7. D12：文件权限 ───────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn open_restricts_file_to_owner() {
    let dir = fixture_dir("permissions");

    // 先看 SQLite **自己**建出来是什么权限（不经 `open`）：这决定了 0600 是不是白做的。
    // 实测 644（umask 022）—— 也就是说"库里是私钥、文件却世界可读"是默认行为，
    // D12 那条不是装饰。⚠️ 这里必须用**另一个文件**：拿明文库去喂 `open` 只会得到 NotADatabase。
    let unhardened = dir.join("sqlite-default.db");
    let raw = Connection::open(&unhardened).unwrap();
    raw.execute_batch("CREATE TABLE t(x);").unwrap();
    drop(raw);
    let default_mode = fs::metadata(&unhardened).unwrap().permissions().mode() & 0o777;
    println!("sqlite 默认建出的权限 = {default_mode:o}");

    let db = dir.join("akasha.db");
    let _conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
    let hardened = fs::metadata(&db).unwrap().permissions().mode() & 0o777;
    println!("经 create() 之后的权限 = {hardened:o}");
    assert_eq!(hardened, 0o600, "D12：库文件应当是 0600");
    assert_ne!(
        default_mode, 0o600,
        "若 SQLite 本来就给 0600，这条断言就失去意义 —— 该重新论证 D12"
    );
}

// ── 8. §5.2 的时序：`rekey` 之后盐变不变 ───────────────────────────────────

#[test]
#[allow(unsafe_code)] // 同上：`rekey` 与 `key` 一样只有 C API，不能走 SQL 文本（D4）
fn rekey_keeps_content_and_reports_salt_behaviour() {
    let dir = fixture_dir("rekey");
    let db = dir.join("akasha.db");

    let conn = create(&db, &mut pass(PASSPHRASE)).unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE key_pool(secret TEXT); INSERT INTO key_pool VALUES ('{SECRET}');"
    ))
    .unwrap();
    let before = fs::read(&db).unwrap()[..16].to_vec();

    // 用 C API 而不是 `PRAGMA rekey = '…'`：与本 crate 对 `key` 的纪律一致（D4），
    // 而且 §5.2 的升级路径本来就要用这个调用。
    // SAFETY: 同 `akasha_store::apply_key`。
    let rc = unsafe {
        ffi::sqlite3_rekey(
            conn.handle(),
            NEW_PASSPHRASE.as_ptr().cast(),
            NEW_PASSPHRASE.len() as i32,
        )
    };
    assert_eq!(rc, ffi::SQLITE_OK);
    drop(conn);

    let after = fs::read(&db).unwrap()[..16].to_vec();
    println!("salt before  = {before:02x?}");
    println!("salt after   = {after:02x?}");
    println!("salt changed = {}", before != after);

    // 实测：**盐不变**。这条是 ADR-0002 §5.2 的时序依据（"原地换 KDF 参数"要按
    // 盐会不会被重生成来排队），上游改了它就得回去重排 —— 所以它值得一条断言。
    assert_eq!(
        before, after,
        "rekey 之后盐变了：ADR-0002 §5.2 的升级时序要重排"
    );

    // 真正要守住的不变量：换口令之后**新口令能开、旧口令不能**，且内容还在
    {
        let conn = open(&db, &mut pass(NEW_PASSPHRASE)).unwrap();
        let secret: String = conn
            .query_row("SELECT secret FROM key_pool", [], |r| r.get(0))
            .unwrap();
        assert_eq!(secret, SECRET);
    }
    assert!(matches!(
        open(&db, &mut pass(PASSPHRASE)).unwrap_err(),
        StoreError::NotADatabase
    ));
}

// ── 9. plan 0402：盐在文件头，且每个库各不同（D2 的直接证据）───────────────

#[test]
fn cipher_salt_is_the_file_header_and_differs_per_vault() {
    let dir = fixture_dir("cipher-salt");
    let a = dir.join("a.db");
    let b = dir.join("b.db");

    // 同一个口令建两个库。D2 说盐是"16 字节随机、存于库文件头部前 16 字节"，
    // 而"随机"与"在头部"这两半都只在**两个库对比**时才看得出来。
    let conn_a = create(&a, &mut pass(PASSPHRASE)).unwrap();
    let conn_b = create(&b, &mut pass(PASSPHRASE)).unwrap();
    let salt_a = text_pragma(&conn_a, "PRAGMA cipher_salt");
    let salt_b = text_pragma(&conn_b, "PRAGMA cipher_salt");
    println!("salt a = {salt_a}\nsalt b = {salt_b}");

    assert_eq!(salt_a.len(), 32, "盐应当是 16 字节的十六进制：{salt_a}");
    assert!(salt_a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(
        salt_a, salt_b,
        "同一个口令建的两个库拿到了同一个盐 —— 盐不是随机生成的，D2/D3 要重看"
    );

    // "存于文件头部前 16 字节"是可断言的：`cipher_salt` 与文件头逐字节相同
    let head_a = hex(&fs::read(&a).unwrap()[..16]);
    assert_eq!(salt_a, head_a, "盐没落在文件头前 16 字节");
    assert_eq!(salt_b, hex(&fs::read(&b).unwrap()[..16]));

    // 口令相同、盐不同 → 密文必须不同。两个文件逐字节相同说明盐根本没参与派生。
    assert_ne!(
        fs::read(&a).unwrap(),
        fs::read(&b).unwrap(),
        "两个库的字节完全相同：盐没有参与密钥派生"
    );
}

// ── 10. plan 0402：内存安全是**进程级**，且只能开不能关（§6 的更正依据）────

#[test]
fn memory_security_can_be_enabled_but_never_turned_off() {
    let dir = fixture_dir("memory-security");
    let conn = create(&dir.join("akasha.db"), &mut pass(PASSPHRASE)).unwrap();

    // 我们的打开路径本来就开了它（ADR-0002 §6 的"建议开"），所以建库之后读回来应当是 "1"。
    // ⚠️ 读回来的是 `on && executed` 的合取（`executed` = 安全分配器被用过至少一次），
    // 所以这条断言同时说明"确实开了"和"确实在走那个分配器"。
    assert_eq!(
        text_pragma(&conn, "PRAGMA cipher_memory_security"),
        "1",
        "内存安全没生效 —— 上游 sqlcipher_init_memmethods 可能没装上包装分配器"
    );

    // **关不掉**：上游 `sqlcipher_set_mem_security` 的实现是 `if(on) { … }`，
    // 设 OFF 既不报错也没有效果。§6 原写的"连接级开关"两半都不对（进程级 + 单向），
    // 这条断言就是那个更正的护栏。
    conn.execute_batch("PRAGMA cipher_memory_security = OFF")
        .unwrap();
    assert_eq!(
        text_pragma(&conn, "PRAGMA cipher_memory_security"),
        "1",
        "居然关得掉了：ADR-0002 §6 把这条写成\"可开关的连接级设置\"就又不算错了，回去改"
    );
}
