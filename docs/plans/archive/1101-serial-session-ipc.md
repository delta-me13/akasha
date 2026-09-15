# Plan 1101: 串口 `Session` 接入 app

- **关联**：ROADMAP 阶段 11 ·「串口 `Session` 接入 app（命令 + 注册表 + 标签页 + 关闭与回收）」
- **前置**：plan 0801 / 0802（`akasha-serial` 的载体、`ports()` 与参数校验已就位）
- **状态**：已完成（2026-09-15）

## 目标

`akasha-serial` 接到 app 上：一个串口 `Session` 由命令打开、字节双向流动、关闭标签页即丢掉它自己。
串口与 local / ssh 是**同一种东西**（`Transport` 装进 `Sessions` 的 `live` 表），所以这一段
一行新的会话模型都不该有 —— 变的只有"载体从哪来"。

## 非目标

- **端口枚举与手动输入路径**（plan 1102）：本条只走"从池里挑一行配置"这一条输入路径
  （池行的六个字段经 IPC 原样交给命令）。1102 在同一个命令上加"列端口 / 手输路径 / 参数可编辑"。
- **设备消失时的表现**（plan 1103）：串口被拔掉时读端报错、会话结束这条路径在本条里
  **已经自然成立**（读流结束 → `retire` → `session_ended`），但"原因可读"是 1103 的活。
- 串口热插拔事件驱动的自动重连（`scope.md` §10，`later`）；串口协议解析（只搬字节）。
- 串口配置池的增删改查界面（与主机池的界面缺口同一批，尚未规划）。

## 前置检查（动手前核对过的四个形状）

1. **串口在 `Sessions` 里占哪张表**：`live`（与 local / SSH 同一张），**不新增第四张表** ——
   它是一条 `Transport`，不是隧道 / SFTP 那种"自持连接的实体"。
2. **没有本地进程时走哪条路**：`SerialTransport::session_leader()` 是 `None`（plan 0801 已定），
   于是 `Sessions::register` 里 `leader = None`、`watch(None)` / `forget(None)` 都是空操作
   —— 与 SSH **同一形状**（ADR-0003 D4）。看门狗在这条路径上无可回收之物，这是对的：
   串口没有子进程。
3. **`resize` 必须不再被当成失败**：串口的能力位是 `Capabilities::NONE`，`resize` 答
   `Unsupported`；而前端的 `attachTerminal` 每次 `fit()` 都发一次 `resize_session`，
   它现在把错误当失败报给用户（`onStatus("error")`）—— 于是串口标签页会**一打开就是红的**。
   处置：`Sessions::resize` **先看能力位**（这正是 `Transport::resize` 的契约要求的），
   不具备该能力的载体上它是**空操作**。这样前端不必为串口写一条特例，
   而"谁能 resize"仍然只有载体自己一个来源。
4. **E2E 的设备从哪来**：本机 `/dev` 下没有任何串口设备（问题 #150），所以用
   `portable-pty` 造一对 PTY、把**从端的路径**当设备（与 plan 0801 的单测同一条路）。
   测试进程持有主端：写主端 → 从端 → app 读到；app 写 → 从端 → 主端 → 测试进程读到。

## 步骤（每步都能独立验证）

1. **`akasha-serial` 进 app 的依赖**（`src-tauri/Cargo.toml`，带 `version` + `path`，问题 #30）。
   它同时让 `cargo-deny` 的图里多出这一支（那正是想要的：`serialport` 与 libudev 从此在门禁射程内）。
2. **`src-tauri/src/serial.rs`**：
   - `SerialEntry`（池行的过 IPC 表示：`id` / `name` / `port` / `baud` / `dataBits` / `stopBits` /
     `parity` / `flow`）+ `vault_serials`（只读，按名字排序 —— 与 `vault_hosts` 同形）；
   - `SerialParity` / `SerialFlow` 两个 IPC 枚举，**穷尽 `match`** 映射到
     `akasha_store::pools::serial` 与 `akasha_serial` 两侧（池加一种取值时这里编译不过）；
   - `open_serial_session`（**参数显式**，不是池行 id）：六个字段 → `SerialSettings`
     （`DataBits::try_from` / `StopBits::try_from`，越界报**字段与取值**）→
     `SerialTransport::open` → `session::open_terminal`（**与 local / SSH 同一条尾巴**）。
     它是**同步**命令：打开一个本地设备没有握手，参数在碰设备之前已经校验过。
   - `SerialIpcError`：`settings { field, value }`（要改的是那个字段）/ `open { path, message }`
     （要去看的是那个设备）/ `internal`。**没有 `locked`** —— 打开一个串口不碰库。
3. **前端**：`SessionTarget` 加 `{ kind: "serial", settings }`、`TabKind` 加 `"serial"` 并进
   `CLOSABLE`、`src/serial/SerialPicker.tsx`（列池里的行，选一行交给 `onConnect`）、
   `src/ipc/serials.ts` 与 `session.ts` 的 `openSerialTerminalSession`、`mock.ts` 两个分支
   **明确报错**（不是返回空列表）。串口不推迟微任务：它不弹任何提示（同 local）。
4. **E2E `serial_session`** 登记进 `src-tauri/justfile` 的 `E2E_TARGETS`（清单末尾 ——
   它只用 serials 池，不与前面那几条共享行）。

## 验收命令（可直接粘贴执行，并写出预期输出）

