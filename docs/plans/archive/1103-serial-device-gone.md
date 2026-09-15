# Plan 1103: 设备消失时串口会话以可读原因结束

- **关联**：ROADMAP 阶段 11 ·「设备消失时串口会话以可读原因结束，标签页随之关闭」
- **前置**：plan 1101（会话能打开、能回收）· plan 1102（界面能开一条串口会话）
- **状态**：已完成（2026-09-15）

## 目标

把测试用的 PTY 主端关闭（等价于拔掉设备）之后：

| 要发生什么 | 由谁负责 |
|---|---|
| 那条会话**自己结束** | 读端报错 → 流结束 → `retire`（plan 1101 已通，本条不改一行） |
| 标签页随之关闭 | `session_ended` → `closeTab`（plan 0306 已通） |
| 界面上说得出**为什么** | 本条：载体把读端失败的原因交出来 |
| 没有残留注册 | 本条：把它变成一条可断言的读数 |

串口没有退出码，所以 `SessionEnded.status` 一直是 `null` —— 设备被拔掉与 shell 正常退出
在界面上长得一模一样。本条补的就是这一格。

## 非目标

- 自动重连与热插拔事件（`scope.md` §10，`later`）—— 本条只保证"结束得干净、看得见"
- 设备回来之后怎么接（重新打开是一条新会话）
- SSH 那条路（连接中途断掉）的同类原因：它有自己的结局语义（ADR-0003 D4），不借这一条
  一并改；`Transport` 上那个新方法的默认实现对它是 `None`

## 前置检查

实测（2026-09-15，本机；一次性探针 `akasha-serial/tests/probe_device_gone.rs`，读完数即删）：

```console
$ cargo test --package akasha-serial --test probe_device_gone -- --nocapture
PROBE: Err kind=BrokenPipe display=Broken pipe raw_os_error=None
PROBE: 耗时 26.119µs
PROBE: 写失败 kind=Io(Custom { kind: BrokenPipe, error: "Broken pipe" }) display=IO 失败：Broken pipe
PROBE: shutdown = Ok(None)
```

三件必须记住的事：

1. ⚠️ **拔掉设备是 `Err`，不是 `Ok(0)`**：从端那一路的 `read` 立刻返回 `BrokenPipe`。这一条是
   整条路的前提 —— `SerialReader`（plan 0801）把 `Ok(0)` 当"设备安静"继续等，所以若上游
   用 `Ok(0)` 表示消失，这条会话会**永远不结束**（而且忙等）。上游的实现（`serialport` 4.10.1
   `posix/poll.rs::wait_fd`）是 `poll` 见到 `POLLHUP`/`POLLNVAL` 就报 `BrokenPipe`，否则报
   `Other(EIO)`；两条都是 `Err`，所以真设备被拔掉（EIO 那一支）与 PTY 主端关闭走的是同一个判定。
2. **`shutdown()` 答不出这件事**：它返回 `Ok(None)`（"串口没有结局可报"，plan 1101 定的），
   `write` 那一侧也各自失败（`TransportError::Io`）。所以原因只能由**读端**带出来，而它现在
   在 `akasha_pty::spawn_batcher` 的读循环里被丢掉（`Err(_) => return`）—— 那条循环同时服务
   PTY 与 SSH，本条**不改它**：串口自己知道，就由串口自己记。
3. **原因必须由载体持有**：读端线程是唯一见到那个 `io::Error` 的地方，而它拿的是
   `Box<dyn Read + Send>`。所以记录点在与读端共享的那份状态里（`Arc<Mutex<…>>`，与 plan 1101
   的停止标志同一个模式），收尾时由载体读出。

## 步骤

1. **载体契约**：`akasha-pty` 的 `Transport` 加 `stream_error(&self) -> Option<String>`，默认
   `None`。文档写清它与 `shutdown()` 的分工：后者答"结局是什么"（退出码 / 信号），这一条答
   "输出流为什么不是正常结束"。返回已经渲染好的句子 —— 载体自己知道该说什么（设备路径、方向），
   会话层不替它拼。
2. **串口侧**：`SerialReader` 在**真实读错误**时把 `io::Error` 记进与载体共享的那一格
   （`is_idle` 那两类不算：超时是"现在没有数据"）；`SerialTransport` 存下设备路径，
   `stream_error()` 用新增的 `SerialError::DeviceGone { path, source }` 渲染。
   收尾立起停止标志之后读端给 `Ok(0)`，所以"我们自己关的"不会留下原因。
3. **会话层**：`Retired` 加 `stream_error: Option<String>`（`retire` 在 `shutdown()` 之后读），
   `SessionEnded.status` 取**结局优先、原因次之** —— 契约形状不变（仍是 `Option<String>`），
   语义从"退出码/信号"扩成"这次结束的可读描述"。`log_ended` 相应多一支：
   `err = %原因`（`%err` 是 `docs/logging.md` §2 已登记的字段形态）。
4. **前端**：`src/ipc/session.ts` 把 `session_ended` 的 `status` 透传给 `onEnded`
   （**现在整个被丢掉**：本地的"退出码 3"也一样看不见）；`attach.ts` / `TerminalPane` 逐层带下去；
   `App.tsx` 关标签页之前把那一句写进应用级通知行（`[data-session-notice]`）—— 标签页自己要关，
   所以那一句不能留在那个面里。
5. **E2E**：`tests/support/` 的假设备加一个**可拔插**的构造（不读主端，于是 `unplug()` 真的
   关得掉设备——那条读线程持有一份主端副本，`try_clone_reader` 的 dup 会让 PTY 一直有效）；
   新增 `tests/serial_device_gone.rs`，登记进 `justfile` 的 `E2E_TARGETS`（排在 `serial_ports_ui` 之后）。

