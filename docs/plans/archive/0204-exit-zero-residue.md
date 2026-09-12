# Plan 0204: 真正退出零残留

- **关联**：ROADMAP 阶段 2 ·「真正退出零残留（窗口关闭退出 / app 重载 / panic 三条路径）」
- **前置**：plan 0203（已能开 shell 并看到输出）
- **状态**：已完成（**两条路径**；`tauri dev` 重载那条见「实施记录」与 plan 0205）
- **影响面**：`src-tauri/src/**`（退出钩子、`shutdown_all`）、`src-tauri/crates/akasha-pty`（会话级回收）

## 目标

**真正退出**时每个 PTY 子进程都被**显式 kill + wait 收尸**，零残留。
覆盖三条路径：关闭窗口（当前 = 退出）、`tauri dev` 的 Rust 改动重载、panic。

> ⚠️ **"窗口关闭"在本 plan 里就是退出** —— 收托盘是阶段 3（plan 0301 / 0302）才存在的语义。
> 那时"关窗口不回收"会与本 plan 的方向**相反**，那条反向规则那时才落地。
> 本 plan 只保证：**真的退出时，一个子进程都不剩。**

## 非目标

- **不**做托盘（阶段 3）；**不**让窗口隐藏（plan 0302）
- **不**做 SSH / 隧道回收（阶段 5/6；届时 registry 的 shutdown-all 要覆盖它们）
- **不**用 drop 代替回收：`drop` 不是收尸路径（`AGENTS.md` §3.3）

## 前置检查

```bash
pgrep -af 'target/debug/akasha' || echo "当前无 app 在跑（预期）"
grep -rn 'kill\|wait' src-tauri/crates/akasha-pty/src | head    # 看 shutdown 路径现状
```

## 步骤

1. `SessionRegistry` 加**显式** `shutdown_all()`：逐个 `Transport::shutdown()` → `wait()` 收尸。
   - 不得依赖 `Drop`；不得只发信号不等回收（那会留下僵尸）
2. 挂三条退出路径：
   - **窗口关闭 / 退出事件**：先 `shutdown_all()` 再放行退出
   - **app 重载**（`tauri dev` 重编译重启）：同一入口必须被调用，不能在重载路径上漏掉
   - **panic**：设置 panic hook，尽力回收后再 abort（panic 路径允许"尽力"，但必须在日志里留下证据）
3. 回收顺序要确定：先停读循环（否则会继续从上锁的 PTY 读），再 kill，最后 wait。
4. 回归：反复起停 5 次，确认没有残留累积。

## 验收命令

```bash
just dev
# 1) 开一个 shell（例如跑 `sleep 600`），记下它的 PID（在终端里 `echo $$` 或从 ps 找）
# 2) 关闭窗口（当前 = 退出），然后在 shell 里确认：
pgrep -af 'sleep 600'     # 期望：无输出（零残留）
pgrep -af 'target/debug/akasha'   # 期望：无输出

# 3) app 重载路径：在 just dev 常驻的前提下改一个 Rust 文件触发重启，
#    重启完成后确认上一轮的子进程同样不残留：
pgrep -af 'sleep 600'     # 期望：无输出

# 4) 反复 5 次起停后仍无累积：
for i in 1 2 3 4 5; do echo "--- round $i"; pgrep -af 'target/debug/akasha|sleep 600' || echo clean; done
```

Victauri 侧（app 运行中）：`introspect { action: "processes" }` 记录子进程 PID ——
退出后拿这份清单在 shell 里逐个确认。**"收托盘时存在子进程"是阶段 3 的预期行为，不是本 plan 的失败。**

单测侧（可选但推荐）：`SessionRegistry::shutdown_all()` 对假 `Transport` 的调用序列
（shutdown → wait，且每个 session 恰好一次）。

## 回滚

回退退出钩子与 `shutdown_all`；回滚后回到"退出可能残留子进程"的旧状态（这是本来就该修的问题）。

## 实施记录

### 探针：什么才算"残留"

`sh -c 'trap "" HUP; echo AKPROBE=$$; exec sleep 600' &` —— 一个**明确忽略 SIGHUP** 的进程。
选它不是刁难，而是因为**别的探针测不出问题**：

