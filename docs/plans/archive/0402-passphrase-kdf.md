# Plan 0402: 口令 → KDF → 库密钥

- **关联**：ROADMAP 阶段 4 ·「口令 → KDF → 库密钥（**不依赖 OS keychain**）」；
  [ADR-0002](../../adr/0002-secret-storage.md) D2 / D3 / D5 / D7 / D12 与 §6 的落地
- **前置**：plan 0401（库能开 —— `akasha-store` 已存在）
- **状态**：已完成（2026-09-12）
- ⚠️ **后续（2026-09-12）**：本文「明确不做内存擦除」的**结论已被推翻** ——
  口令改为一整页受保护内存（`mlock` / 静止不可读 / 不进 core dump / fork 清零），
  见 [plan 0406](./0406-memsafe-passphrase-page.md)。本文其余部分仍是当时的记录。—— 单测 22/22（`akasha-store`）、`just ready` 6/6

## 目标

把"口令"变成**只有一条路能进来的类型**，并把 0401 的打开路径补成**能真的验证口令**的路径。

三件具体的事，前两件来自 0401 之后暴露出来的东西，第三件是 §6 尚未覆盖的一项：

1. **`Passphrase` 类型**：空值**不可表示**（不是"打开函数里有个 if"）、`Debug` 输出不含内容、
   没有 `Display` / `Serialize` —— 于是"口令进日志"不是靠自觉，而是**写不出来**（D5）。
2. **`open` / `create` 拆开**：实测（见实施记录）**文件不存在或为 0 字节时，任何口令都能"打开"** ——
   空的库里没有任何东西可解，KDF 根本没有运行。也就是说 0401 的 `open()` 在"新建"这条路上
   **没有验证过口令**，而它同时把这把错口令当成了创建口令。
   `create` 用 `PRAGMA user_version = 1`（D7 的 v1）把口令**钉进文件**，此后只有这一把能开。
3. **`cipher_memory_security = ON`**（§6 的"建议开"）：让 SQLCipher 的密钥材料在释放时被擦除。
   实测它是**进程级、只能开不能关**，§6 原写的"连接级开关"两半都不对。

## 非目标

- **表结构与建表**（plan 0403）：`create` 只写版本号，**一列都不建**
- 解锁界面 / 口令强度 / 自动锁定（`scope.md`；当前前端是验证壳层，`AGENTS.md` §4.0）
- 导出的独立口令（plan 0404）
- **把 `akasha-store` 接进 app**（plan 0403）—— 仍无消费者，所以本步不新增 command，
  也就没有 Victauri 可驱动的真实路径（`AGENTS.md` §7 那一条在 0403 补）
- **口令的内存擦除**（`zeroize` 那类）：**明确不做**。库一解锁，私钥与页缓存就同样躺在内存里，
  单独擦掉一个输入缓冲不改变威胁模型 —— 边界是"文件是密文"（D5/D12）。做了的是
  `cipher_memory_security`（擦 SQLCipher 自己分配的那一份），理由写进 crate 文档

## 判据（来自 ROADMAP）

- **无任何 `keyring` 类依赖**（P1）
- **口令不以任何形式出现在磁盘上**（配置文件、日志、临时文件都不行）

实现暴露出来的两条一并作为判据（否则第 2 条在"新建"这条路上有空档）：

- **新建的库从第一刻起只认那一把口令**：`create` 之后换任何口令都打不开
- **口令打不进日志**：`format!("{passphrase:?}")` 里不含口令本身

## 步骤

### 1. `Passphrase`（`src/passphrase.rs`）

- `Passphrase::new(Vec<u8>) -> Result<Self, StoreError>`：空 → `EmptyPassphrase`（D5）。
  "碰文件之前就拒绝"因此变成"空值根本造不出来"；
- **不实现 `Clone`**（多一份副本就得回答为什么）；`expose()` 是 `pub(crate)` ——
  口令流出本 crate 必须是显式动作，reviewer 能直接看到它去了哪；
- 手写 `Debug`：只输出 `<redacted>`，**不输出长度**；
- 口令是任意字节（`Vec<u8>`，不是 `String`）：`String` 会强制 UTF-8，还会在堆上多留一份。

### 2. `open` 与 `create` 分开

