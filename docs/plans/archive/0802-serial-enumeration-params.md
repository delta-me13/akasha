# Plan 0802: 端口枚举与连接参数

- **关联**：ROADMAP 阶段 8 ·「端口枚举与连接参数（波特率/数据位/停止位/校验/流控）」
- **前置**：plan 0801（serial crate 与 feature 结构）
- **状态**：已完成（2026-09-15）

## 目标

枚举本机串口，并用完整参数打开：**波特率 / 数据位 / 停止位 / 校验 / 流控**。

判据（ROADMAP 原文）：**枚举在本机列出真实端口；参数错误时给出可读报错**。

展开之后这条判据由三件事组成：

1. **枚举**：`akasha_serial::ports()`。它把上游的设备类别映射成本 crate 的类型，输出
   **顺序稳定、同一路径只出现一次**（"可断言的输出"）；**空表是正常结果，不是失败**。
2. **参数**：五个参数经映射进 `serialport`，并能在真实 tty 上**回读**出内核留得住的那几个
   （见「前置检查」第 5 条）；取值不合法时在本层就报出**字段与取值**。
3. **降级**：枚举不可用时不硬失败 —— 上游拿不到 `libudev::Context` 时返回空表（第 3 条），
   而"手动指定路径"这条路本来就不经过枚举（plan 0801 的 `open()` 收一个显式路径）。

## 非目标

- **串口会话接入 app**（命令 / 事件 / 前端标签页 / 关闭与回收语义）。ROADMAP 阶段 8 的两个
  条目都是 crate 级的，没有一条覆盖它 —— 这是一处**已发现的范围缺口**，见「实施记录」。
- 串口协议的解析（只搬字节）、热插拔事件驱动的自动重连（先手动）。
- 库里的池行 ↔ 参数的**字段搬运**：本 plan 只给取值域的入口（`TryFrom<u8>`），
  搬字段的那一层随上一条（将来的 IPC plan）一起做。

## 前置检查（先验证后确定：读的是 4.10.1 的源码，量的是本机）

| 事实 | 结论 |
|---|---|
| Linux 上的枚举有两套实现：开着 libudev 时走 tty 子系统的 `Enumerator`（`enumerate.rs:613`），关闭它时扫 `/sys/class/tty/*` 并要求 `/dev/<名字>` 存在（`enumerate.rs:743`） | 一个 feature 开关决定"哪一套代码在生效"，所以两套都要真的执行一次（配方 `just serial-check`） |
| 实测（本机）：libudev 那套返回 **32 条** `/dev/ttyS0`…`/dev/ttyS31`，**32 条全在 `/sys/class/tty/` 里、32 条全都不是 `/dev` 下的节点**；sysfs 那套返回 **0 条** | **枚举出来的端口不保证能打开** —— 判据不得写成"每条都能开"。上游那句"打不开就跳过"的过滤只对 `serial8250` 驱动生效（`enumerate.rs:624`），本机没有触发 |
| `available_ports()` 在拿不到 `libudev::Context` 时返回**空表**（`enumerate.rs:615`），只有系统调用失败才 `Err` | 运行期缺库的表现是"列出 0 个端口"，所以"枚举失败"与"没有端口"必须由本层分成两种结果 |
| 参数是否真的生效可以回读：`SerialPort` 的 `baud_rate` / `stop_bits` / `flow_control` / `data_bits` / `parity` 读的都是同一份 termios（`tty.rs:605`–`739`） | 回读是这条判据在本机上唯一的装置 |
| 实测（PTY 从端；请求 115200 / 7 数据位 / 2 停止位 / 偶校验 / 软件流控）：回读 `115200` ✅、`Two` ✅、`Software` ✅、**`Eight` ❌**、**`None` ❌** | PTY 留得住**波特率 / 停止位 / 流控**；**数据位与校验位被归一化**（上游在 Linux 非 ppc 上走 `termios2` + `TCSETS2`，`termios.rs:126`）。所以断言只能覆盖前三项，后两项的证据在真机 |

## 步骤

1. `error.rs`：把 `SerialError` 从 `transport.rs` 搬出来（它现在跨"打开 / 枚举 / 参数"三处），
   新增 `Enumerate { source }` 与 `Settings { field, value }` 两档；文案里带**字段名与取值**。
