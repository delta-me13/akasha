# Plan 0205: 被 SIGKILL 的退出路径也零残留

- **关联**：ROADMAP 阶段 2 ·「被 SIGKILL 的退出路径也零残留」
- **前置**：plan 0204（能执行代码的两条路径已经零残留；本 plan 只处理"无人能执行代码"的那条）
- **关联 ADR**：[ADR-0005](../../adr/0005-sigkill-exit-watchdog.md)（方案就是它定的）
- **状态**：已完成
- **影响面**：`src-tauri/crates/akasha-pty`（新模块 `watchdog`、`Transport` 加
  `session_leader()`）、`src-tauri`（启动接线 + 退出路径）、`src-tauri/justfile`（E2E 清单）

## 目标

`tauri dev` 重编译重启、`kill -9`、`kill -TERM` 之后，**上一轮会话里的进程一个都不剩**
—— 包括**忽略 SIGHUP** 的那些。

## 为什么 0204 修不了它（实测，不是猜测）

`tauri dev` 的重启是 `child.kill()` → `SharedChild::kill()` = **SIGKILL**（上游
`crates/tauri-cli/src/interface/rust.rs`）。SIGKILL 不可捕获，进程里**没有任何代码会被执行**
—— 所以"退出前显式回收"这条路在这条路径上从原理上就不通。0204 的实测输出：
新一代 app 已启动，上一轮的探针仍存活。

## 非目标

- **不**做托盘（阶段 3）；**不**改 0204 已经验证过的两条路径（它们是更早、更精确的回收点）
- **不**做完整 supervisor（PTY 归它持有）—— 那是阶段 5/6 的形态，ADR-0005 §4 记了理由
- **不**做 Windows 的 Job Object / macOS 的 `proc_listpids`（见"未覆盖"）

## 决定（ADR-0005）：伴生看门狗 + 单向管道协议

**看门狗就是 app 可执行文件本身再运行一次**（`--akasha-session-watchdog`），它读一条管道，
**读到 EOF 就把登记过的会话全部回收**：

| 环节 | 位置 |
|---|---|
| 协议（`+<pid>` / `-<pid>`）+ 解析 + 主循环 + 启动进程 | `akasha-pty/src/watchdog.rs` |
| "这个载体背后是哪个会话" | `Transport::session_leader()`（PTY 覆写成 shell 的 pid） |
| 登记 / 撤销的确切时机 | `src-tauri/src/session.rs` 的 `register` / `close` / `shutdown_all` |
| 识别 argv、启动、与终端脱钩 | `src-tauri/src/watchdog.rs` + `main.rs` 第一行 |

关键性质：**触发信号是管道 EOF**（由进程的 fd 表决定），不是任何"父/子"关系 ——
所以 app 怎么死都无所谓（SIGKILL / SIGTERM / abort / 段错误），也不关心会话是从哪个线程启动的。

`PDEATHSIG` 被否掉的两条硬伤（ADR-0005 §4）：① 只作用于**直接子进程**，无法终止 shell 里
忽略 SIGHUP 的孙进程 —— 而那正是本 plan 的验收判据；② 它的"父"是**创建它的那个线程**，
从线程池里启动会话就会"A worker 线程退下去 → 误杀用户的会话"。

## 步骤

1. **定方案并写 ADR**：[ADR-0005](../../adr/0005-sigkill-exit-watchdog.md)（PDEATHSIG /
   完整 supervisor / 扫孤儿 / 单独 bin 逐条否掉，见 §4）。
2. **`akasha-pty::watchdog`**：协议编解码、`run`（读协议 → EOF → 逐个 `kill_session`）、
   `detach`（`setsid`，使 `^C` 与关闭终端不能终止它）、`SessionWatchdog`（启动进程、`watch`/`forget`）。
3. **`Transport::session_leader()`**（默认 `None`）：PTY 覆写成子进程 pid ——
   与 `PtyTransport::shutdown` 用的是同一个 pid。
4. **接线**：`Sessions` 先登记会话再让它"存在"（登记排在插表之前、锁之外）；
   `close` / `shutdown_all` 收干净之后才撤销登记（收尾失败就**不撤销**，留给看门狗再试）。
5. **启动顺序**：看门狗在最前面启动（会话一存在就必须已经在兜底清单里），但**日志推迟到
   `.setup()`** —— 早于日志插件注册的 `tracing` 事件会**静默消失**（实测遇到过）。
6. **测试**：`akasha-pty` 的真进程用例（含诱饵）、`src-tauri` 的协议断言用例、
   一条**真 SIGKILL** 的集成用例（真实 app 二进制 + 真管道 + 真会话）。

## 验收命令

```bash
cargo test --package akasha --test session_watchdog   # 真 SIGKILL + 真实 app 二进制
just test                                            # 全部单测 + 那条集成用例
just ready                                           # 可执行的 DoD（6 步）
```

三条真实路径的实测（`tauri dev` 重载 / `kill -9` / `kill -TERM`）：
**脚本 + 原始输出见「实施记录」** —— 判据是一条"忽略 SIGHUP"的探针在每条路径之后
都从 `/proc` 里消失。

## 回滚

回退 `watchdog` 模块与它的接线（`Sessions::attach_watchdog` 不再被调用即可，
`None` 分支就是原样）；回滚后回到 plan 0204 的记录状态
（能执行代码的两条路径干净、SIGKILL/SIGTERM 路径有残留）。

## 未覆盖（照实记录，不声称零残留）

