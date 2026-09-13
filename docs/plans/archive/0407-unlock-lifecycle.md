# Plan 0407: 解锁与锁定的生命周期

- **关联**：ROADMAP 阶段 4 ·「解锁与锁定的生命周期」；
  [ADR-0002](../../adr/0002-secret-storage.md) D4 / D5 / D6 / D13
- **前置**：plan 0403（四类池能读写）· plan 0404（导出要库口令 → 本步的约束）·
  plan 0406（口令的受保护页）
- **状态**：已完成（2026-09-12）—— `just ready` 6/6；`just test` 180 passed；`just test-e2e` 退出码 0

## 目标

四个问题各要一个**有证据的**结论：

1. **谁持有**解好的 `Connection`；
2. **口令从哪来**，以及它经 IPC 进来的那些副本怎么办；
3. **什么时候锁**，锁的时候抹掉什么；
4. **并发**：同时解锁会怎样。

## 先量再写

探针（写完即删）在一个真实的解锁 / 锁定周期上取内存快照。**针是真随机的**
（`/dev/urandom` 24 字节）—— 第一版用"运行期算出来的固定序列"，结果基线里就冒出十几处命中，
分不出"我们的缓冲区"与"别处"。三条测量纪律也写在探针里：**必须也扫 `---p` 的段**
（口令那一页静止态没有读权限，只看 `r` 开头的段正好把它漏掉）、**读缓冲用完就擦**
（否则下一次扫描会把上次的拷贝当成新命中）、**要有正对照**（解锁期间必须扫得到）。

| 时刻 | 口令出现在几处 | 派生密钥出现在几处 | `VmLck` |
|---|---|---|---|
| 解锁前 | 1（测试自己那根针） | — | **0 kB** |
| 解锁中（连接 + 口令页都活着） | **3**（针 / SQLCipher 的 codec 副本 / 受保护页） | **3**（针 / SQLCipher 两处） | **64 kB** |
| 丢掉连接 | 2（针 / 受保护页） | 1（针） | **4 kB** |
| 丢掉口令 | 1（针） | 1（针） | **0 kB** |

三件事因此从"上游说"变成"实测"：

- **口令与派生密钥在连接 drop 之后一处不剩**（剩下的那个命中就是运行扫描的测试自己那份）。
  派生密钥是用 `openssl` CLI 按 SQLCipher 4 的默认参数算出来的：PBKDF2-HMAC-SHA512、
  256000 轮、32 字节，盐 = 库文件前 16 字节 —— 这与 ADR-0002 §7 的磁盘事实一致；
- **那条 wipe 是 `cipher_memory_security = ON` 真的在做**：它把 SQLite 的分配器换成
  `sqlcipher_mem_malloc / mem_free`，**每次分配 `mlock`、每次释放先擦零再 `munlock`**
  （vendored `sqlite3.c` 里那两段可逐行核对）；
- `VmLck` 的 64 → 4 → 0 正是这个机制的轨迹：解锁期间锁住的不只是我们那一页（4 kB），
  还有 SQLCipher 自己锁的 60 kB。

## 非目标

- **口令的输入 UI**（设计稿未定，`AGENTS.md` §4.0）：本步只到"命令能收、能锁、能观测"
- **空闲超时自动锁**：没有 UI 也没有配置项，超时会变成"偶尔需要重新输入口令"这类难以解释的行为；
  它要防的场景（无人值守时机器被他人操作）属于**锁屏**，交给 OS 更合适
- **忘记口令的恢复路径**：ADR-0002 D5 已裁定不可恢复，这里不重开
- **口令走 raw body 的那条更紧的通道**：实测代价是"前端必须手写一行直接调用 `invoke`"
  （生成的包装函数会把参数包成对象，而 raw body 必须是整个 payload），
  那与 `AGENTS.md` §0 绝对禁止 #1 冲突 —— 为**少一份不可达的副本**去破一条架构规则，
  不划算。**记录为后续收紧路径**，等有 UI 时连同 §5 的例外一起改

## 决定

### 1. 谁持有：`tauri::State<Vault>`，不是专用线程

```rust
pub struct Vault { unlocked: Mutex<Option<Unlocked>> }
struct Unlocked { conn: Connection, passphrase: Passphrase }
```

- **`Mutex` 而不是专用线程**：存储层是同步的、快的（除了 KDF），专用线程要多一套
  actor 协议却没有可量的收益；而 `Connection` 是 `Send` 不是 `Sync`，
  `Mutex` 正好给出"独占一次用"这件事需要的 `&mut`（`Passphrase::expose` 也要 `&mut`）；
