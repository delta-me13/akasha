# Plan 0405: 可搬迁性验证

- **关联**：ROADMAP 阶段 4 ·「可搬迁性验证」（展开见 [`../../portable.md`](../../portable.md)）
- **前置**：plan 0403（四套池的 CRUD —— 没有数据就验不了"数据还在"）、
  plan 0407（`vault_unlock` —— "数据**可用**"要有命令能读出来）
- **状态**：已完成（2026-09-12）—— `just ready` 6/6；`just test` 186 passed；
  `just test-e2e` 退出码 0（自起分支第三段就是这条）

## 目标

把 [`../../portable.md`](../../portable.md) §5 的五步**做成一条能跑的配方**，并补上 §4 第 3 条
（**便携模式下不可写 → 启动即报错**）—— 后者落地前**没有实现**（实测见下），现在有了。

判据是 ROADMAP 那一句：搬走整个文件夹后重启，**原有主机 / 密钥 / 规则都在**。
"只验证能开"不算过。

## 非目标

- 不做 webview 及其依赖栈的环境重定向（`scope.md` §1.1 已豁免）
- 不做 `strace` 类全量写入审计（判据已简化为"搬走文件夹后还能开、数据还在"）
- 不跨机器（同一台机器换目录是同一回事；另外两个平台交给 CI 矩阵）
- 不做"库里不存绝对路径"的静态检查（那是 plan 0403 的判据；这里验的是它的**后果**）

## 先量再写（2026-09-12 实测）

| 量什么 | 结果 | 结论 |
|---|---|---|
| 便携目录**可写**时 | `config.json` 真的被读到（`lifecycle` probe 报 `close_behavior=exit`），库也落在它里面 | "数据目录从 bin 推导"已经对，缺的只是搬家那一步的验证 |
| 便携目录 `chmod 500` 时（**今天**） | app **照常启动**，日志只有一条 `config not found path=<那个目录>/config.json` | §4 第 3 条**没实现**；而且那条日志还把"写不了"说成了"没有配置文件" —— 正是要消掉的那个静默 |
| 把 debug 二进制**复制**到临时目录再起 | 起来了；Victauri 的端口/令牌照常出现在 `<temp>/victauri/<pid>/` | 五步能自动化 —— 被测的是"bin 在哪"，不是"构建目录在哪" |
| `vault_unlock` 这类 `invoke_command` 走哪条路 | 走 webview 里的 JS bridge：Vite 不在时返回 `bridge not responding` | 这一段必须在 Vite 起着的环境里跑（`just test-e2e` 本来就有这条件） |
| `app_state`（probe） | **不过 bridge**，Vite 不在也能读 | "app 挑了哪个数据目录"用 probe 观察，不用 grep 日志反推 |

## 四个决定

1. **被测对象是"复制出来的那一份 bin"**。`just test-e2e` 用的 app 跑在 `target/debug/` 里，
   搬不动它（那是构建目录）。所以这一段**自己起 app**：把 `CARGO_BIN_EXE_akasha`
   复制进临时布局 `A/{akasha, akasha-data/}`，在 A 起一次、搬成 B、在 B 再起一次。
   ⚠️ 代价是**不能与别的 akasha 同时跑** —— 单实例（plan 0304）会让第二份自己退掉，
   于是"app 起不来"看起来像可搬迁性坏了。所以配方先查一遍并说清原因。
2. **便携目录不可写 → 一条 `error` + 退出码 2，不弹窗**。判"可写"用**真的写一个探针文件**
   （随后删掉），而不是看 mode 位：mode 位看不出 ACL、只读挂载与 squashfs。
   为什么不弹窗：GUI 报错的形态属于**真实 UI 阶段**（要引 dialog 插件与权限），
   而这一条的机器可查形态是"退出码 ≠ 0 + 那条 error"；**不静默退回 OS 目录**才是它的要害。
3. **判据分两半，写在同一条用例里**：app 侧（probe 报的路径 + `vault_unlock` 读回四套池行数）
   与库侧（搬完用同一口令打开，四行**内容**逐项一致）。只验 app 侧不够 ——
   库侧才是"没有静默丢内容"。
4. **不进 `just ready`**：它要 GUI + Vite，与 `just test-e2e` 同一类。形态是一条新配方
   `portable`（`just --list` 里可见），由 `just test-e2e` 的自起分支**再跑一遍** ——
   E2E 入口仍然只有一处（`AGENTS.md` §12），CI 三个平台因此顺带都覆盖到。
   复用别人的 app 那一支显式跳过并说明原因。

## 步骤（每步都能独立验证）

1. `config.rs`：把「bin 同目录那个便携目录」抽成 `portable_dir()`（`data_dir()` 复用它，
   避免两处写同一表达式）；新增 `require_writable()` 与 `NotWritable`；单测含**正对照**
   （同一个目录可写时必须通过）。
2. `lib.rs`：`.setup()` 里检查便携目录可写性 —— 不可写就 `error!` 后 `std::process::exit(2)`。
   放 `.setup()` 是因为日志插件在那时已就绪，而窗口/托盘还没建（拒绝得要早、要说得出来）。
3. `tests/portable.rs` 两条用例：
   * `data_survives_the_move`：A 起（probe=exit、`vault_status` 在 A、解锁建库）→ 停 →
     用库函数灌四套池各一行 → `mv` A→B → B 起（probe=exit、路径在 B、解锁读回 1/1/1/1）→ 停；
   * `an_unwritable_dir_refuses_to_start`：布局 `chmod 500` → 退出码 2 + stdout 里有那条 error。
     先做**正对照**（这台机器上真的写不进去）—— 写不进去才算数，否则显式跳过。