```rust
pub const FORMAT_VERSION: i64 = 1;                        // D7 的 v1
pub fn vault_path(data_dir: &Path) -> PathBuf;            // data_dir/akasha.db（D1）
pub fn open(path: &Path, passphrase: &Passphrase) -> Result<Connection, StoreError>;
pub fn create(path: &Path, passphrase: &Passphrase) -> Result<Connection, StoreError>;
```

- `create`：文件**存在且有内容** → `VaultExists`（**永不覆盖**一个库）；0 字节的残留算"可以建"；
  建库 → 送密钥 → `PRAGMA user_version = 1`（这一步同时把文件实体化）。到此为止。
- `open`：文件不存在 / 0 字节 → `NoVault`（"还没建过"与"口令错"必须是两件事，
  界面上的下一步动作不同）；否则送密钥 → 验钥匙 → **校验 `user_version`**。
- 版本校验（D7）：`== 1` 继续；其余（含 `< 1`）→ `UnsupportedVersion { found }`。
  ⚠️ `< 1` 是**拒绝**而不是"走迁移"：v1 之前没有版本，非空文件里出现 0 说明它不是本程序的库
  （明文库走不到这一步 —— 它在读第一页时就已经 `NotADatabase`，实测）。改 D7 措辞并记 §10。

### 3. `cipher_memory_security`（§6 的"建议开"）

- 位置：**送密钥之前**。它不读库（pragma handler 直接返回字符串），所以与 D4 不冲突 ——
  而排在前面才有意义：`sqlite3_key` 会复制一份口令进 codec context，那一份才该落在安全分配器上。
  D4 的措辞据此改成"先于任何**读页**的操作"。
- 三项实测（都会让想当然的断言失败）：默认 **0**（关）；设 `OFF` **无法关闭**；读回值是
  `on && executed` 的合取 → 只能断言"我们打开过之后读回来是 1"，**不能**断言"默认是 0"
  （进程级，测试顺序会串）。

### 4. 测试

- `tests/sqlcipher_contract.rs` **增补**（上游行为，换版本时应当失败）：盐 = 文件头前 16 字节且每库不同、
  `cipher_memory_security` 只能开不能关；
- `tests/passphrase_contract.rs`（我们的行为）：类型层拒空、`Debug` 不含口令、`create` 之后
  只认那一把、`create` 不覆盖已有库、`open` 对"没有文件/0 字节"给 `NoVault`、
  `user_version` 2 与 0 都给 `UnsupportedVersion`、`vault_path` 是 `akasha.db`；
- `tests/passphrase_on_disk.rs`（**判据本身**）：用一眼能认出来的标记口令执行完
  `create` + 解锁 + 解锁失败，再**递归扫数据目录里每一个文件** —— 0 命中；
  另有一个**对照目录**放含标记的文件，扫描器必须找到它（否则"0 命中"可能只是扫描器坏了）。
  fixture 留在 `target/store-passphrase/` 供人工复核。

### 5. 判据 ①：无 `keyring` 类依赖 → **永久门禁**（并修复一个先于本 plan 的缺口）

- `deny.toml` 的 `[bans] deny` 加上 `keyring` 与各平台后端（名字取自 `cargo add --dry-run` 的
  真实特性名，不是猜的）；不禁 `security-framework` / `windows` / `dbus`（通用平台绑定）；
- ⚠️ **必须给 cargo-deny 加 `--workspace`**（`src-tauri/justfile` 的 `deny` / `deny-offline`）。
  实测：不加时图根**只有 `akasha`**（workspace root 同时是真实包），于是 `akasha-store` 与
  **它独有的整棵子树**根本不在图里 —— `[bans] deny` 写了 `keyring` 也静默地不生效。
  这一条同时补上一个**先于 0402 的缺口**：`akasha-store` 的 vendored OpenSSL 此前从未被许可证门禁看过；
- 负例（`AGENTS.md` §6）：临时把 `keyring` 加进 `akasha-store` 的依赖 → `just deny-offline`
  必须失败且逐个报出它们，然后撤掉。

## 验收命令

