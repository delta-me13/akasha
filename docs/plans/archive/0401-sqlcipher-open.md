# Plan 0401: `rusqlite` + SQLCipher 打开加密库

- **关联**：ROADMAP 阶段 4 ·「`rusqlite` + **SQLCipher**（`bundled-sqlcipher-vendored-openssl`）」；
  [ADR-0002](../../adr/0002-secret-storage.md) D2 / D3 / D4 / D5 / D8 —— 本 plan 是它们的落地
- **前置**：plan 0400（ADR-0002 已进入实现中）
- **状态**：已完成（2026-09-12）—— 契约测试 9/9、`just ready` 6/6

## 目标

新建 `src-tauri/crates/akasha-store/`（纯逻辑、零 Tauri 依赖，`AGENTS.md` §3.1），
把"**用口令打开一个 SQLCipher 库**"这一件事做对；并把 ADR-0002 §7 的 7 项**实测跑掉** ——
那 7 项在 ADR 里写的是"预期"，本 plan 的产物是"事实"。

调用顺序不可换（D4）：`Connection::open` → **立刻** `sqlite3_key()` → 才发第一条语句。
SQLCipher 是"首次用到密钥时才派生"，任何 PRAGMA 与查询都算"用到"。违反的表现是
**库打不开**，而错误信息不会指向"你少调了一次 key"。

## 非目标

- 口令的应用层封装、解锁界面、自动锁定（plan 0402）
- 四套池的表结构与 CRUD（plan 0403）
- 导出的落点选择、确认门槛、导入（plan 0404）—— 本 plan 只**实测** D6/D7 的格式事实
- **把 `akasha-store` 接进 app**：现在还没有消费者（plan 0403 才有）。空依赖会掩盖
  "分层到底通不通"这件事，所以 `akasha/Cargo.toml` 这一步不动

## 判据（来自 ROADMAP）

- 用**错误口令打不开库**
- `.db` 文件里**搜不到明文密钥**

## 步骤

### 1. 建 crate 与依赖

```bash
cd src-tauri
cargo new --lib crates/akasha-store        # 自动落进 members = ["crates/*"]
cargo add -p akasha-store rusqlite --features bundled-sqlcipher-vendored-openssl
cargo add -p akasha-store thiserror
cargo add -p akasha-store --dev tempfile
```

⚠️ **`unsafe` 这一刀比预想的难切**（展开时实测）：workspace 是 `unsafe_code = "forbid"`，
而 rustc 规定 **`forbid` 不能被 `#[allow]` 覆盖** —— 也就是说 `AGENTS.md` §3.4 承诺的
"单点 `#[allow(unsafe_code)]` + `// SAFETY:` 注释"在当前配置下**根本写不出来**。
想"只在本 crate 放宽"也走不通：cargo **不允许部分覆盖**继承来的 lint
（`cannot override workspace.lints ...`），继承是全有或全无。

所以实际做法是两步，缺一不可：

1. workspace 的 `unsafe_code` 由 `forbid` 改为 **`deny`**（root `Cargo.toml` 里写清代价）；
2. 新增 `.ast-grep/rules/no-unsafe-outside-store.yml`：`unsafe` **只许出现在
   `src-tauri/crates/akasha-store/**`**，把 `forbid` 给出的那条保证按规则补回来。

规则按 `AGENTS.md` §6 用**一对探针**验证过再删：`src-tauri/src/` 下的真 `unsafe` 命中，
`akasha-store/` 下的同类不命中，注释与字符串里的 "unsafe"（诱饵）不命中。

### 2. `open()`：口令是打开后的第一个操作

- `StoreError`：空口令（D5）单独一支；`SQLITE_NOTADB`（错误口令**或**文件损坏）单独一支，
  名字照实叫 `NotADatabase` —— SQLCipher 不区分这两者，我们也不假装能区分；
- `open(path, passphrase: &[u8]) -> Result<Connection, StoreError>`：空口令**在碰文件之前**就返回；
- `sqlite3_key` 之后立刻发一条 `SELECT count(*) FROM sqlite_master` 作为**验钥匙**：
  口令错的话失败发生在这里，而不是在几十行之后的某次写入；
- `unsafe` 那一段：`// SAFETY:` 说明 `handle()` 是本连接的活句柄、`key` 的
  `ptr/len` 在调用期间有效、`sqlite3_key` 不在调后持有它；
- **不设 `journal_mode`**（D8）：默认就是 `DELETE`，改它反而会引入 `-wal`。

### 3. 契约测试 `tests/sqlcipher_contract.rs`（ADR-0002 §7 的 7 项）

每例头部写明它守的是哪条 D。前两例是 ROADMAP 判据，其余是"上游事实"的回归护栏 ——
上游换版本时它们该红。

1. `cipher_settings_reports_adr_parameters` —— `PRAGMA cipher_settings` 的实际输出
2. `cipher_version_is_sqlcipher_4` —— 经 `rusqlite::ffi` 拿到 `sqlite3_key`，并记下实际版本
3. `wrong_passphrase_cannot_open` —— 错误口令的**确切报错形态**（判据 ①）
4. `empty_passphrase_refused_before_touching_file` —— 应用层拒绝；
   同文件另一例 `empty_key_really_disables_encryption` 用**裸 ffi** 证明 D5 的理由不是猜测
5. `plaintext_export_roundtrip_and_user_version_not_copied` —— D6/D7 的格式事实
6. `plaintext_secret_not_found_in_db_file` —— `.db` 里 grep 不到明文私钥（判据 ②）
7. `rekey_changes_salt` —— 决定 ADR §5.2 的时序