4. `src-tauri/justfile`：新增 `portable` 配方（Vite + `cargo test --test portable`）；
   新增 `E2E_SELF_APP` 清单并纳入 guard（这个目标**自己起 app**，不能由 `run_targets` 跑）；
   `test-e2e` 自起分支的第三段调用它。
5. 根 `justfile` 转发；`docs/just.md` §2 登记。

## 验收命令

```bash
just ready      # 6/6（config 的新单测在这里：纯函数 + 探针文件）
just portable   # 可搬迁性：搬家五步 + 不可写拒绝（Vite 由配方自己保证）
just test-e2e   # 全绿；自起分支的第三段调用的就是上一条
just test       # workspace 单测（新增单测计入）
```

预期：`portable` 打印 A/B 两个位置与解锁读回的行数，三条用例全 ok；
拒绝那条打印退出码 `2` 与那条 error。

## 判据

| # | 判据 | 机器可查处 |
|---|---|---|
| 1 | 便携目录存在时，数据**落在 bin 同目录** | `tests/portable.rs`：A 起的 app 报的路径在 A 里 |
| 2 | 搬走后 app 仍能开，且**认的是新位置** | B 起的 app 报的路径在 B 里；`lifecycle` probe 读到跟着走的 `config.json` |
| 3 | 原有**主机/密钥/规则都在** | B 起的 app `vault_unlock` 读回 4 套池各 1 行；库侧逐项比对内容 |
| 4 | 便携目录不可写时**启动即报错**，不静默退回 | 退出码 2 + `portable data dir not writable` |
| 5 | 这些结论不靠日志反推 | 前三条读的是 probe 与 IPC 返回值；日志只在第 4 条（那正是被验的行为） |

## 回滚

纯增量：删 `tests/portable.rs`、`portable` 配方与 `.setup()` 里那一段即可回到今天
（`config.rs` 的 `require_writable()` 与 `portable_dir()` 无人调用，可一并删）。

## 实施记录

**落点**

| 文件 | 改了什么 |
|---|---|
| `src-tauri/src/config.rs` | 抽出 `portable_dir()`（`data_dir()` 复用它）；新增 `portable_data_dir()`、`require_writable()`（写探针文件）、`NotWritable`、`EXIT_NOT_WRITABLE = 2`；3 条单测 |
| `src-tauri/src/lib.rs` | `.setup()` 开头：便携目录不可写 → `error!` + `exit(2)`（在窗口与托盘之前） |
| `src-tauri/tests/portable.rs` | 3 条用例：搬家、不可写拒绝、**没有便携目录时不许拒绝**（正对照的另一半） |
| `src-tauri/justfile` | `portable` 配方；`E2E_SELF_APP` 清单 + guard；`test-e2e` 自起分支的第三段 |
| 根 `justfile` / `docs/just.md` §2 | 转发 + 登记 |

**实测输出（2026-09-12）**

```text
A 的 vault_status = {"path":"…/akasha-portable-307-a/akasha-data/akasha.db","state":"missing","unlocked":false}
B 的 vault_status = {"path":"…/akasha-portable-307-b/akasha-data/akasha.db","state":"present","unlocked":false}
B 解锁读回四套池 = {"forwards":1,"hosts":1,"keys":1,"serials":1}
test result: ok. 3 passed; 0 failed
```

拒绝那条：

```text
退出码 ExitStatus(unix_wait_status(512))     # = 2
[akasha_lib][ERROR] portable data dir not writable err=not writable: 权限不够 (os error 13) path=…/akasha-data
```

**写用例时撞出来的两件事**（都不是产品 bug，都是"就绪"这个概念）

1. **发现目录出现 ≠ app 就绪**：Victauri 的插件 setup 比 app 自己的 `.setup()` 早，
   所以刚连上时 `lifecycle` probe 还是 `{"initialized": false}`（`record()` 在 `.setup()` 里，
   而拒绝启动的检查在它前面）。第一版用例把那个中间态当成了答案，红在
   `probe 里没有 close_behavior：{"initialized":false}`。改成**等那个字段自己出现**。
   （同一条理由还有第二层：`invoke_command` 走 webview bridge，是最后才好的一个，
   所以 `vault_status` 也改成"等它真的答一次"。）
2. **库是 app 建的，不是我们建的**：第一版的 `seed()` 用 `create()` → `VaultExists`。
   顺序是有意的（第 1 步要验的正是"app 自己把库建在便携目录里"），所以 `seed()` 改用 `open()`。
3. **`chmod 500` 那个 fixture 在单测里站不住**：`no-println` 规则豁免的是 `**/tests/**`，
   **不豁免** `src/` 里 `#[cfg(test)]` 模块里的 `eprintln!`（那条"跳过并说明原因"因此红在 lint 上）。
   改成**结构性**造法：把探针路径指到一个普通文件底下（`ENOTDIR`，root 也绕不过去）——
   于是它不依赖权限、平台或文件系统。`chmod` 那条留在 `tests/portable.rs`：
   那里要的是 **app 真的看见一个不可写目录**，而它本来就有"跳过并打印原因"的豁免。

**没有做的事**

- **没有 GUI 报错面**：拒绝启动是"日志 + 退出码 2"，不是弹窗。理由见决定 2（要引 dialog 插件与权限），
  边界记进了 `portable.md` §4 与 `STATUS.md` 的待验证。
- **没有在本机验 Windows / macOS**：`chmod` 那条正对照在 Windows 上会显式跳过（并打印原因）；
  另两个平台的覆盖交给 CI 矩阵 —— 但 CI 至今没跑过（无 remote），照实记着。
- **复用别人 app 的那一支不跑这一段**（单实例会让它起的第二份自己退掉），
  `test-e2e` 会打印原因；想验就单独跑 `portable`。