- **口令留在 `Unlocked` 里**：这是 plan 0404 留下的硬约束 —— 导出的"独立口令"检查
  要在解锁窗口内拿得到库口令，否则用户得为一次导出重新输一遍。代价是解锁期间
  多 4 kB 的 `VmLck`（实测），锁定时归零；
- **KDF 不挡 UI**：`vault_unlock` 是 `async` 的，`open` / `create` 放
  `tauri::async_runtime::spawn_blocking`（实测解锁约 105 ms，ADR-0002 §7.1）。
  `Connection` 与 `Passphrase` 都是 `Send`（`memsafe::Secret` 对 `NoAccess` 态实现了 `Send`），
  所以整份 `Unlocked` 能跨线程回来。

### 2. 口令从哪来：一个只有一条出路的 newtype

命令参数是 `PassphraseInput`：`Deserialize` + `specta::Type`，**没有 `Debug` / `Clone` /
`Serialize`**（想把它打进日志是**编译错误**，与 `Passphrase` 同一手法）。
它唯一的出路是 `into_bytes()` → `Passphrase::new()` —— 后者由 `memsafe` **擦零源缓冲**。

| 副本在哪 | 谁擦得掉 | 依据 |
|---|---|---|
| JS 里那个 `string` | 前端的事（不可达） | plan 0406 非目标 |
| tauri 的请求体缓冲 + `serde_json::Value` | **擦不掉**（我们没有 `&mut`） | 生命周期 = 这一次调用，之后 free **不擦** |
| 反序列化出来的那个 `String` | **擦得掉**：`into_bytes()` 把缓冲搬进 `Passphrase::new` | 实测：解锁中 3 处 → 丢口令后 1 处 |
| 受保护页里的那一份 | drop 时 `munmap`（实测 `VmLck` 归零、`---p` 段数回落） | plan 0406 |
| SQLCipher 内部的副本 | drop 连接时擦零（实测：派生密钥 3 处 → 1 处） | `cipher_memory_security` |

**不可达的那两份照实记**（写进 ADR-0002 的边界表，与 `/proc/self/mem` 那条并列）：
它们不是"遗漏处置"，是"这一层没有接口"；能收紧的方向是 raw body，代价见「非目标」。

### 3. 什么时候锁：只有显式锁

| 触发 | 锁吗 | 为什么 |
|---|---|---|
| 用户 / 前端显式 `vault_lock` | **锁** | 唯一有条目的入口 |
| **关窗口** | **不锁** | 默认语义是**收托盘**（plan 0302）：进程、会话、终端缓冲全都留着 —— 库也一样 |
| **进程退出** | **不锁** | 进程退出后，解好的连接与锁住的那一页一起消失（实测 `VmLck` 归零）。写一个 exit hook **并不产生实际效果** |
| 空闲超时 | 不锁 | 见「非目标」 |

"锁"抹掉的是**口令与连接**，不是会话：终端里正在运行的作业与这个库无关。

### 4. 并发：一个 `Mutex` 就够了

单实例（plan 0304）保证只有一个进程；进程内并发调用 `vault_unlock` 时，
第二个看到 `AlreadyUnlocked`（**不替换、不静默成功**）。检查是 check-then-act：
并发时两边都会算一次 KDF（105 ms）而只有一个成功 —— 代价照实记，不做加锁排队。

## 步骤

1. `vault.rs`：`Vault` / `Unlocked` / `PassphraseInput` / `VaultError` 的新变体
   （`AlreadyUnlocked`、`NotLockable`、`WrongPassphrase`、`UnlockFailed`、`Internal`）；
   `StoreError` → `VaultError` 写成**穷尽 `match`**（存储层加变体时这里编译不过）；
   `VaultState` → 建还是开，抽成纯函数并配单测（`Missing` / `Empty` → `create`，`Present` → `open`）。
2. 三个命令：`vault_unlock`（解锁 → 读一次四类池 → 返回各池行数）、`vault_lock`、
   `vault_status`（加一个 `unlocked`）。登记进 `bindings.rs`，运行 `just gen-types`。
3. 日志：`vault unlocked` / `vault locked` / `vault unlock failed`（英文、消息是事件名、
   数字进字段，`docs/logging.md`）。
4. `akasha-store` 的 `tests/unlock_lifecycle.rs`：把上面那张表变成**三条会失败的测试**
   （`VmLck` 轨迹 + 内存扫描 + 派生密钥）。扫描器只在 Linux 上运行；
   `openssl` CLI 不在时**显式跳过并写明原因**（不静默通过）。
5. E2E（`src-tauri/tests/vault_unlock.rs`）：在真实 app 上走一遍
   `vault_status` → 造库 → `vault_unlock` → 断言行数 → `vault_lock` →
   断言**app 进程自己的 `VmLck` 回落到解锁前**（读 `/proc/<app pid>/status`），
   再加一条"锁上之后错误口令解不开"——锁必须是真的。