| 探针 | 改前行为 | 说明 |
|---|---|---|
| 前台 `sleep 600` | 不残留 | 内核在 master 关闭时 SIGHUP 掉会话里的**全部**进程组 |
| 后台 `sleep 601 &` | 不残留 | 同上（不是"shell 转发"救的，是内核按 session 发的） |
| **忽略 SIGHUP** 的那个 | **三条路径都残留** | 只有点名 SIGKILL 收得走 |

改前实测（同一次会话，逐条累积）：关窗口后残留 1 个、SIGKILL 后 2 个、SIGTERM 后 **3 个**。
也就是说：`pgrep 'sleep 600'` 这种判据**在改之前也是绿的**，用它就等于没测。

### 改后（三条路径的实测输出）

| 路径 | 结果 |
|---|---|
| 关窗口退出 | ✅ E2E `exit_residue` 绿：「零残留：忽略 SIGHUP 的 1739 已随会话被收掉」；app 进程也真的退出 |
| panic | ✅ 临时把 `greet` 改成 panic 后实测：hook 先打崩溃现场 → `shut_down=2 failures=[]` → 日志有两条「会话已显式回收」→ `abort()`；探针同步消失 |
| `tauri dev` 重载 | ❌ **做不到，且原因确定**：上游 `run_dev_watcher` 用 `child.kill()` → `SharedChild::kill()` = **SIGKILL**，进程没有任何执行代码的机会。实测 `just dev` + 改 Rust 文件：新一代 app 起来了，上一轮的探针 **880 仍活着** |

### 手段

- `akasha-pty::teardown::kill_session`：Linux 扫 `/proc/<pid>/stat` 的 **session id**，
  把 `sid == 会话首进程` 的进程逐个 SIGKILL；**必须先于** kill 子进程调用（pid 复用窗口）。
  其他 unix 退到 `killpg`，Windows 无实现（要 Job Object）—— 缺口写在模块文档里。
- `Sessions::shutdown_all()`：整份搬出清单（不握锁 wait）→ 逐个 `Transport::shutdown()`
  （载体自己保证 kill + wait 收尸）→ 一起摘牌；幂等；失败**记账不上抛**。
- 挂点：`RunEvent::Exit`（**不用** `ExitRequested` —— 阶段 3 的"收托盘"正是在那里 `prevent_exit`）
  与 panic hook。

### 与计划的偏差（都有理由）

1. 步骤 3 的「先停读循环」**做不到**：阻塞在 `read()` 上的线程没法定点打断（`close` fd 对阻塞读
   不可靠，`read` 又没有超时接口）。**kill 才是停它的手段** —— 所以顺序是
   「收会话 → kill 子进程 → wait」，读循环在被 kill 后自己带出 EOF 收工。
2. 只做 `Transport::shutdown()`（= kill 那个 shell）**修不了本 plan 要修的问题**：
   忽略 SIGHUP 的进程它够不着。会话级回收是必需项，不是加料。
3. `Sessions` 改成 `Arc<Mutex<Inner>>` + `Box<dyn Transport>`：退出钩子/panic hook 要拿到
   **同一份**会话表（tauri 的 `State` 只借给命令），而装箱让 `shutdown_all` 能用假载体测调用序列。

### 顺手查出来的两件事（已修，记进 STATUS 的坑）

- `tauri-plugin-log` 默认带的 `TargetKind::LogDir` 在**日志目录不可写时会让插件初始化失败**，
  而插件失败 = **app 起不来**（实测：`PluginInitialization("log", "只读文件系统")` 直接 abort）。
  现在只留 Stdout —— 一个终端不该因为日志文件写不了就打不开。
- `tracing/log-always` 会把**依赖树里**的 tracing 事件一起转成 `log` 记录
  （含 `tracing::span::active` 这种平时看不见的）。默认级别下实测把 app 日志刷成几十万行、
  明显拖慢 app；级别显式定在 `Info`。

### 未覆盖

- **SIGTERM / SIGINT**（`kill <pid>`、Ctrl+C）：进程直接死，钩子跑不到。今天的行为**不比改前差**
  （内核 hangup 照旧级联），但也没变好 —— 记在 plan 0205 的候选方案里。
- 非 Linux 平台没有会话级回收（Windows / macOS 的探针会留下来）。