## 验收命令

前置：`just dev` 不必开，`just test-e2e` 会自己起 app（已有则复用）。

```bash
# 1) 库内：拔掉设备时读端报错、原因是它、停止之后不再有原因
cargo nextest run --package akasha-serial
# 预期：N passed（含 3 条新用例），0 failed

# 2) 门禁（含生成物已提交这条）
just ready
# 预期：✅ just ready 全绿（6/6）

# 3) 端到端：新的 serial_device_gone
just test-e2e
# 预期：exit 0，且 serial_device_gone 打印——
#   打开前 sessions = {"live":1,"registered":1}
#   拔掉之后：标签页随之关闭，通知行里有 "串口设备已断开"，sessions 回到打开前
```

**判据怎么收口**（ROADMAP 原文：把测试用的 PTY 主端关闭时会话结束、没有残留注册）：

| 判据 | 断言 |
|---|---|
| 会话结束 | 界面上的标签页数回到打开前（会话自己走 → 标签页跟着走） |
| 原因可读 | `[data-session-notice]` 的那句话里含"串口设备已断开"与设备路径（**读的是界面渲染出来的那句话**） |
| 没有残留注册 | `app_state { probe: "sessions" }` 回到打开前的读数（串口没有本地进程，`processes` 那一条对它不适用） |
| 别把"安静"当"消失" | 库内：设备还在时读端拿不到字节（`TimedOut` 重试），拔掉之后才报错 |

## 回滚

- 通知行不想要了：`App.tsx` 去掉那一处渲染即可 —— `status` 的透传与后端行为不受影响。
- 原因不想要了：`akasha-pty` 那个方法删掉、`akasha-serial` 的记录点删掉、`Retired` 那个字段删掉，
  会话层回到"串口没有结局可报"（plan 1101 的行为）。

## 实施记录

### 落地时定下的四件事

1. **原因进 `SessionEnded.status`，契约形状不变**：串口没有退出码，`Transport::shutdown()`
   只能答 `Ok(None)`；原因是载体自己报的（`Transport::stream_error`，默认 `None`）。
   会话层算"结局优先、原因次之"，于是生成物只多了一段文档、字段一个没变 —— 而前端
   此前**一直丢掉**的 `status` 终于有人读（本地终端的"退出码 N"也一起可见）。
2. **记录点在读端**：`akasha_pty::spawn_batcher` 的读循环对错误与 EOF 一视同仁
   （`Err(_) => return`；它同时服务 PTY 与 SSH，那条判定不为串口改），所以"为什么结束"
   出了读端就没有第二个人知道。串口因此自己记（与停止标志同一个 `Arc<Mutex<…>>` 模式），
   当场渲染成一句话（`SerialError::DeviceGone`，带设备路径）。收尾会先立起停止标志，
   读端此后给的是 `Ok(0)`，所以"我们自己关的"不会留下原因 —— 单测两面都钉住。
3. **那句话必须住在壳层**：标签页与会话同生命期，说出原因的那个面**随即被卸载**，
   所以 `status` 现在逐层交到 `App.tsx`（`adoptSession` → `attach` → `TerminalPane` → `App`），
   由它先记进通知行（`[data-session-notice]`）再关闭标签页。顺序反过来就没有地方说这句话了。
4. **E2E 的假设备必须能真的拔掉**（`FakeSerialDevice::unpluggable`）：`try_clone_reader` 给的是
   一份主端副本，只要它还在，从端那一路就仍然有效；而 `portable-pty` 的写端 `Drop` 会往从端
   写 `0a 04`（换行 + `VEOF` —— 它把"关写端"当"发 EOF"）。于是那台设备**既不起读线程、
   也不取写端**，只持有唯一一份主端：`unplug()` 一关，从端的读写当场失败
   （实测 `BrokenPipe`，33 µs）。

### 验收命令的实际输出

```console
$ cargo nextest run --package akasha-serial
25 tests run: 25 passed, 0 skipped（新增 1 条：拔掉设备之后读端报错、且有那句话）

$ cargo nextest run --package akasha-pty
40 tests run: 40 passed（新增 1 条：`stream_error` 默认是 `None`）

$ cargo nextest run --package akasha --lib session
18 tests run: 18 passed（新增 1 条：原因出得了载体，原样交回 `Retired`）

$ just ready
✅ just ready 全绿（6/6）

$ pnpm build                # 前端类型检查不在 just ready 的门禁里（见 STATUS 的问题）
exit 0；产物 878.75 kB / gzip 241.58 kB（CSS 14.96 kB）

$ just test-e2e             # 退出码 0：第一段 25 个目标 / 30 个用例 + 第二段 1 个 = 26 / 31
── E2E: serial_device_gone ──   0.36 s
  假串口设备：/dev/pts/0（本进程持有主端，读主端=false）
  会话开起来了：/dev/pts/0
  已拔掉设备：关掉唯一一份主端，从端那一路的 read 当场报错
  通知行："「/dev/pts/0」已结束：串口设备已断开：/dev/pts/0（Broken pipe）"
  拔掉之后 sessions probe = {"live":1,"registered":1}（打开前 {"live":1,"registered":1}）
```

### 留下的两条

- 真设备（USB 转串口）被拔掉时内核报 EIO，而 PTY 主端关闭走的是 `POLLHUP` —— 两条在上游
  （`serialport` 的 `posix/poll.rs::wait_fd`）都进 `Err`，但**只有后者在本机实测过**。
- 前端类型检查不在 `just ready` 内（`pnpm build` 需手动执行）—— STATUS 已记着这条，
  本条不重复处理。