- **没有门禁的那条路径**：`tauri dev` 的重编译重启只能靠**手动**实测（`just test-e2e` 启动的是
  自己的 app，改不了它的源码去触发重载）。机制本身有门禁 —— `tests/session_watchdog.rs`
  用**真 SIGKILL** 终止真实会话，那条用例在 `just test` 里。**但"CLI 真的用 SIGKILL"这件事
  没有自动断言**，只能靠上游源码 + 这条手动实测。
- **登记与死亡之间的窗口**：`spawn()` 返回后、`+<pid>` 写进管道前的**几个微秒**里
  app 被 SIGKILL，那个会话没人登记。EOF 之前的缓冲数据不会丢（内核先交付再报 EOF），
  所以窗口只在这几微秒。
- **Windows / macOS**：Windows 没有 POSIX 会话（等价物是 Job Object），macOS 目前退到
  `killpg`（作业控制下的作业收不到）。集成用例非 Linux 时**显式跳过并打印原因**。
- **`SIGSTOP` 住的看门狗**：它读不动管道，收尾会推迟到它恢复运行。不是"收不掉"，
  但也不是"立刻"。

## 实施记录

### 落地内容

| 位置 | 内容 |
|---|---|
| `akasha-pty/src/watchdog.rs`（新） | `FLAG` / `is_invocation` / `Verb` / `parse` / `encode` / `run`（读协议 → EOF → 逐个 `kill_session`）/ `detach`（`setsid`）/ `SessionWatchdog`（启动进程、`watch`、`forget`、回收线程） |
| `akasha-pty/src/transport.rs` | `Transport::session_leader()`（默认 `None`） |
| `akasha-pty/src/pty.rs` | PTY 覆写成 `process_id()` —— 与 `shutdown` 用的是**同一个** pid |
| `src-tauri/src/session.rs` | `attach_watchdog`（`OnceLock`）；`register` **先登记再插表**；`close` / `shutdown_all` **收干净才撤销**（收尾失败**不撤销**，留给看门狗再试） |
| `src-tauri/src/watchdog.rs`（新） | `run_if_watchdog`（识别 argv → `detach` → 主循环）、`start_early` / `Startup` / `report` |
| `src-tauri/src/main.rs` | **第一行**识别看门狗模式（早于 Tauri，否则已经启动了一个窗口） |
| `src-tauri/justfile` | E2E 清单加 `E2E_NO_APP`：不需要真实 app 的集成测试有**明确的归属清单**，不再被 guard 判红 |

一个细节值得记下：`parse` 里**不把数字交给 `u32::from_str` 去宽容** —— 它接受前导 `+`，
于是 `++1` 会被读成"登记 pid 1"，而 pid 1 是 init（看门狗会去 SIGKILL 它）。现在逐位校验。

### 验收命令的实际输出

- `just ready` —— **6/6 全绿**（fmt-check / lint / test / deny-offline / gen-types-check / docs-check）
- `just test` —— **66 tests run: 66 passed**（akasha-pty 37，其中 8 条是看门狗；
  akasha 21；akasha-core 8）
- `cargo test --package akasha --test session_watchdog` —— **1 passed**（0.15 s）：
  真实 app 二进制 + 真管道 + **真 SIGKILL** + 忽略 SIGHUP 的探针 + **诱饵会话**
  （从没登记过的那条必须不受影响），结束后还断言看门狗自行结束
- `just test-e2e` —— **9 个用例全绿**（含 0204 的 `exit_residue`：关窗口路径**没有**退化）

**红-绿自检**：临时让 `run()` 跳过 kill，同一条集成用例 20 s 后**报红**；去掉探针后通过
—— 它测的是修复本身，不是任何实现都能通过。

### 三条真实路径的实测

脚本临时写在 `target/`（运行完即弃：它要真实启动 `pnpm tauri dev`、真要改一个 Rust 文件）。
判据只有一条：**忽略 SIGHUP** 的探针（`sh -c 'trap "" HUP; …; exec sleep 600' &`）
在 app 死掉之后必须从 `/proc` 里消失，且整个命名空间里不再有 `sleep 600`。

| 路径 | app pid | 探针 pid | 结果 |
|---|---|---|---|
| A `tauri dev` 重编译重启（CLI 的 SIGKILL） | 184 → 510（新一代） | 479 | ✅ 探针消失、`sleep 600` 一个不剩 |
| B `kill -9` | 1104 | 1393 | ✅ 同上 |
| C `kill -TERM` | 1456 | 1746 | ✅ 同上（**0204 也没覆盖过这一条**） |

每条路径的 app 日志里都有"兜底真的在位"的证据（这是与"偶然通过"的区别）：

```text
[akasha_lib::watchdog][INFO] 看门狗已启动：app 被 SIGKILL（`tauri dev` 重载 / `kill -9` / `kill -TERM`）时由它收掉会话 watchdog=Some(208)
```

### 三个中途发现的问题（都进了 STATUS）

1. **早于日志插件的 `tracing` 事件会静默消失**：看门狗启动得比 `tauri-plugin-log` 早，
   第一版把"已启动"当场输出 —— app 日志里**没有任何输出**（只有 victauri 的 INFO）。
   于是拆成 `start_early`（启动进程 + 完成挂接）与 `report`（`.setup()` 里再报）。
2. **SIGKILL 之后立刻读 `/proc/<pid>/stat` 会读到 `R` 或 `Z`**：`R` 是信号还没投递，
   `Z` 是父进程还没 `wait`。把 `Z` 当"仍存活"让两条**已经成功**的路径报红了一次 ——
   与问题 #45 同族，只是换了对象。
3. **"命令行里含 `sleep 600`"会匹配到沙箱包装进程**：bwrap 自己的 cmdline 里带着整段
   脚本文本，容易造成误判。判据必须收紧到"argv 恰好是 `sleep 600`"。