2. `settings.rs`：`TryFrom<u8>` for `DataBits` / `StopBits`（值域 = serial 池的 `CHECK`：
   5..=8 / 1..=2，越界报 `Settings`）+ `SerialSettings::validate()`（空路径 → `NoPath`、
   `baud == 0` → `Settings`），`open()` 改为先 validate。
3. `enumerate.rs`：`PortInfo { path, kind }` + `PortKind`（Usb 五项 / Pci / Bluetooth / Unknown）
   + `ports()`：映射上游的类别、按路径排序、同一路径只留信息量最大的一档。
4. 单测：四种类别**各一条**（USB 那条查五项字段）、排序与去重、越界取值（9 / 3 / 0）的报错文案。
5. `transport.rs` 的 PTY 回读用例（白盒：直接读 `port` 的 getter）—— 只断言前三项，
   并把后两项的实测写进注释：不得让"四个参数都验过"这个印象成立。
6. `tests/enumeration.rs`：真实调用的三条不变量（顺序稳定、无重复、路径非空）+
   Linux 上的 sysfs 一致性（每条都在 `/sys/class/tty/<名字>` 里找得到）。
7. 配方 `just serial-check`（两种 feature 配置各执行一遍全部用例）+ 根 `justfile` 转发 +
   `docs/just.md` §2 一行。
8. `just ready`；删掉探针文件；文档同步与归档。

## 验收命令

预期输出写在各条之后。第 ②–③ 条在 `src-tauri/` 下执行。

```bash
# ① 两条枚举实现都真的执行了一次
just serial-check
# 预期：默认（libudev）与 --no-default-features（sysfs）两种配置下全部用例通过、0 个失败

# ② 枚举的真实调用（本机走 libudev）：32 条
cargo nextest run --package akasha-serial --test enumeration -- --nocapture
# 预期：2 tests run: 2 passed，并打印本机枚举到的条数

# ③ 负例自检（执行后还原）：删掉 ports() 的排序 → ① 必须红
#    （顺序稳定与去重都在这一点上，见步骤 3 / 4）
```

## 回滚

删掉 `enumerate.rs`、`tests/enumeration.rs` 与 `error.rs`（把 `SerialError` 搬回 `transport.rs`），
撤销 `settings.rs` 的两个新增，并把 `just serial-check` 那一套配方与 `docs/just.md` §2 的行一起撤掉。
本 plan 不加依赖（`Cargo.lock` 不动）、不改 app 的 manifest、不加 command / event。

## 实施记录

**2026-09-15（本 plan 全部落地）**

- 前置检查来自一个一次性探针（`tests/zz_probe_0802.rs`，验完即删）：
  - 默认配置（libudev）`available_ports()` → **32 条** `/dev/ttyS0`…`/dev/ttyS31`，
    且 **32 条全在 `/sys/class/tty/` 里、32 条全都不是 `/dev` 下的节点**；
  - `--no-default-features`（sysfs）→ **0 条**；
  - PTY 从端上请求 `115200 / 7 数据位 / 2 停止位 / 偶校验 / 软件流控` → 回读
    `115200` / **`Eight`** / `Two` / **`None`** / `Software`（数据位与校验位被归一化）。
- 验收命令 ①：`just serial-check` 退出码 **0**，两种配置各 **24 passed / 0 skipped**。
- 验收命令 ②：`cargo nextest run --package akasha-serial --test enumeration -- --nocapture`
  → `2 tests run: 2 passed`，并打印 **32 条**（`/dev/ttyS0`…`/dev/ttyS31`，字典序）。
- 验收命令 ③（负例自检）：把 `ports()` 里的 `sort_by` 拿掉后重新执行 `just serial-check`
  → **红**，红的是 `normalize()` 的三条单测（排序、去重、映射各一条）。
  ⚠️ **`tests/enumeration.rs` 那两条没有红** —— 本机上游返回的顺序恰好已经是字典序，
  所以"真实调用"那条断言在这台机器上无法失败。**只在真实机器上执行的断言可能是永真的**，
  这个差异已记进 `docs/STATUS.md`。
- `just ready` 六步全绿；`just test` **386 条**。
- **发现的范围缺口（问题 #151）**：阶段 8 的两条都在 crate 层 —— 串口接入 app（命令 / 事件 /
  界面 / 关闭与回收语义）在 ROADMAP 里没有任何条目。本 plan 不越出条目范围去做它。
