# Plan 0108: Windows 目标的类型检查真的通过（问题 #149）

- **关联**：ROADMAP 阶段 1 ·「Windows 目标的类型检查真的通过」
- **前置**：plan 0801（平台边界：串口的 `libudev` 只在 Linux 上链）/ plan 0102（CI 矩阵要它才可能绿）
- **状态**：进行中
- **影响面**：`src-tauri/crates/akasha-pty/`（`Cargo.toml` + `teardown.rs` + `watchdog.rs`）、
  `ROADMAP.md`、`docs/plans/README.md`、`docs/STATUS.md`

## 目标

`akasha-pty` 用了 Windows 上不存在的 `rustix::process` 且没有 `cfg` 守卫，
于是 `cargo check --target x86_64-pc-windows-msvc` 在它这里就红（3 个 E0432 / E0433），
依赖它的 `akasha-serial` 跟着红。CI 的 `checks-other` 执行的就是
`cargo check --workspace --all-targets`，所以那两个平台**至今不可能通过**。

判据（来自 ROADMAP）：**Windows 目标上的类型检查退出码 0**。

## 非目标

- **不实现 Windows 上的会话回收**（Job Object 那一套）：本机没有 Windows 主机，
  写出来的代码无法按 `AGENTS.md` §7 实测。这是**另一条工作项**，理由与留下的缺口见本文末。
- **不在本地核对 `akasha-store` / `akasha-ssh` / `akasha-bw` / `akasha`**：它们带 C 依赖
  （vendored OpenSSL、`ring`），在非 Windows 主机上为 Windows 目标构建不出来。
  这一层只能由 CI 的 Windows 格子给出结论（见「验收命令」）。
- **不改 Linux 或 macOS 上的任何行为**：`teardown` 与 `watchdog` 的 unix 路径逐字保留。

## 前置检查

```bash
cd src-tauri && cargo check -p akasha-core --target x86_64-pc-windows-msvc   # 改之前：退出码 0
cd src-tauri && cargo check -p akasha-pty  --target x86_64-pc-windows-msvc   # 改之前：3 个错误
cd src-tauri && cargo check -p akasha-serial --target x86_64-pc-windows-msvc # 改之前：同一个错误
```

改之前的错误形状（问题 #149 的原文）：

```text
error[E0432]: unresolved import `rustix::process`
  --> crates/akasha-pty/src/teardown.rs:41:5
error[E0433]: cannot find `process` in `rustix`
  --> crates/akasha-pty/src/teardown.rs:105:13
error[E0433]: cannot find `process` in `rustix`
  --> crates/akasha-pty/src/watchdog.rs:115:13
```

上游把 `process` 模块限定在 `#[cfg(not(windows))]`（`rustix-1.1.4/src/lib.rs:266`），
而这三处都没有那条守卫。

## 步骤

1. `teardown.rs`：`rustix::process` 的导入、`sigkill`、`kill_group` 三处限定在 `cfg(unix)`；
   `kill_session` 的非 unix 分支保持"什么都不做"—— 它本来就是那个口径（该模块的平台差异表）。
2. `watchdog.rs`：`detach()` 分成两条平台路径 —— unix 走 `setsid`；Windows 上"脱钩"由 spawn
   那一侧完成（`CREATE_NO_WINDOW`：没有控制台，`^C` 就送不到它手上），所以那里是空操作。
3. `Cargo.toml`：`rustix` 从 `[dependencies]` 挪进 `[target.'cfg(unix)'.dependencies]`
   —— Windows 上不该留一个用不到的依赖。
4. 两个模块的头部注释按新事实更新：`teardown` 的平台表里"Windows 编译不过"这一条不再成立，
   取而代之的是"Windows 上编译得过、但收会话仍是空的"。
5. 负例（`AGENTS.md` §6 的口径）：改完之后**把守卫撤掉一处**重新执行步骤 5 的检查，
   确认它重新变红 —— 否则分不清"守卫在工作"与"检查没有覆盖到那一处"。

## 验收命令

```bash
cd src-tauri && for p in akasha-core akasha-pty akasha-serial; do
  cargo check -p "$p" --target x86_64-pc-windows-msvc || exit 1
done
# 期望：三条都退出码 0（改之前 akasha-pty 与 akasha-serial 是红的）

just check    # 期望：Linux 目标无回归，退出码 0
just ready    # 期望：fmt / lint / test / deny / gen-types / docs 六步全绿
```

- 完整判据（`akasha-store` / `akasha-ssh` / `akasha-bw` / `akasha` 四个成员）在 CI 的
  `checks-other` × Windows 上给出。**本机为 Windows 目标构建它们的 C 依赖不可能**，
  所以这一条只能等仓库有 remote 之后实际执行一次 —— 它不阻塞本 plan 的落地。

## 留下的缺口（不在本 plan 内）

Windows 上"回收整个会话"仍是空的：`kill_session` 返回 0，`Child::kill()` 只收得走 shell 本身。
POSIX 那一套（会话 / 进程组 / `setsid`）在 Windows 上不存在，等价物是 **Job Object**
（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`：句柄一关，作业里的进程全部结束），
它同时能替掉伴生看门狗在 Windows 上的那条路径。

不在这里实现的原因只有一条：**本机没有 Windows 主机**，连"现在的行为是什么样"
（ConPTY 关闭时到底带走多少进程）都观测不到。按 `AGENTS.md` §7，写出来也无法验收。
展开时机：有 Windows 主机可执行 E2E 时；届时先写 ADR（进程模型与 ADR-0005 同源），再动代码。

## 回滚

把 `teardown.rs` 的 `use rustix::process::…` 改回无门控即可回到"Windows 编译不过"的状态。
其余改动（`Cargo.toml` 的目标表、`detach` 的平台分派、`CREATE_NO_WINDOW`）可单独保留，
它们不影响 Linux 与 macOS。

## 实施记录

**改之前（问题 #149 的原文）**：`cargo check -p akasha-pty --target x86_64-pc-windows-msvc` 退出码
101、3 个错误 —— `unresolved import` `rustix::process`（teardown.rs:41）、
`cannot find process in rustix`（teardown.rs:105 / watchdog.rs:115）；`akasha-serial`
因依赖它报同一个错误。

**改之后（2026-09-15 实测）**：

| 检查 | 结果 |
|---|---|
| `cargo check -p akasha-core -p akasha-pty -p akasha-serial --target x86_64-pc-windows-msvc` | 三条都退出码 **0** |
| 负例：撤掉 `teardown.rs` 的 `#[cfg(unix)]` 再执行一次 | **重新变红**（`error[E0433]: cannot find module or crate rustix`），恢复之后又回到 0 |
| 同一条命令带 `--all-targets` | `criterion` → `alloca v0.4.0` 的 C 构建脚本要 MSVC 的 `lib.exe`（本机没有 MSVC 工具链）。**与本次改动无关**，记在这里以免下次误判 |
| `cargo check --workspace --all-targets`（Linux 目标） | 退出码 **0**，无回归 |
| `just ready` | 六步全绿 |

**顺带的一处**：`sigkill`（只被 Linux 的会话扫描调用）补上 `#[cfg(target_os = "linux")]` ——
在那之前它在 macOS 上是一个没人调用的私有函数（dead_code 警告）。

**没有做的事**（见「留下的缺口」）：Windows 上的会话回收。判据里那四个带 C 依赖的成员
（store / ssh / bw / app）本机核对不了，只能等 CI 的 Windows 格子 —— 而它要仓库先有 remote。
