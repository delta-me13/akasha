# ADR-0005：被 SIGKILL 的退出路径由**伴生看门狗进程**收尾

- **状态**：**已定案**（Frozen，2026-09-12）
- **日期**：2026-09-12
- **决策者**：cyrene
- **影响范围**：进程模型（app 多一个常驻伴生进程）、`Transport` 契约（多一个
  `session_leader()`）、每一条未来的载体（SSH / 串口）都要回答"我有没有本地进程要代收"、
  以及 `akasha-pty` / `src-tauri` 的启动与退出路径
- **关联**：[plan 0204](../plans/archive/0204-exit-zero-residue.md)（可执行代码的两条路径）、
  [plan 0205](../plans/archive/0205-sigkill-exit-residue.md)（本决策的落地）

---

## 1. 背景

`AGENTS.md` §3.3 要求"真正退出 → 零残留"。plan 0204 用**在进程里显式回收**（`RunEvent::Exit`
+ panic hook + `kill_session`）覆盖了其中两条路径，但第三条**从原理上无法做到**：

| 路径 | 谁执行回收 | 结果 |
|---|---|---|
| 关窗口 / 正常退出 | `RunEvent::Exit` | ✅ 零残留（0204 实测） |
| panic | panic hook | ✅ 零残留（0204 实测） |
| **`tauri dev` 重编译重启** | **进程内无代码可执行** | ❌ 忽略 SIGHUP 的进程留下 |
| **`kill -9` / `kill -TERM`** | **进程内无代码可执行** | ❌ 同上 |

原因是实测（读上游源码 + 复现）：`tauri dev` 的重启走的是 `child.kill()` →
`SharedChild::kill()` = **SIGKILL**（`crates/tauri-cli/src/interface/rust.rs` 的
`child.kill().context("failed to kill app process")`）。SIGKILL 不可捕获，**进程里没有任何
一行代码会执行**。`kill -9` 同理；`kill -TERM` 也等价，因为 app 没有信号处理。

内核在这种时刻只提供一种手段：**PTY 挂断时它会按会话发 SIGHUP** —— 而那对
`nohup` / `trap "" HUP` / setsid 出去的守护进程无效。plan 0204 的基线记录里，这类进程在
三条路径上**均未被回收，且反复启动与终止会累积**。

## 2. 决策

**额外启动一个"看门狗"伴生进程，其职责只有一项：读一条管道，读到 EOF 即回收全部已登记的会话。**

- 看门狗**就是 app 可执行文件本身**再执行一次（`--akasha-session-watchdog`），
  由 `main` 的第一行识别该模式 —— 不进 Tauri、不开窗口。
- 协议单向、逐行、只有两种：`+<会话首进程 pid>`（登记）、`-<会话首进程 pid>`（撤销）。
  app 创建会话时先登记、回收完毕后再撤销；**EOF 表示"app 已终止"**。
- 看门狗 `setsid()` 与终端脱钩，因此 `^C`、关闭终端都不会终止它 —— 其职责正是"app 终止后仍存活"。
- 它**不输出任何东西**：它触发时 app 已不存在，没有读者。

关键性质：**触发信号是管道 EOF，而不是任何"父进程/子进程"关系**。因此它不依赖 app 的终止方式
（SIGKILL、SIGTERM、abort、段错误），也不依赖会话由哪个线程发起。

## 3. 理由

1. **EOF 由进程的 fd 表决定，不由线程决定。** 这一条是它胜过 `PDEATHSIG` 的根本原因（见 §4）。
2. **它回收的是"会话"，不是"子进程"。** 用的就是 plan 0204 已经验证过的 `kill_session`
   （扫 `/proc` 里 `sid == 首进程 pid` 的进程逐个 SIGKILL）—— 忽略 SIGHUP 的进程同样会被回收。
3. **不改动数据通路。** app 仍然自己持有 PTY master、自己读写字节；看门狗只额外登记一份
   pid 清单。plan 0204 已经通过的 9 条 E2E 用例（含 raw 通道 10 MB、终端渲染）不受影响。
4. **对将来的后端是同一个入口。** SSH 连接 / 隧道 / 串口要回收的对象不同（进程组、连接、
   服务端会话），但"app 被 SIGKILL 时由谁完成回收"这个问题只有一个答案：
   `Transport::session_leader()` 往后可以泛化成更抽象的代收凭据，而看门狗与协议无需改动。
