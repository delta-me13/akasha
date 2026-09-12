# Plan 0405: 可搬迁性验证

- **关联**：ROADMAP 阶段 4 ·「可搬迁性验证」（展开见 [`../portable.md`](../portable.md)）
- **前置**：plan 0403（四套池的 CRUD —— 没有数据就验不了"数据还在"）、
  plan 0407（`vault_unlock` —— "数据**可用**"要有命令能读出来）
- **状态**：进行中

## 目标

把 [`../portable.md`](../portable.md) §5 的五步**做成一条能跑的配方**，并补上 §4 第 3 条
（**便携模式下不可写 → 启动即报错**）—— 后者今天**没有实现**，实测见下。

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
just ready        # 6/6。config 的新单测在这里（纯函数 + 探针文件）
just test-e2e     # 全绿；自起分支的第三段就是可搬迁性
just test         # workspace 单测（新增单测计入）
```

预期：可搬迁性那一段打印 A/B 两个位置与解锁读回的行数；拒绝那条打印退出码与错误消息。

> 步骤 4 那条配方落地后，它自己也是一条入口（`just --list` 可见、只跑可搬迁性那一段）。
> 本文件的「步骤」与「验收命令」按 §8 的规矩**在落地时就地更新**。

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

（边做边追加实际输出）
