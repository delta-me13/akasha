# Plan 0406: 口令的内存防护（`memsafe`）

- **关联**：ROADMAP 阶段 4 ·「口令的内存防护」；[ADR-0002](../../adr/0002-secret-storage.md) D5
  （口令"只以字节缓冲存在于内存"那一条的内存立场）
- **前置**：plan 0402（`Passphrase` 类型与 `open` / `create` 已就位）
- **状态**：已完成（2026-09-12）—— 单测 25/25（`akasha-store`）、`just ready` 6/6

> 本 plan 与实现同一次会话完成：这是**用户当次的直接指令**，优先级高于 ROADMAP 的排期
> （`AGENTS.md` 开头的优先级表）。写在这里只是为了让 0402 之后的读者知道它从哪来。

## 目标

0402 的口令只做到"**进来之后只有这一个形态**"，内存里它仍是普通堆上的一块 `Vec<u8>` ——
进程被 dump、内存被换进 swap、或者一次越界读，它都会泄露。

本步把它放进 `memsafe` 的**一整页受保护内存**，并把上游的四条承诺变成**会失败的测试**：

| 上游承诺 | 我们的判据 |
|---|---|
| `mlock`（不进 swap） | 进程级 `VmLck` 在造口令后必须涨（那一页 4 KiB） |
| Unix 静止态 `PROT_NONE` | smaps 里那一页的权限位是 `---p` |
| Linux `MADV_DONTDUMP`（不进 core dump） | 那一页的 `VmFlags` 含 `dd` |
| Linux `MADV_WIPEONFORK`（fork 不继承） | 那一页的 `VmFlags` 含 `wf` |

## 非目标

- **前端那侧的字符串**不可达（IPC 反序列化的缓冲、JS 里的 `string`）—— 我们能保证的是"进来之后"
- **SQLCipher 自己的密钥材料**不由本步负责：那是 `cipher_memory_security = ON`（plan 0402 §3）
- **防住"已经能在进程内执行代码的攻击者"**：任何一层都做不到，见下面的边界
- 不改 `Passphrase` 的对外语义（空值不可表示、不进日志、任意字节）—— 只换它**存在哪**

## 判据（来自 ROADMAP）

口令在内存里**不被换出、不被 dump、静止时读不到**，且这四条**各有一条会失败的测试**。

## 步骤

### 1. 选型：只取"整页内联"的那条 API

- `memsafe::Secret<[u8; N]>`，**不是** `MemSafe<Vec<u8>>`：后者只保护 `Vec` 的 24 字节头，
  真正的字节还在普通堆上（上游文档把这条叫 "the `MemSafe<Vec<u8>>` pitfall"）。
  `Secret` 的构造只接受 `[u8; N]`，所以"字节一定在页里"是**类型保证**；
- 上限 `MAX_LEN = 256`（页的大小 = 字节数）。超了是 `PassphraseTooLong`，**不截断**；
- 源缓冲由上游**volatile 擦零**（`from_bytes` 的契约）—— 那是普通 `zeroize` 用法最容易漏的一份；
- 依赖只有 `libc` / `winapi`，MIT；`MADV_DONTDUMP` 是 `cfg(target_os = "linux")`，
  所以 macOS / Windows 的**类型检查**照样过（CI 那两个 job 不会失败）。

### 2. `Passphrase` 换成受保护页

```rust
pub struct Passphrase {
    secret: Secret<MAX_LEN>,   // 受保护页（静止时 PROT_NONE / Windows 只读）
    len: usize,                // 实际长度；页里剩下的字节是 0
}
pub const MAX_LEN: usize = 256;
```

- `expose(&mut self) -> Result<Exposed<'_>, _>`：返回值是**提权窗口**（守卫 drop 就降回
  `PROT_NONE`）。`pub(crate)` 不变；`Exposed` 刻意不实现 `Debug` / `AsRef<[u8]>`
  （前者使它无法输出，后者会诱使调用方把 `&[u8]` 存到守卫生命期之外）；
- `open` / `create` 因此收 `&mut Passphrase`：读口令是一次需要**独占**的提权动作；
- 新错误 `MemoryProtection(#[from] memsafe::error::MemoryError)`；
  ⚠️ 上游**没有降级路径**：`mlock` 失败 = 解锁不能进行，不是"静默不锁"。

### 3. 测试：把上游的承诺变成会失败的断言

`tests/passphrase_contract.rs` 新增两条（Linux 专用）：

1. `the_passphrase_page_is_locked_and_excluded_from_core_dumps` —— **建口令前后各取一次
   smaps 快照，多出来的那一页就是它**（不靠猜地址），断言权限位 `---p`、`VmLck` 涨、
   `VmFlags` 含 `dd` 与 `wf`；