6. 文档：ADR-0002 补 D4/D13 的落地记录与 §10 修订行；`scope.md` §6 的库那一节；
   ROADMAP 勾选；STATUS 覆盖写。

## 验收命令

```bash
cd src-tauri && cargo test -p akasha-store --test unlock_lifecycle -- --nocapture
# 预期：3 passed；打印「0 → 64 → 4 → 0 kB」的 VmLck 轨迹、
#      口令 1 → 3 → 2 → 1（解锁中必须 > 基线，否则说明扫描器没在工作）、
#      派生密钥 0 → 3 → 1 → 1（最后那个 1 是测试自己算出来的那份）

just gen-types && git diff --stat src/ipc/bindings.ts   # 预期：新增 vaultUnlock / vaultLock 与字段
just test-e2e                                            # 预期：退出码 0，含 vault_unlock 一条
just test                                                # 预期：全部通过（akasha-store 增加 3 条）
just ready                                               # 预期：6/6
```

## 回滚

`git revert` 两个提交即可：app 侧的三个命令与状态是新增的，`vault_status` 只多一个字段；
存储侧的测试文件删掉不影响任何产品代码（本步**没有改** `akasha-store` 的库函数）。

## 实施记录

### 验收命令的实际输出

| 命令 | 实际结果 |
|---|---|
| `cargo test -p akasha-store --test unlock_lifecycle -- --nocapture` | **4 passed**：`VmLck` 轨迹 `0 → 4 → 152 → 4 → 0 kB`；口令 `1 → 2 → 2 → 1` 处（多出来的那处是 `---p` 页）；派生密钥 `3 → 1` 处（剩下的 1 处是测试自己算的）；重新 `open` 也回到 `4 → 172 → 4 kB` |
| `just test-e2e` | 退出码 **0**（新增 `vault_unlock` 一条）：`vault_status` = `missing`/`unlocked:false` → 造库 → 解锁返回 `{"keys":1,"hosts":1,"serials":1,"forwards":1}` → app 进程 `VmLck` **0 → 192 → 0 kB** → 错误口令被拒且仍然锁着 → 锁上之后还能再解开 |
| `just test` | **180 passed**（原 171） |
| `just ready` | **6/6** |
| `just gen-types` | 生成物多出 `vaultUnlock` / `vaultLock` / `PassphraseInput` / `VaultContents` / `VaultError` 的五个变体与 `vaultStatus.unlocked` |

### 探针与最终测试不一致的两处（照实记，两处都是"测量仪器的错"）

| | 探针（先量） | 最终测试 | 为什么不一样 |
|---|---|---|---|
| 解锁中的 `VmLck` | 64 kB | **152 kB** | 探针自己分配了一块 64 MiB 的读缓冲，而且那块缓冲就在它要扫的地址空间里 —— 数字里混进了它 |
| 解锁中口令的处数 | 3 处 | **2 处** | 探针除了那根针还留着一份 `baseline = pass.clone()`。**第三处不是 SQLCipher 的，是探针自己的** —— 这条差别推翻了一个想当然（ADR-0002 §7.5 第 2 条） |

以此为准的是测试那一份：它复用一块 1 MiB 的缓冲（读完即擦）、没有多余的拷贝、并且带**正对照**。
两条都进了 STATUS 的已知问题（#90 / #91）。

### 中途改过的决定

- **口令的副本清单少了一行**：原以为 SQLCipher 的 codec 里也留着一份口令 —— 量下来它只有派生密钥。
- **`VaultContents` 的计数用 `u32`**：生成器**拒绝**把 `usize` 导出成 TS（BigInt 精度，与问题 #32 同源）；
  转换写成 checked，不写 `as`。
- **加了一条判据**：`open` 那条路（不只是 `create`）也要把内存还回来 —— 重新打开才是真实使用里那条路。
- **为让 app 侧不必依赖某个 `rusqlite` 版本**，`akasha-store` 把 `Connection` 再导出一遍：
  `libsqlite3-sys` 带 `links`，两个版本连编都编不过（`AGENTS.md` §3.1 的依赖方向没变）。

### 没有做的事

- **没接四类池的 IPC 命令**（库层能读能写，但还没有能写的前端命令）：E2E 造数据直接调
  `akasha-store` 的库函数，落到 `vault_status` 报出来的那个路径上。
- **没做 raw body 的口令通道**、**没做空闲超时**：理由在「非目标」里。
- **没做解锁 UI**：没有设计稿就没有验收标准（`AGENTS.md` §4.0）。
