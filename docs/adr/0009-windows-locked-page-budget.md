# ADR-0009：Windows 上给受保护页留足可锁的页额度

- **状态**：**实现中**（2026-09-20）
- **日期**：2026-09-20
- **决策者**：cyrene
- **影响范围**：`src-tauri/src/store/protected.rs`（一处 Windows-only 的 `unsafe` 与一条进程级调优）、Windows 上进程的工作集下限、`AGENTS.md` §3.4 里 `unsafe` 放行理由的措辞
- **取代**：补上 [ADR-0002](./0002-secret-storage.md) 在 Windows 上缺的那一步 —— 它的 D1（`cipher_memory_security`）与 D13（`memsafe` 受保护页）在 Windows 上**共用同一个** `VirtualLock` 额度，而那边默认只放行 50 页。**ADR-0002 保持已定案、一字不改**：数据文件格式、KDF、口令路径、导出容器都不变，本条只加一步进程级调优。
- **关联**：问题 #167 / #168；[plan 0110](../plans/0110-windows-locked-page-budget.md)

---

## 1. 背景

两个机制各自都要把内存页锁在物理内存里：

| 机制 | 管什么 | Windows 上的实现 |
|---|---|---|
| `memsafe` 的受保护页（ADR-0002 D13） | 口令（256 字节）与私钥（16 KiB） | `VirtualAlloc` + `VirtualLock`（`memsafe-1.0.2/src/ffi/win.rs`） |
| SQLCipher 的 `cipher_memory_security = ON`（ADR-0002 D1） | 每个连接的每一次内部分配 | `sqlcipher_malloc` → `sqlcipher_mlock` → `VirtualLock`（`libsqlite3-sys` 的 `sqlcipher/sqlite3.c`；失败**只记日志**） |

在 Linux 与 macOS 上这两条各自去要自己的额度（`RLIMIT_MEMLOCK` / 系统策略），互不相关。**Windows 上它们争的是同一份额度**，而那份额度就是**进程的最小工作集**。

本机（Windows）实测，一个只做 `VirtualLock` 的探针进程：

| 每次锁的字节数 | 能锁住几次 | 折合 |
|---|---|---|
| 4 KiB | 44 | 176 KiB |
| 16 KiB | 11 | 176 KiB |
| 32 KiB | 5 | 160 KiB |

三种形态都停在 176 KiB（默认最小值 50 页 = 200 KiB，减去文档说的“一点开销”）。一个 16 KiB 的私钥页因此只放得下 11 个；而 SQLCipher 那边一旦先占住若干页，后面那次 `VirtualLock` 就会拿到 `ERROR_WORKING_SET_QUOTA`（`Os { code: 1453 }`）。

**后果（问题 #167）**：`just ready` 在本机 Windows 上停在 `test` —— `tests/export_contract.rs` 的 `an_encrypted_export_restores_into_another_directory` 在 `keys::private_key` 处失败。它是**竞态**：同一次完整运行里 `the_export_file_is_itself_a_vault` 也红、单独执行又绿。在用例内插桩量到的余量（单位 = 一个 16 KiB 的私钥页）说明这一点：

| 时刻 | 还能锁住几个 16 KiB 页 |
|---|---|
| 进程刚启动 | 11 |
| `populated()` 之后 | 3 |
| `restore` 之后 | 4 |
| `open(restored)` 之后 | 0 |

⚠️ 这不是测试脚手架的问题：**App 里“库解锁着 + 读一把私钥”走的是同一条路**。

## 2. 决策

**在 Windows 上，第一次构造受保护页之前，把本进程的最小 / 最大工作集抬到 16 MiB / 256 MiB，只调一次；失败只记一条 `warn`，不阻断任何操作。**

- **落点**：`src-tauri/src/store/protected.rs` 的 `ensure_working_set()`，由 `Protected::new` 调用（那是唯一构造受保护页的地方）。
- **只在 Windows 上生效**：`#[cfg(windows)]`；其余平台是一个空函数。
- **`unsafe` 的形状**：一处 `unsafe extern "system"` 声明加一处调用，`#[allow(unsafe_code)]` 单点放开，`// SAFETY:` 写明三条前置（伪句柄、两个尺寸落在文档允许的范围里、只改本进程的上下限）。
- **不引入 `windows-sys`**：只用到两个函数，而新增一个依赖要走 `just deny` 的许可证与来源门禁。

## 3. 理由