```bash
# 1. 类型与门禁（含生成物是否已提交）
just check && just gen-types-check

# 2. 库内：池行的六个字段 → 参数（纯函数那一层）
cargo nextest run --package akasha --lib          # 期望：全过；越界取值报字段与取值

# 3. 端到端（自包含：没有 app 就自己起一套，跑完收掉）
just test-e2e                                       # 期望：serial_session 通过，退出码 0
```

E2E 的判据逐条（ROADMAP 原文 =「真实 app 上打开一个串口会话并双向传字节；
关闭标签页后 `live` / `registered` 归零」）：

| 判据 | 断言 |
|---|---|
| 从界面打开 | 点 `.tab-new-serial` → 选 `[data-serial-id]` → 标签页出现且状态"已连接" |
| **从设备到界面** | 测试往 PTY 主端写一串 → `window.__akashaTerminal.screenText` 里出现它 |
| **从界面到设备** | 往标签页里敲一行 → 测试从主端**读到同一串**（两串不同，避免被 PTY 的 ECHO 混淆） |
| 关闭标签页即回收 | 点 `.tab-close` → 标签页数回到打开前；`app_state { probe: "sessions" }` 的 `live` / `registered` 回到打开前的读数且**两者相等** |

⚠️ **"归零"的口径**：app 一启动就有那个本地终端标签页，所以 E2E 读的是
"关闭串口标签页之后回到**打开前**那个读数"（与 `ssh_session` 同一条口径），
而不是字面的 0。串口没有本地进程，这条判据**只能看注册表**（同 ADR-0003 D4）。

## 回滚

`git revert` 本条的提交即可：新增的是一个 crate 依赖、一个模块、两条命令、一个前端面板，
没有改库格式、没有迁移、没有动既有命令的语义（`Sessions::resize` 那一处除外 ——
它把 `Unsupported` 从"错误"改成"空操作"，回滚时跟着回去）。

## 实施记录

### 三个落点（动手时的判断，实施后仍然成立）

1. **参数显式，不是池行 id**（骨架里"池行 → `SerialSettings` 的字段搬运落在哪一层"的答案）：
   命令收一个 `SerialParams` 结构体（六个字段），映射与校验在 app 侧（`src-tauri/src/serial.rs`）。
   两个立刻兑现的好处：plan 1102 的"手输路径 + 参数可编辑"**不必再开第二条命令**；
   "打开一个设备"这条路径**不碰库**，于是它没有 `locked` 那一档，而 `vault_serials`
   （列池里有哪些）仍然需要解锁。
2. **`resize` 的 `Unsupported` 不再算失败**（前置检查第 3 条）：`Sessions::resize` 先看能力位。
   不这么改，串口标签页**一打开就是红的**（前端每次 `fit()` 都发一次 `resize_session`）；
   而 `session::tests::resize_reaches_the_transport` 仍然通过（有该能力的载体照旧收到）。
3. **E2E 的设备是测试进程自己造的 PTY 从端**（本机没有串口硬件，问题 #150）。
   两串文本故意不同（`from-device-1101` / `to-device-1101`），所以"主端读到的那一串只可能来自
   app"不依赖行规程的行为。

### ⚠️ 一处实测出来的问题：`serialport` 默认**独占**打开 + React StrictMode 挂载两次

`serialport` 的 `TTYPort::open` 会打 `TIOCEXCL` 与独占 `flock`（上游 `posix/tty.rs`，
错误映射里 `EBUSY` 就是"这个终端已经被别人按独占打开了"）。而 dev 构建的 StrictMode 把 effect
走两遍（挂载 → 清理 → 挂载，**同一次 commit 里同步完成**）—— 本地终端那条路是无害的
（多启动一个 shell 再立刻回收它），所以此前没有人需要处理它；串口这边第一遍那次**真的把设备打开了**，
于是第二遍当场收到 `EBUSY`：

   终端不可用：串口打不开：/dev/pts/0（Device or resource busy）

处置：把"推迟一个微任务再开"**从 SSH 那条扩到串口**（`src/terminal/attach.ts`）——
被丢弃的那次挂载因此根本不发命令（只有本地终端保持同步发出去）。
**不做"关闭独占"**：真实设备上两个进程抢一个串口本来就不该成功，那条错误该留着。

### 验收（实测）

| 命令 | 结果 |
|---|---|
| `cargo nextest run --package akasha --lib serial` | **5 passed**：池值 → 参数的全量映射 / 越界取值报字段与取值 / 空路径报出用户给的那一串 / 不存在的设备报出它试过的路径 / 六个字段的搬运 |
| `cargo nextest run --package akasha --lib` | **53 passed** |
| `just test`（workspace） | **392 passed, 0 skipped** |
| `just test-e2e` | 退出码 **0**：24 个目标全过（第一段 23 + 第二段 1），耗时约 2 分钟 |
| `pnpm build` | 退出码 0（`tsc` + vite） |

`serial_session` 实测（**1.00 s**）：

   设备：/dev/pts/0（本进程持有主端）
   串口池：[{"baud":115200,"dataBits":8,"flow":"none","id":1,"name":"e2e-serial-port","parity":"none","port":"/dev/pts/0","stopBits":1}]
   界面：串口标签页已连接（打开前 1 个标签页）
   设备 → 界面：from-device-1101
   界面 → 设备：主端读到了 "to-device-1101\n"
   关标签页：sessions probe = {"live":1,"registered":1}（打开前 {"live":1,"registered":1}）