```bash
# ① 全部单测（判据都在里面，具名可查）
just test
# 期望：akasha-store 由 9 增到 22 passed；全 workspace 0 failed

# ② 判据 ①：keyring 类依赖被常驻禁令挡住
just deny-offline
# 期望：bans ok, licenses ok, sources ok —— 且这次的图**包含** crates/* 独有的子树
cd src-tauri && cargo tree -p akasha-store -e normal --prefix none \
  | grep -cE '^(keyring|secret-service|linux-keyutils|dbus-secret-service)$'   # 期望 0

# ③ 判据 ②：数据目录里搜不到标记口令（fixture 是真实文件）
grep -rl 'akasha-passphrase-marker-9d0f4c7b1e5a' src-tauri/target/store-passphrase/vault/
# 期望：无输出（grep 无匹配时退出码 1，不是失败）
# 反面样本 —— 对照目录必须命中 1 个文件，说明这条 grep 真的能搜到东西：
grep -rl 'akasha-passphrase-marker-9d0f4c7b1e5a' src-tauri/target/store-passphrase/control/

# ④ 新建的库从第一刻起是密文（不是"空的所以谁都打得开"）
head -c 16 src-tauri/target/store-passphrase/only-one-passphrase/akasha.db | od -c | head -1
# 期望：不是 "SQLite format 3"（那是明文库的特征）；**文件非空** —— 0 字节就说明口令没被钉住。
# ⚠️ 字节数**不是**定值：`create` 自己写一页（4096，用例内有断言），之后建表会长大（这个 fixture 是 8192）

# ⑤ 口令不会被写入日志：本 crate 连日志设施都没有，
#    也没有 argv / 环境变量 / 配置文件的读取口（口令只能作为 &Passphrase 传进来）
cd src-tauri && cargo tree -p akasha-store -e normal --prefix none \
  | grep -cE '^(tracing|log|clap|argh|structopt|dotenv|envy)$'   # 期望 0

# ⑥ 门禁
just ready
```

## 回滚

`git revert` 整个提交：`akasha-store` 仍**没有消费者**，删掉 `create` / 版本校验 / `Passphrase`
之后 app 行为不变。**唯一不该回滚的是 `--workspace` 那一条** —— 它修的是门禁的盲区，
不是本 plan 的产物（回滚它等于把缺口放回去）。

## 实施记录

**401 没覆盖到的事实（本 plan 的起点）**：

| 要测的 | 实测 |
|---|---|
| 文件不存在 / 0 字节时 `open(任意口令)` | **都能打开**：库 0 字节、KDF 根本没有运行（耗时 **~0.19 ms**，对比真实解锁 **~105 ms**）→ 于是"错误口令打不开"在新建这条路上**不成立** |
| `create`（= 送密钥 + `PRAGMA user_version = 1`）之后 | 文件 **4096 字节**、头部不是 SQLite 魔数、正确口令读回 `user_version = 1`、错一个字节的口令 → `NotADatabase` |
| `PRAGMA cipher_salt` | 32 位十六进制，**逐字节等于文件头前 16 字节**（D2 的"盐存于头部"由此证实）；同一口令建的两个库盐不同、字节不同 |
| 真实解锁耗时 | **105–108 ms**（256,000 次 PBKDF2-HMAC-SHA512，本机）；**错误口令也是 ~104 ms** —— KDF 先运行完才轮到读页失败 |
| `cipher_memory_security` | 默认 **0**；设 `ON` 后读回 `1`；**设 `OFF` 无法关闭**（上游 `sqlcipher_set_mem_security` 的实现是 `if(on)`）；且是**进程级**全局 → §6 的"连接级开关"两半都不对 |
| 明文 sqlite 文件喂给 `open` | `NotADatabase` —— 所以明文库不可能绕过版本校验 |
| `sqlite3_key` 能否排在 `cipher_memory_security` 之后 | 可以，且库照样加密（这就是 §3 敢把顺序反过来写的原因） |

**门禁的缺口（不在原计划里，但必须修复）**：cargo-deny 默认只把 **manifest 指向的那个包**当图根
（workspace root 同时是真实包 `akasha`），于是 `akasha-store` 及其独有子树（`keyring`、
`openssl-src`、`openssl-sys`）**不在图里**：`[bans] deny` 写 `keyring` 也静默不生效。
加 `--workspace` 后图 **580 → 583** 个 crate，负例从 `bans ok` 变成
`bans FAILED … zbus-secret-service-keyring-store ← keyring ← akasha-store`。
**这就解释了 0401 那句"没有新增许可证放行"为何可疑** —— 那时 vendored OpenSSL 从未被看过一眼。

**与 ADR 不符的三处**（都已记入 §10）：D4 的顺序措辞（改为"先于任何读页操作"）、
D7 的 `« < 1 » 走迁移`（改为拒绝）、§6 把 `cipher_memory_security` 写成"连接级开关"。