5. **`current_exe()` 在 dev / release / 打包后是同一个答案。** 不引入第二个 bin 即避开了
   构建（`cargo run` 只执行默认 bin）与打包（sidecar 配置）两处耦合。

## 4. 被否掉的替代方案

| 方案 | 为什么不选 |
|---|---|
| **Linux `PR_SET_PDEATHSIG`**（子进程在父进程终止时被终止） | 两处根本缺陷。① **只作用于直接子进程**：它能终止该 shell，但无法终止 shell 中忽略 SIGHUP 的孙进程 —— 而孙进程正是本 ADR 需要回收的对象，实测的验收判据直接不通过。② **"父"是创建它的那个线程**，而会话是从 Tauri 命令（线程池）中发起的：一个 worker 线程退出即会**误终止用户正在使用的会话**。采用它就必须额外保证"必须从长驻线程发起 spawn"—— 那是一条静默约束，此后任何把 spawn 挪到别处的改动都会导致随机终止会话。 |
| **完整 supervisor**（PTY 由监管进程持有，app 只与它通信） | 它是**最终形态**（SSH/隧道的回收也需要它，见 `docs/scope.md` 阶段 5/6），但那要求把字节通路整个搬出 app：raw 通道的调用方、合批、背压、E2E 均需改动。为"退出兜底"这一项支付该代价不成比例 —— 本决策是它的**窄化版**（只管 pid 清单，不管字节）。 |
| **重启后的新实例扫描孤儿进程**（找"父是 1、带本应用特征"的） | 判据不可靠：可能终止其它程序的进程；且只覆盖"有人重新启动"的场景 —— `kill -9` 之后无人重启时不起作用。 |
| **单独做一个 `akasha-watchdog` bin** | 与构建/打包耦合：`cargo run` 只执行默认 bin（`src-tauri/Cargo.toml` 的 `default-run` 就是为同类问题加的），纳入产物还需 `externalBin` sidecar。而 `current_exe()` + 一个 argv 标志在三种形态下均成立。 |
| **cgroup / systemd scope 随 app 终止** | 依赖 systemd 与用户级 unit，不可移植（Windows/macOS 均需另寻方案），而且引入了"用户机器上必须有 systemd"的隐含要求。 |

## 5. 代价与对策（照实记下）

| 代价 | 对策 / 现状 |
|---|---|
| 多一个常驻进程（每个 app 实例一个） | 它只阻塞在 `read_line` 上，不轮询、不占 CPU；回收子进程的线程负责 `wait`，不留僵尸 |
| 多一条**跨进程协议**要维护 | 只有两种行，编解码在 `akasha_pty::watchdog` 里同一处，并有"写出去的行必须能被 `parse` 解析"的往返用例守着 |
| 会话登记与"app 被 SIGKILL"之间有一瞬的窗口（登记行还在管道缓冲里就没写出去） | 内核**先交付缓冲数据、再报 EOF**，所以"刚登记即被终止"是安全的；真正的窗口是 `spawn()` 返回与写 `+` 之间那几个微秒 —— 已知且记在 plan 0205 的"未覆盖"里 |
| 平台差异：Windows 没有 POSIX 会话 | 照实记为**未实现**（等价物是 Job Object）；macOS 目前退到 `killpg`。两者都由 `teardown` 模块与 `docs/STATUS.md` 记录，不假设两者行为一致 |

## 6. 复审条件

- **完整 supervisor 立项时**（阶段 5/6 的 SSH/隧道回收）：重新评估"看门狗 + app 自己持有
  资源"是否还成立 —— 那时正确的答案很可能是"资源归 supervisor，app 只是客户端"，
  本 ADR 的看门狗会被它取代（而不是叠加）。
- **Windows 支持排上日程时**：Job Object（`kill on job close`）能给出同样的保证，
  届时决定是"Windows 走 Job Object、unix 走看门狗"还是统一到 supervisor。
- 若出现**多实例共享一套会话**的形态（例如一个后台服务被多个窗口共用），
  "看门狗的生命周期 = 一个 app 实例"这个前提需要重新审视。
