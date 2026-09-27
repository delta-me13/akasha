# Plan 0112: macOS 上进程级判据的真实化

- **关联**：ROADMAP 阶段 1 ·「macOS 上的进程级判据真实化」
- **前置**：plan 0204 / 0205（零残留的两条退出路径与伴生看门狗）；`pty/teardown.rs` 的 macOS
  分支（`ps` + `getsid`）已落地 —— 本 plan 修的是**判据**，不是回收实现
- **状态**：未开始
- **影响面**：`src-tauri/tests/support/mod.rs`、`exit_residue.rs`、`session_watchdog.rs`、
  `tab_close.rs`、`window_close.rs`、`docs/STATUS.md`、`ROADMAP.md`、`docs/plans/README.md`

## 目标

把「这个 pid 现在真的死了吗」从**恒真**变成**真读数**，让 macOS 上三处进程级判据真的成立：

1. `exit_residue`（`test-e2e` 第二段）：关窗退出之后，app 与那个**忽略 SIGHUP** 的探针都不在；
2. `session_watchdog` 的 `a_sigkill_of_the_app_leaves_no_child_behind`（`E2E_NO_APP`，由 `just test` 执行）；
3. `tab_close` / `window_close` 的进程级断言（今天在非 Linux 上一律退化成恒真）。

判据（来自 ROADMAP）：**macOS 上这几条有非空读数，且撤掉会话级回收后必红**。

## 非目标

- **不改 macOS 的回收实现**：`kill_session` 在非 Linux 的 unix 上走 `/bin/ps` 列 pid + `getsid`
  判会话，已经与会话等价（见 `pty/teardown.rs` 的平台差异表），并有专为 macOS 门控的真机会话用例。
- **不做 Windows 的进程判活**：`tasklist` 与 MSYS 的 PID 语义不同（问题 #162 那一类），
  本 plan 只把「看不到」这件事保持为显式跳过。
- **不改 `single_instance` 第 5 层**：它要的是「按可执行文件数实例」，不是「某个 pid 是否存活」，
  两者共用一个 helper 会把两件事混在一起。
- **不动 `vault_unlock` / `passphrase_contract` 那几条 `VmLck` 判据**：它们缺的是 macOS 上的
  `/proc` 等价读数（ADR-0002 D13 已登记的边界），与 pid 存活判定无关。

## 前置检查（这次审计的证据）

```bash
grep -n "fn alive" -A 3 src-tauri/tests/exit_residue.rs src-tauri/tests/session_watchdog.rs
grep -n "Skipping\|跳过: 会话级回收" src-tauri/tests/session_watchdog.rs src-tauri/tests/exit_residue.rs
```

- `exit_residue.rs:92` 的 `alive()` 读 `/proc/<pid>` 且**没有平台门控** —— macOS 上 `/proc` 不存在，
  它恒为 `false`，于是关窗那条「app 真的退出了」的断言**立刻为真**；
- `exit_residue.rs:189` 在非 Linux 打印「跳过」且不启动探针，而文件头 `:23` 自称
  「非 Linux 上只断言关窗口 = app 真的退出」—— 那句断言在 macOS 上没有证据；
- `session_watchdog.rs:170` 的跳过理由写「macOS 要 `proc_listpids`」，**已过期**：那条实现已由
  `ps` + `getsid` 补齐，理由应按新事实改写；
- CI 实测（`E2E（macOS）` 作业日志）：这一段 `1 passed ... finished in 0.22s` —— 真的走完
  「关窗 → 等 app 退出 → 等探针消失」要 30 s 级窗口，0.22 s 只能是空过。

## 步骤

1. **落一个共享判活**（`src-tauri/tests/support/mod.rs`；它已有 `#![allow(dead_code)]`，
   且已是跨目标脚手架的住处）：

   ```rust
   pub fn process_state_visible() -> bool     // Linux 与 macOS 为真，其余平台为假
   pub fn process_is_alive(pid: u32) -> bool  // 只有前一行为真时才有意义
   ```

   - **Linux**：`/proc/<pid>/stat` 的 state ≠ `Z`（从 `tab_close.rs:163` 与 `window_close.rs:102`
     收拢，两处实现逐字相同）；
   - **macOS**：`/bin/ps -o state= -p <pid>` —— **空输出 = 已回收 = 死**；首字符 `Z` = 僵尸 = 死；
     其余状态（含 `?E` 这类过渡态）算活着，让等待自然走完；
   - **⚠️ 僵尸这一档必须处理**：`exit_residue` 观察的 app 是配方 shell 的子进程，它退出后、
     shell `wait` 之前是**僵尸**。只问 `kill(pid, 0)` 会把僵尸判成「还活着」，30 s 之后得到一个假红。
2. **`exit_residue.rs`**：`alive()` 换成共享判活；探针的启动条件从 `cfg!(target_os = "linux")`
   改成 `support::process_state_visible()`（探针命令 `sh -c 'trap "" HUP; …' &` 在 macOS 的
   `/bin/sh` 上同样成立）；文件头与两处跳过文案按新事实改写。
3. **`session_watchdog.rs`**：同一替换；跳过条件改成 `process_state_visible()`，理由改成
   「Windows 上进程判活不可用」。
4. **`tab_close.rs` / `window_close.rs`**：删掉各自的 `process_death_visible()` 与 `alive()`，
   改用共享判活。这两处今天在非 Linux 上返回恒真，改完 macOS 会**真的断言** —— 这是本 plan 的
   预期变化，不是回归。若真的红，按缺陷处置，**不得**把断言改回去。
5. **负例（成对，口径见 `AGENTS.md` §6）**：临时让 macOS 上的会话级回收失效
   （`pty/teardown.rs` 的 `kill_session` 在非 Linux 分支直接返回 `0`）→ `exit_residue` 与
   `session_watchdog` 那一条必须**变红**；恢复后转绿。没有这一步就分不清「判据在工作」与
   「断言恒真」，而后者正是本 plan 要修掉的东西。
6. **登记与订正**：在 `docs/STATUS.md` 登记问题 #181，并把「待验证」里
   「权限位、单实例、托盘在非 Linux 平台未验证」那条按 CI 六格全绿的现状改写；
   `docs/just.md` §8 里「Windows / macOS 只做类型检查」一并订正（macOS 另有 `e2e-macos`）。

## 验收命令

```bash
# 1) macOS 上真的执行（沙箱外：pty 设备与 ~/.cargo 都在工作区之外）
just runner-run test-e2e
#    期望：退出码 0；日志里不再出现「跳过: 会话级回收…」；
#          exit_residue 那一段的耗时 > 1s（不再是 0.2s 的空过）

# 2) 看门狗那一条（不需要 app；同样在沙箱外执行）
just runner-run test
#    期望：session_watchdog 的 a_sigkill_of_the_app_leaves_no_child_behind 执行并通过（不是跳过）

# 3) 负例（步骤 5：撤掉会话级回收 → 必红；恢复 → 转绿）

# 4) 不回归
just runner-run ready
#    期望：六步全绿（fmt / lint / test / deny / gen-types / docs）
```

- CI 的 `E2E（macOS）` 给同一读数的第二个来源：日志里那两条跳过行不应再出现，
  `exit_residue` 的耗时也不再是秒级以下。

## 回滚

判据改动只落在测试里：把 `exit_residue.rs` / `session_watchdog.rs` 的判活改回 `/proc` 版本，
即回到今天的空过状态；共享判活与另两处替换可单独保留。

## 实施记录

（边做边追加：负例的红、恢复后的绿、macOS 与 Linux 两侧的实际输出。）
