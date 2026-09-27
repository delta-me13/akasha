# Plan 0112: macOS 上进程级判据的真实化

- **关联**：ROADMAP 阶段 1 ·「macOS 上的进程级判据真实化」
- **前置**：plan 0204 / 0205（零残留的两条退出路径与伴生看门狗）；`pty/teardown.rs` 的 macOS
  分支（`ps` + `getsid`）已落地 —— 本 plan 修的是**判据**，不是回收实现
- **状态**：已完成（2026-09-27）
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

**2026-09-27（macOS 26.6.2 / arm64；全部经 `just runner-run` 在沙箱外执行）**

1. **共享判活**（`tests/support/mod.rs`）：`process_state_visible()` / `process_is_alive(pid)`。
   Linux 读 `/proc/<pid>/stat` 的 state（`Z` = 已死），其他 unix 执行 `/bin/ps -o state= -p <pid>`
   （空输出 = 已回收，首字符 `Z` = 僵尸）。⚠️ **与步骤 1 的一处偏离**：`process_state_visible()`
   取 `cfg!(unix)` 而不是「Linux 与 macOS」—— `pty/teardown.rs` 的会话级回收本来就覆盖**全部 unix**
   （`cfg(all(unix, not(target_os = "linux")))` 那一支），只认两个平台会让 BSD 上的断言退化成恒假；
   Windows 仍为假，语义不变。读不出来按「已死」算，由调用点的正向断言守住（只会红，不会静默通过）。
2. 四个调用方改用共享判活：`exit_residue.rs`（连同探针的启动条件与两处文案）、`session_watchdog.rs`
   （跳过条件与理由）、`tab_close.rs` / `window_close.rs`（各自的 `alive()` / `process_death_visible()`
   删除）。`docs/just.md` §7 那行「Windows / macOS 只做类型检查」按现状改写。
3. **正例**：`just test` → `463 tests run: 463 passed, 0 skipped`（333.4 s），其中
   `akasha::session_watchdog a_sigkill_of_the_app_leaves_no_child_behind` **执行并通过**（3.18 s）；
   `just ready` → **6/6**（fmt 1s · lint 21s · test 141s · deny 1s · gen-types 33s · docs 3s）。
4. **正例（E2E 判据）**：`just test-e2e` 第二段 `exit_residue` 由 0.22 s 的空过变成真读数（0.28 s：
   探针 `Some(pid)` 真的启动、`process_is_alive(probe)` 真的为真、关窗之后真的消失）；第一段的
   `tab_close` / `window_close` 进程级断言同样是真的。⚠️ 该次运行的**整体退出码是 1**：
   `ssh_session` / `ssh_config_import` 的回声判据超时 —— 用 `git stash` 把本 plan 的全部改动移出之后
   重新执行 `just test-e2e`，**同一目标、同一断言、同样超时**，故与本次改动无关，记为新问题 #183。
   ⚠️ 本文验收命令里「耗时 > 1 s」那一条的预期不成立：真实路径是「轮询到就返回」，两条等待都在
   亚秒级完成（负例那一次才是 15 s 级）。
5. **负例**：`pty/teardown.rs` 的非 Linux 分支临时改成 `let _ = leader; 0` →
   * `just test`（临时给配方加 `--no-fail-fast`；不加时 nextest 在第一处失败处就结束，其后的用例一概不执行）：
     `463 tests run: 457 passed, 6 failed`，含 `session_watchdog`（20.1 s）与 5 条 `pty` 用例；
   * `just test-e2e`：`exit_residue` 报「退出后仍有残留：忽略 SIGHUP 的 74434 还活着」（15.3 s），
     `tab_close` 报「关闭标签页之后探针 A(72845) 还活着」；
   * 还原（`git checkout -- src-tauri/src/pty/teardown.rs`）之后重新执行，三条都转绿 ——
     「判据在工作」与「断言恒真」由此分得开。
6. **登记**：问题 #181 标为已修（索引行由 `just docs-archive` 移入归档）；新问题 #183 记
   `ssh_session` / `ssh_config_import` 的回声超时（原始工作区同样复现）。