fixture 故意落在 `target/store-contract/`（不是 tempdir）：判据 ② 是**安全声明**，
必须能拿一个真实文件手工 `grep` 复核。测试开头清掉上一次的残留。

### 4. 实测值记回文档

- ADR-0002 §7 的 7 个复选框打勾，**并把实际值写进去**（"预期"改成"实测"）；
- 与 ADR 写的不一致 → 改 ADR 并在 **§10 修订记录**记一行（实现中允许就地修订）；
- 顺带记一条 D12 相关的事实：**SQLite 默认建出的库文件权限是什么**（不是 0600 就说明
  0600 必须由数据目录那一侧显式设置，plan 0405 不能假设它已经是）。

### 5. 门禁

`just ready` 全绿；`cargo tree -p akasha-store` 里出现 `openssl-src`（vendored，不是系统
OpenSSL）；`Cargo.lock` 差异已提交。

## 验收命令

```bash
# ① 全部单测 + 契约测试（判据 ① ② 都在里面，具名可查）
cd src-tauri && cargo nextest run -p akasha-store --no-capture --no-fail-fast
# 期望：9 passed, 0 failed；输出里有
#   cipher_version = 4.5.7 community / cipher_provider_version = OpenSSL 3.6.3
#   PRAGMA kdf_iter = 256000; · cipher_page_size = 4096; · HMAC_SHA512 · PBKDF2_HMAC_SHA512
#   raw error = SqliteFailure(Error { code: NotADatabase, extended_code: 26 },
#                             Some("file is not a database"))

# ② 判据 ② 的手工复核（fixture 是真实文件，不是内存库）
grep -c 'BEGIN OPENSSH PRIVATE KEY' src-tauri/target/store-contract/plaintext-grep/akasha.db
# 期望输出 0；⚠️ grep 无匹配时退出码是 1，不是失败
# 反面样本（空 key 那个库）应当输出 1 —— 说明这条 grep 真的能搜到东西：
grep -c 'BEGIN OPENSSH PRIVATE KEY' src-tauri/target/store-contract/empty-key-raw/akasha.db

# ③ 加密库确实不是明文库：头部没有 SQLite 魔数
head -c 16 src-tauri/target/store-contract/plaintext-grep/akasha.db | od -c | head -1
# 期望：不是 "SQLite format 3"（那是明文库的特征）

# ④ 链的是 vendored SQLCipher，不是系统 OpenSSL / 裸 SQLite
cd src-tauri
cargo tree -p akasha-store -e normal,build --prefix none | grep -c '^openssl-src'   # 期望 1
cargo tree -p akasha-store -e features --prefix none -i libsqlite3-sys | grep sqlcipher
# 期望：能看到 "bundled-sqlcipher" 与 "bundled-sqlcipher-vendored-openssl" 两条特性边

# ⑤ 门禁
just ready
```

## 回滚

`git revert` 整个提交即可：这一步**没有消费者**，删掉 crate 与依赖，app 行为不变 ——
唯一的痕迹是提交里的这次实测记录（它是本 plan 的价值，不该回滚）。

## 实施记录

**实测值**（ADR-0002 §7 的 7 项，全部跑完）：

| 项 | 实测 |
|---|---|
| `cipher_settings` 输出形态 | 一列 `pragma`，每行一条 `PRAGMA <名> = <值>;` |
| 参数取值 | `kdf_iter = 256000` · `cipher_page_size = 4096` · `HMAC_SHA512` · `PBKDF2_HMAC_SHA512` |
| 版本 | SQLCipher **4.5.7 community**，provider `openssl`，**OpenSSL 3.6.3**（`openssl-src 300.6.1+3.6.3`） |
| 错误口令 | `SqliteFailure(Error { code: NotADatabase, extended_code: 26 }, Some("file is not a database"))` |
| 空口令 | `sqlite3_key(…, 0)` **返回 `SQLITE_ERROR`（1）**，然后连接**照常可用** → 得到一个明文库 |
| 明文导出 | round-trip 通过；**`user_version` 不传递**（源库 7 → 导出 0），源库自己不受影响 |
| 明文 grep | 库 8192 字节，头部非 `SQLite format 3`，`grep` 私钥 **0** 命中（空 key 的对照库 1 命中） |
| `rekey` 后的盐 | **不变**（前 16 字节逐字节相同）—— §5.2 的时序依据 |
| D12 的权限 | SQLite 自己建出来是 **644**；`open()` 之后 **600**（不是白做的） |

**与 ADR 预期不符的只有一处**：D5 把空 key 说成"关闭加密"，实测机制是
`sqlite3_key_v2` 在 `nKey == 0` 时**直接 `return SQLITE_ERROR`**、根本不挂 codec
（源码 `libsqlite3-sys-0.30.1/sqlcipher/sqlite3.c:107794`）。**危险点反而更尖锐**：
返回错误之后连接照常能用，所以"不看返回值"就会写出一个明文库，而每一步都"成功"。
已在 ADR §10 记一行并改掉 D5 的括号说明。

**额外发现（不在原计划的 7 项里）**：`victauri-plugin` 已经依赖 `rusqlite ^0.32`，
而 `libsqlite3-sys` 带 `links = "sqlite3"` —— cargo 不允许同一原生库两个版本，
所以本 crate 的 rusqlite 版本**不是我们选的**（0.40.2 直接被拒）。好处是 features 并集，
全 app 从此只有一个 sqlite，而且是我们指定的那个 SQLCipher。

**一处已知的过渡态**：`akasha-store` 还没被 app 依赖（plan 0403 才接），于是
`cargo build -p akasha` 与 `cargo build --workspace` 会用**不同的 rusqlite feature 集**
（前者没有 SQLCipher）—— 来回切会重编一次 `libsqlite3-sys`。接进 app 之后消失。