1. **这是上游文档给出的处置本身**，不是绕路（引文见 §4）。
2. **两个保护都留着**：SQLCipher 的 `cipher_memory_security` 与 `memsafe` 的受保护页都不动，只是让进程拿得到足够的额度 —— 不削弱任何一条安全属性。
3. **落点保证顺序**：受保护页一定早于任何连接被打开（`create` / `open` / `to_encrypted` 都要先有一个 `Passphrase`），所以抬额度必然发生在 SQLCipher 开始上锁之前。
4. **代价有界**：最小值是“内存管理器尽量保住的下限”，而本应用的常态驻留远高于 16 MiB；最大值只受“必须不小于最小值”这一条约束，取 256 MiB 是为了不低到让内存管理器开始裁剪本进程。

## 4. 依据（上游文档原文）

`VirtualLock` 的 Remarks（2026-09-20 取自 learn.microsoft.com）：

> Each version of Windows has a limit on the maximum number of pages a process can lock.
> This limit is intentionally small to avoid severe performance degradation.
> Applications that need to lock larger numbers of pages must first call the
> `SetProcessWorkingSetSize` function to increase their minimum and maximum working set sizes.
> The maximum number of pages that a process can lock is equal to the number of pages in its
> minimum working set minus a small overhead.

`SetProcessWorkingSetSize` 的参数约束（同一来源）：

> `dwMinimumWorkingSetSize` … must be greater than zero but less than or equal to the maximum working set size. The default size is 50 pages …
> `dwMaximumWorkingSetSize` … must be greater than or equal to 13 pages …, and less than the system-wide maximum (number of available pages minus 512 pages).

本机对参数的实测（每组参数各用一个新进程：锁页本身会消耗额度）：

| 最小值 | 最大值 | 结果 |
|---|---|---|
| 1 MiB | 4 GiB | 能锁 62 个 16 KiB 页 |
| 8 MiB | 4 GiB | 510 个 |
| 16 MiB | 4 GiB | 1022 个 |
| 16 MiB | 最大值取上限 | 1022 个（把最大值给到上限**不会**取消最小值） |
| 0 | 0 | 调用失败（`ERROR_INVALID_PARAMETER`） |

修好之后在同一条用例上量到的需求侧：`open(restored)` 之后还剩 1006 个 16 KiB 页可锁，也就是 SQLCipher 与我们的页**合起来**只占约 288 KiB —— 16 MiB 是它的五十多倍，留的是并发连接（导出 / 还原会同时开三个库）与页缓存随库增长那部分余量。

## 5. 被否掉的替代方案

| 方案 | 为什么不选 |
|---|---|
| 只在 Windows 上**不开** `cipher_memory_security` | 不需要 `unsafe`，但丢掉 SQLCipher 自己那份密钥副本的锁定与擦零 —— 那是 ADR-0002 D1 要的东西，而本条的目的正是两条都留住 |
| 让 `Protected::new` 在 `VirtualLock` 失败时降级成不锁 | ADR-0002 D13 已经写明：锁页失败意味着“不能进行”，而不是“悄悄不锁” |
| 缩小 `MAX_PEM_LEN`（16 KiB 的私钥页） | 那是功能上的倒退（私钥上限变小），而问题在额度而不在页的大小 |
| 不修，记成 Windows 上的已知限制 | 它会让 App 在 Windows 上随机失败，而 CI 抓不到（完整门禁只在 Linux 执行） |
| 换用 `SetProcessWorkingSetSizeEx` 加硬下限标志 | 同一族 API 的另一个入口，多一个标志位而拿不到额外的东西 |

## 6. 代价与已知不足

- **进程在 Windows 上多了一条 16 MiB 的驻留下限**：它是下限而不是占用（本应用的常态驻留远高于它），但在内存极紧的机器上会减少可回收的余量。取 16 MiB 而不是更大，正是因为这一点。
- **`unsafe` 从一处变成两处**：`AGENTS.md` §3.4 里“只有存储模块允许出现它”这句话本身不变（两处都在 `src-tauri/src/store/` 内），但它后面那句“为什么”要跟着改。
- **`Protected::new` 多了一次 `Once` 检查**：一次原子读，代价可忽略。
- **它不覆盖 macOS / Linux**：那边的额度是另一套机制，本条一个字节都没改。

## 7. 复审条件

- 引入 `windows-sys`（或某个现成的安全包装）之后，把这两行手写的 `extern` 换掉。
- SQLCipher 改变 `cipher_memory_security` 的分配行为（例如改成只锁密钥材料）→ 16 MiB 可以调小。
- 应用在 Windows 上的常态驻留接近 256 MiB → 最大值需要重新取值。
- Windows 上仍出现锁页失败的读数 → 先按 §4 重新量一遍额度，再决定是否抬高最小值。

## 修订记录

- 2026-09-20：写下（实现中）。依据是本机的实测读数与上游文档（§4），落地在 plan 0110；同一次改动里还修掉了问题 #168 的另两条 Windows 专有失败（plan 0111）。



