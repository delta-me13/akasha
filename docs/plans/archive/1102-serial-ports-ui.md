# Plan 1102: 端口枚举与参数接进界面

- **关联**：ROADMAP 阶段 11 ·「端口枚举与参数接进界面（列端口 + 手动路径兜底 + 池行取值 + 可读报错）」
- **前置**：plan 1101（会话能打开、能回收）· plan 0802（`ports()` 与两个 `TryFrom` 已就位）
- **状态**：已完成（2026-09-15）

## 目标

界面上一屏看齐三条输入，点「打开」开一条会话：

| 输入 | 给出什么 |
|---|---|
| 本机枚举到的端口 | 设备路径（点一条填进表单） |
| 手输的设备路径 | 设备路径（**枚举不可用时的兜底**） |
| 池里的一条配置 | 路径 + 五个参数（点一行填满表单） |

四个参数（波特率 / 数据位 / 停止位 / 校验位 / 流控）可改；取值不合法时**当场**看到字段与取值。

## 非目标

- 打开 / 关闭 / 回收（plan 1101 已在）；设备消失（plan 1103）
- 热插拔事件驱动的刷新：先"打开面板时读一次"（同 `TunnelPanel`）
- 串口配置池的增删改查界面（与主机池那个缺口同一批，尚未规划）
- **端口的可用性探测**：不试着打开、不标记"可用 / 不可用"（见前置检查）

## 前置检查

实测（2026-09-15，本机）：

```console
$ cargo test --package akasha-serial --test … -- --nocapture   # 一次性探针，读完数即删
枚举到 32 条：/dev/ttyS0 … /dev/ttyS31（`kind` 全是 `Unknown`）
$ ls /dev/ttyS* | wc -l
0
```

三件必须记住的事：

1. ⚠️ **枚举结果不代表可用**（问题 #150）：udev 报 devnode 时**不检查那个节点在 `/dev` 下是否存在**
   —— 上面那两行读数就是这件事本身（列出 32 条，`/dev` 下一条都没有）。所以界面**不得**把列表
   当成"可用端口"，也不得据它禁用别的输入：三条输入**并列**，任一条都能开。
2. **顺序是路径的字典序**（`ttyS0`, `ttyS1`, `ttyS10`, …），不是自然序 —— 由 `akasha-serial`
   的 `normalize` 定，本条不改它。
3. **`PortInfo` / `PortKind` 不带 serde，也不带 specta**（`akasha-serial` 的依赖里没有它们）。
   过 IPC 的 DTO 落在 **app 侧**（`src-tauri/src/serial.rs`），映射写成穷尽 `match` ——
   `akasha-serial` 加一种 `PortKind` 时这里**编译不过**，而不是悄悄少一个分支。

## 步骤

1. **命令**：`src-tauri/src/serial.rs` 加 `serial_ports() -> Result<Vec<SerialPort>, SerialIpcError>`
   （**同步**：一次系统调用，没有握手），DTO = `SerialPort { path, kind }` + `SerialPortKind`
   （`Usb { vid, pid, serial, manufacturer, product }` / `Pci` / `Bluetooth` / `Unknown`）。
   登记进 `src-tauri/src/bindings.rs`，`just gen-types` 提交生成物差异。
2. **错误档**：`SerialIpcError` 加 `Enumerate { message }`（列不出来）。`SerialError::Enumerate`
   从 `Internal` 改挂到它 —— 用户在这一档的下一步动作是**手输路径**，而"内部状态不可用"
   会把方向带偏。
3. **前端读取层**：`src/ipc/serials.ts` 加 `listPorts()` + `SerialPortsUnavailable`；
   它**不碰库**（与池那条 `SerialsUnavailable` 分两个错误类：一个说"先解锁"，一个说"这里列不出来"）。
4. **表单**：`src/serial/SerialPicker.tsx` 从"点一行就开"改成"三条输入 → 一个「打开」按钮"：
   枚举列表（点击填路径）、六个字段（`data-serial-field`）、池行（点击填整行）、`data-serial-open`。
   打开失败渲染成一行；参数那一档同时带上 `data-settings-field`（字段名）与文本里的取值。
   数值字段是**文本框**：越界取值要能从界面产生，而 `<select>` 里写不出 9。
   「不是数字」这一档在前端拦（JSON 边界的必答项），**范围那一档仍归后端** ——
   前端不写第二份取值域。
5. **接线**：`src/App.tsx` 的 `openSerialTab` 收"六个字段 + 标题"（池行不再是唯一来源）；
   `src/ipc/mock.ts` 的 `serial_ports` 明确报错（dev-web 没有真枚举）。
6. **E2E**：新增 `src-tauri/tests/serial_ports_ui.rs`（判据见下），
   `serial_session` 改成"填表单 → 打开"（**同一份 PTY 假设备提进 `tests/support/`**，
   两个目标各造一份的写法会分叉）。两个目标都登记进 `justfile` 的 `E2E_TARGETS`。

## 验收命令

前置：`just dev` 不必开，`just test-e2e` 会自己起 app（已有则复用）。