2. `proc_self_mem_still_bypasses_page_protections` —— **记录边界**，不是期望属性：
   `/proc/<pid>/mem` 的读走 `FOLL_FORCE`，绕过页保护，那一页照样读得出来。
   它哪天**开始失败**，说明保护变强了 → 回来改这里与 ADR 的措辞。

另有 3 条不依赖平台的行为测试：字节原样回来（含非 UTF-8 与内嵌 NUL）、超一页明确报错、
**错误文本里不含口令**。

### 4. 文档

- ADR-0002：D5 的正文补"口令活在受保护页里"，§7.1 记实测，§8 加一条复核条件
  （上游不再维护 / 出 RUSTSEC 公告 / 改变保护语义），§10 记一行；
- `scope.md` §6 的"密钥来源"行补一句结论（细节仍只在 ADR）；
- plan 0402 是**归档记录**，不改写它，只加一行指向本 plan 的注记。

## 验收命令

```bash
# ① 全部单测：判据都在里面，且**测试自己会打印**实测值（--no-capture 才看得到）
just test
cd src-tauri && cargo nextest run -p akasha-store --no-capture \
  -E 'test(the_passphrase_page) | test(proc_self_mem)'
# 期望：
#   受保护页：0x…-0x…（4096 字节）VmFlags=[mr mw me lo ac wf dd sd]
#   VmLck：0 kB → 4 kB
#   /proc/self/mem 读那一页：Ok(4096)
# 期望：akasha-store 25 passed, 0 failed

# ② 那页在 maps 里确实不可读（手工复核）
just test >/dev/null 2>&1   # 先让 fixture 落盘
grep -c memsafe src-tauri/Cargo.lock                       # 期望 ≥1（依赖真的进来了）
cd src-tauri && cargo tree -p akasha-store -e normal --prefix none | grep -E '^memsafe|^libc|^winapi'
# 期望：memsafe v1.0.2 + libc（Linux 上就这两个；winapi 在 cfg(windows) 下才进图）

# ③ 许可证与来源（新依赖过门禁，且这次图里有 akasha-store 的整棵子树）
just deny-offline                                          # 期望：bans ok, licenses ok, sources ok

# ④ 门禁
just ready
```

## 回滚

`git revert` 本 plan 的提交：`Passphrase` 回到 0402 的 `Vec<u8>` 形态，对外语义不变
（`&mut` 收窄成 `&` 也要一起回——那两处是一件事）。

⚠️ **不该回滚的是"承诺变成测试"这件事**：回滚代码等于回到"口令在普通堆上"，
而那条**没有被任何测试守住**（当时判断的理由是"擦一个缓冲不改变威胁模型"，
本步推翻的正是这个判断 —— 见 §10）。

## 实施记录

**实测值**（Linux，本机）：

| 要测的 | 实测 |
|---|---|
| 静止态的权限 | smaps 里那一页是 `---p`（匿名、无任何权限），**正好 4096 字节** |
| `mlock` | 进程级 `VmLck` **0 kB → 4 kB**（造口令前后各取一次） |
| `VmFlags` | `mr mw me lo ac wf dd sd` —— **`dd`（DONTDUMP）与 `wf`（WIPEONFORK）都在** |
| 源的擦零 | 上游 `Cell::from_bytes` 在拷贝后 `secure_zero` 源缓冲（源码 `cell.rs:264`）；这条**测不了**（源已被移走），只能读源码 + 靠上游契约 |
| `/proc/self/mem` | ⚠️ **读得出来**（`Ok(4096)`，内容就是口令）。`FOLL_FORCE` 绕过页保护 → 这一层防御的是**意外**泄露（越界读、误格式化、core dump、swap、fork），**不是**"能在进程内执行代码的攻击者" |

**另外修正的一个判断**（0402 写的）：当时明确"不做内存擦除"，理由是"库一解锁，私钥与页缓存
同样躺在内存里，单独擦一个输入缓冲不改变威胁模型"。本步推翻的不是那句**事实**，而是它的
**结论**：口令是**唯一能解开整库的东西**，它的生命期比私钥长（跨多次解锁），而 core dump /
swap / fork 是**真实发生过**的泄露路径 —— 这些恰恰是 `mlock` 与 `dd` / `wf` 能防御的，
和"擦一个缓冲"不是一回事。ADR-0002 §10 已记这一行。

**一条没能做的**：`MAX_LEN` 之外**不截断**（明确报错）会让"粘贴了超长口令"变成一条硬错误。
256 字节对用户输入的口令够用（64 位十六进制密钥才 64 字节），但这是**猜的边界**，
不是量出来的 —— 确实遇到再调整，`MAX_LEN` 只有一处定义。