```bash
# 1) 库内：DTO 映射穷尽 + 越界取值报字段与取值
cargo nextest run --package akasha --lib serial
# 预期：N passed（含 4 条新的映射用例），0 failed

# 2) 门禁（含生成物已提交这条）
just ready
# 预期：✅ just ready 全绿（6/6）

# 3) 端到端：新的 `serial_ports_ui` + 改过的 `serial_session`
just test-e2e
# 预期：exit 0，且 serial_ports_ui 打印——
#   枚举：后端 32 条 / 界面 32 条（两侧相等这一条断言在用例里）
#   手输路径：界面连上 PTY 从端 / 主端读到界面写出的字节
#   越界取值：那个标签页的报错行里出现 "data_bits = 9"（面已开出来、当场是出错态）
#   填不出值：面板自己拦下 "abc"，且没有开出新标签页
```

**判据怎么收口**（ROADMAP 原文：界面上看到本机枚举结果并据此（或手输路径）打开；
取值越界时显示字段与取值）：

| 判据 | 断言 |
|---|---|
| 界面看到本机枚举结果 | 界面上那个计数 == `invoke_command("serial_ports")` 的条数（**不写死 32**：CI 上没有这些设备） |
| 据此打开 | 列表非空时点第一条 → 路径字段变成它（CI 上列表为空则**显式跳过并写明原因**） |
| 手输路径打开 | 手输 PTY 从端的路径 → 「打开」→ 标签页说已连接，双向字节都到（**不依赖列表里有能用的条目**） |
| 取值越界显示字段与取值 | 数据位填 9 → 「打开」→ **那个标签页的报错行**里有 `data_bits = 9`（会话是面自己发起的，所以面已经开出来了），关闭它之后 `sessions` probe 回到打开前（**没有登记**） |
| 生成物已提交 | `just ready` 里的 `gen-types-check` |

## 回滚

三条命令各自独立，按需单独回退：

- `serial_ports`（+ DTO + `Enumerate`）不想要了：删 `src-tauri/src/serial.rs` 里那一段、
  从 `bindings.rs` 摘掉登记、`just gen-types` —— `open_serial_session` 不受影响
  （它收的是六个字段，从来不看枚举）。
- 表单：`SerialPicker` 退回 plan 1101 的版本即可（池行那条路一直在），
  枚举与手输随之消失。

## 实施记录

### 落地时定下的四件事

1. **枚举结果与"能不能打开"在呈现上分开**：那一块只有路径与硬件类别（`Usb` 的五项缺了就不显示），
   点一条**只把路径填进表单**，不试开、不标可用。库锁着时只有池那一块说"先解锁"（E2E 里
   库故意不解锁，端口照常列出 32 条）—— 三条输入并列这件事因此是可断言的。
2. **打开失败的那句话由标签页说**：会话是那个面自己发起的（`App.tsx` 只开一个面），所以数据位 9
   时**面已经开出来了**、当场是出错态。面板自己只说两档：读列表失败（`[data-ports-problem]` /
   `[data-pool-problem]`）与"这一栏填不了"（`[data-picker-problem]`，**不开面**）。
3. **前端只拦"寄不出去"**：空串 / 非数字 / 超出 `u32`·`u8` 的宽度 —— 那是 JSON 边界的必答项；
   范围（`5..=8`、`1..=2`、`baud > 0`）照原样交给后端，它报 `data_bits = 9`。
4. **`enumerate` 单独一档**（不再是 `internal`）：这一档用户的下一步动作是手输路径 ——
   而"内部状态不可用"会把方向带偏。加进契约后前端两处 `switch` 当场编译不过（TS2366）。

### 验收命令的实际输出

```console
$ cargo nextest run --package akasha --lib serial
7 passed（1101 的 5 条 + 端口映射 2 条）

$ just ready
✅ just ready 全绿（6/6）

$ pnpm build                # 前端类型检查不在 just ready 的门禁里（见 STATUS 的问题）
exit 0；产物 878.47 kB / gzip 241.44 kB（+8.68 kB / +1.96 kB）

$ just test-e2e             # 退出码 0：25 个目标 / 30 个用例
── E2E: serial_session ──   1.46 s
  界面：串口标签页已连接（打开前 1 个标签页）
  设备 → 界面：from-device-1101
  界面 → 设备：主端读到了 "to-device-1101\n"
  关标签页：sessions probe = {"live":1,"registered":1}（打开前 {"live":1,"registered":1}）
── E2E: serial_ports_ui ──   1.46 s
  枚举：后端 32 条：/dev/ttyS0 … /dev/ttyS31（kind 全是 unknown）
  界面：端口列表 32 条（与后端一致）；池那一块显示先解锁，端口照常
  据此填路径：/dev/ttyS0
  手输路径：from-device-1102 到了界面、"to-device-1102\n" 回到了设备
  越界取值：那个面的报错行里写着 data_bits = 9
  越界取值：关掉之后 sessions probe = {"live":1,"registered":1}（打开前 {"live":1,"registered":1}）
  填不出来的值：面板自己拦下了（没有开面）
```

### 留下的两条

- ⚠️ "点一条枚举结果 → 路径栏被填上"这一半在**枚举为空的机器上显式跳过**（CI 的 runner 就是
  这一类）：它在每一台机器上要么被验过、要么被说过，但没有跨机器的证据。
- 前端类型检查不在 `just ready` 内（`pnpm build` 需手动执行）—— STATUS 已记着这条，本条不重复处理。
