# Plan 0803: Windows 上的串口枚举与它的证据（条件编译）

- **关联**：ROADMAP 阶段 8 ·「端口枚举与连接参数」的续作（阶段 8 第二条的 Windows 面）
- **前置**：plan 0801（libudev 的 Linux-only feature）/ plan 0802（枚举与参数）/ plan 0108（Windows 目标的类型检查）
- **状态**：进行中
- **影响面**：`src-tauri/src/serial/enumerate.rs`、`src-tauri/src/serial/ipc.rs`、
  `src/serial/SerialPicker.tsx`、`src-tauri/tests/windows_ports.rs`、
  `src-tauri/justfile`、`/.github/workflows/ci.yml`、`ROADMAP.md`、
  `docs/plans/README.md`、`docs/just.md`、`docs/STATUS.md`

## 目标

Windows 这条路**从来没有过读数**，缺口有三层，各自独立：

| 层 | 现状 | 判据 |
|---|---|---|
| 枚举的内容 | `serialport` 在 Windows 上把整台机器的 COM 端口报成 `SerialPortType::Unknown` —— 名字之外什么都没有 | 枚举结果里带得出**这台机器上的**描述 |
| 枚举的来源 | Windows 上没有任何 udev / libudev，`PortKind` 的四档在那边**只可能**是 `Unknown` | 走一条 Windows 专有的来源，且它在没有设备时给出**空表** |
| 证据 | 三条串口 E2E 全部用"一对 PTY 的从端"当设备，而 Windows 的 ConPTY 没有设备节点 ⇒ 三条**全平台跳过** | 有一条在 Windows 上**真的执行**的串口测试目标 |

判据（可粘贴执行的形状见「验收命令」）：Windows 上 `serial_ports` 报出带描述的端口；
枚举为空的机器上它给出空表而不是错误；这两条各有一条**真的执行过**的用例。

## 非目标

- **不在 Windows 上造"假串口设备"**：那需要一个内核侧的虚拟串口驱动（com0com 一类），
  属外部安装件。`fake_serial_skip_reason` 的那条理由因此**原样保留**，三条 PTY 用例继续跳过。
- **不碰 Linux / macOS 的枚举**：`serialport` 的 udev 分支逐字保留，设备描述只在 Windows 上取。
- **不做串口热插拔通知**（`ROADMAP.md` 的明确非目标）：枚举仍是"调一次、读一次快照"。
- **不手写 `extern "system"` 那四个注册表函数**：直接声明需要 `unsafe`，而 `AGENTS.md` §3.4
  把 `unsafe` 限定在存储模块一处（由 `no-unsafe-outside-store` 强制）——"读一次注册表"
  不是那条豁免的理由。改用 `winreg`（同一批 API 的安全包装，已在这条依赖树里，
  为 `tauri-build` → `embed-resource` 所用），它只加在 `[target.'cfg(windows)'.dependencies]`。
- **不链接 setupapi**：`SetupDiGetClassDevs` 能给同样的东西，代价是连接一个系统 DLL、
  并把设备信息集与我们的生命周期绑在一起。要读的只是几个字符串，注册表就够。

## 前置检查（本机实测，2026-09-20 起）

```bash
# 1. 上游在 Windows 上一条元数据都不报（枚举只有名字）
cd src-tauri && grep -n "SerialPortType::Unknown" \
  "${CARGO_HOME:-$HOME/.cargo}/registry/src/"*/serialport-4.10.1/src/windows/enumerate.rs
# 实测：com.rs 末尾 collect_ports 里那一行 —— Windows 的完整列表是 Unknown 的列表

# 2. 本机就能为 Windows 目标编译（MSVC 在 /f/VisualStudio/18/BuildTools，SDK 10.0.26100.0）
cd src-tauri && PATH="/f/VisualStudio/18/BuildTools/VC/Tools/MSVC/14.51.36231/bin/Hostx64/x64:$PATH" \
  cargo check --target x86_64-pc-windows-msvc --lib
# 实测：依赖全部编到 windows-sys / webview2-com-sys / wry，只有 openssl-src 挡住（它要原生 perl）

# 3. cfg(unix) 在 Windows 上是假的（这一条决定了"照搬 unix 分支"会红）
rustc --target x86_64-pc-windows-msvc probe.rs -o probe.exe && ./probe.exe
# 实测：cfg(unix) = FALSE / cfg(windows) = TRUE / target_os = windows
```

## 步骤（每步都能独立验证）

1. `enumerate.rs`：`PortKind` 加 `Windows { friendly_name: Option<String>, hardware_id: Option<String> }`
   —— 上游在这里给不出 USB / PCI / 蓝牙，新增一档而不是把 Windows 的端口塞进 `Unknown`。
2. `enumerate.rs`：`ports()` 在 `cfg(windows)` 上改为 `windows::describe(normalize(&raw))` ——
   拿 `available_ports()` 的名字去注册表问描述；`cfg(not(windows))` 上是**原来的两行，逐字不动**。
3. `enumerate.rs`：新增 `#[cfg(windows)] mod windows`。**两处来源、用 COM 名对齐**：
   `SERIALCOMM` 回答"系统认为有哪些串口"（只用它的**值**），设备项树
   `HKLM\SYSTEM\CurrentControlSet\Enum\<枚举器>\<设备>\<实例>` 回答"那是什么设备"
   （读 `PortName` / `FriendlyName` / `HardwareID`）。这一段走的是**别人机器的注册表**，
   所以**两个方向都有界**：深度上限 3（枚举器 → 设备 → 实例），一次枚举至多打开 4096 个键
   —— 深度只挡"树比预期深"，挡不住"树比预期宽"。额度用完就停，少拿几条描述不影响枚举结果
   （最坏退回"只有名字"）。
4. `ipc.rs`：`SerialPortKind` 加 `Windows` 变体，两个 `From` 的穷尽 `match` 各加一支。
5. `ipc.rs`：`SerialPortKind` 的 `Windows` 变体里两个字段**必须逐字段写 `#[serde(rename)]`**
   —— 枚举上的 `rename_all` 只改**变体名**，不改结构体变体里的字段名（与 `SerialEntry`
   那种普通结构体不同）。漏掉它的症状是生成物里叫 `friendly_name`、前端读 `friendlyName`，
   **类型都对得上而界面一直空着**。
6. `SerialPicker.tsx`：`describeKind` 按**属性存在与否**收窄（生成物是
   `{ usb: {...} } & { windows?: never }` 这类联合，不是字符串标签），
   `Windows` 档显示读到的描述，两项都缺时退到一句中性文案。
7. 新增 E2E 目标 `windows_ports`：不造任何假设备，只调 `serial_ports` 并按平台分支断言
   （Windows 上 `windows` 档必须带得出描述；其余平台上必须**没有** `windows` 这一档）。
8. `justfile` 的 `E2E_TARGETS` 加它，并新增 `serial-unit` 配方（`--lib -p akasha serial::`）：
   它**只构建 `akasha` 这一个包**，因此不需要 app、不需要串口设备，几秒钟就有读数，
   三平台的原生构建都可执行（⚠️ 它并不绕开 vendored OpenSSL —— `store` 是 `akasha` 的模块）。
   CI 的 E2E 格子在每个平台执行它。
9. 负例（`AGENTS.md` §6 的口径）：把步骤 2 的 `cfg(windows)` 改成 `cfg(target_os = "linux")`，
   确认 Windows 目标上 `describe` 不再被调用**且编译仍然通过** —— 即"枚举里没有描述"这件事
   不会自己变成编译错误。这是用例存在的理由：那种漏掉只能由**运行**发现。

## 验收命令

```bash
just ready                       # 期望：fmt / lint / test / deny / gen-types / docs 六步全绿

# 串口域在**本平台原生**上的单元读数（新配方；三平台都可执行）
just serial-unit                 # = cargo nextest run --lib -p akasha serial::
# 期望：30 条全过。Windows 上其中两条会真的走一遍注册表那条路
#       （registered_ports_are_com_names_or_nothing / describing_the_devices_never_yields_an_empty_kind）

# Windows 目标的类型检查 —— 本机可执行，MSVC 与 SDK 都在：
#   /f/VisualStudio/18/BuildTools/VC/Tools/MSVC/14.51.36231/bin/Hostx64/x64  （vswhere 可查）
#   /f/Windows Kits/10/Lib/10.0.26100.0
# ⚠️ vendored OpenSSL 要一个**原生** perl，而 PATH 上的只有 msys 的那个（问题 #156）。
#    本机用一份便携 Strawberry Perl 绕过（见实施记录）；它不进仓库、也不进 CI 的判据。
cd src-tauri && OPENSSL_SRC_PERL=<原生 perl> \
  cargo check --lib --target x86_64-pc-windows-msvc
# 期望：退出码 0 且无警告 —— 这是本 plan 之前从未有过的读数

# E2E 目标本身（需要 app；Windows 上它不再被 fake_serial_skip_reason 跳过）
VICTAURI_E2E=1 cargo test --test windows_ports -- --test-threads=1 --nocapture
# 期望（Windows）：windows 档的每一条都带得出描述（frontendName 或 hardwareId）
# 期望（Linux / macOS）：一条 windows 档都没有，且面板上每行的类别都有话说
```

## 回滚

删掉 `cfg(windows) mod windows`、把 `ports()` 的 `cfg` 分派去掉、从 `PortKind` 移除
`Windows` 变体即可回到"Windows 上只有名字"的状态 —— 三处是**同一次改动**，
所以不存在"只剩一半"的中间态。`serial_ports` / `open_serial_session` 的签名不变。

## 实施记录

**缺口（本机实测，2026-09-20）**：三层各自独立 —— 内容（上游在 Windows 上把每个端口都报成
`SerialPortType::Unknown`）、来源（Windows 上没有任何 udev）、证据（三条串口 E2E 全部要一对 PTY
当假设备，而 ConPTY 没有设备节点）。

**本机真的能把 Windows 目标构建出来**（这一条此前只有 CI 有读数）：

| 项 | 实测 |
|---|---|
| MSVC | `/f/VisualStudio/18/BuildTools/VC/Tools/MSVC/14.51.36231`（`vswhere` 可查） |
| Windows SDK | `/f/Windows Kits/10/Lib/10.0.26100.0` |
| `cargo check --lib --target x86_64-pc-windows-msvc` | 退出码 **0**、无警告（依赖一路构建到 `windows-sys` / `webview2-com-sys` / `wry`） |
| 挡在前面的 | vendored OpenSSL 要一个**原生** perl（问题 #156）。本机 PATH 上只有 msys 的那个，
| | 用一份便携 Strawberry Perl（`OPENSSL_SRC_PERL`）绕过；它**不进仓库、也不进 CI 判据** |

**两处问题都由工具当场拦下**（记下来是因为症状都指向别处）：

1. **手写注册表 `extern` 需要 `unsafe`** → `ast-grep scan` 的 `no-unsafe-outside-store` 报 6 处。
   `AGENTS.md` §3.4 把 `unsafe` 限定在存储模块一处，而"读一次注册表"不是那条豁免的理由。
   处置：改用 `winreg`（安全包装，已在这条依赖树里），本模块因此**一行 `unsafe` 都没有**。
2. **枚举上的 `rename_all` 不改结构体变体里的字段名** —— 生成物一度是 `friendly_name`，
   而前端按 `friendlyName` 读。两边的**类型都对得上**，所以 `tsc` 起初也没报错，
   症状只会是"界面上那一栏一直空着"。处置：逐字段 `#[serde(rename)]`，并在类型注释里写明。

**一处自查发现的边界**（步骤 3 定稿时）：`MAX_DEPTH` 只挡住"键树比预期深"，
**挡不住比预期宽** —— 一台插满设备的主机上 `Enum` 下的实例数没有上界。补上 `MAX_KEYS = 4096`
（一次枚举至多打开多少个键，额度用完就停，少拿几条描述不影响枚举结果）。这一层的定位是
**补充信息**，不该让 `serial_ports` 的速度取决于别人机器上插了多少设备。

**一条被自己的用例证伪的设计**（步骤 3 的第一版）：原本打算拿 `SERIALCOMM` 的**键名**
（`\Device\Serial<m>`）去拼设备项路径。微软的 `External Naming of COM Ports` 写明 `<m>` 是
Serial 驱动给设备的编号、**不是** COM 号，也不含枚举器与实例 —— 那条路拼出来的键一个都查不到。
单测 `a_driver_key_becomes_the_device_instance_path` 正是因此变红。处置：**用 COM 名对齐**
（`SERIALCOMM` 给端口集合、设备项树给描述），用例随之改成 `registered_ports_are_com_names_or_nothing`。

**验收读数**：

| 命令 | 结果 |
|---|---|
| `cargo test --lib -p akasha serial::` | **30 条全过**（含两条 Windows 专有：真的走了一遍注册表那条路） |
| `cargo test --workspace --lib` | **218 条全过** |
| `cargo check --lib --target x86_64-pc-windows-msvc` | 退出码 **0**、无警告 |
| `ast-grep scan` / `ast-grep test` | 干净 / **6 条规则全过** |
| `just deny-offline` | `bans ok, licenses ok, sources ok` |
| `just docs-check` | 通过 |
| `tsc --noEmit` | 干净 |

**负例（`AGENTS.md` §6）**：两条都做过，结论相反且都有用 ——

| 负例 | 结果 |
|---|---|
| 把 `cfg(windows)` 改成"永不成立" | **编译失败**（`cannot find function describe`）—— 编译期就拦得住 |
| 保留签名、让它在 Windows 上**什么都不做** | **编译通过**（只剩 dead_code 警告）—— ⚠️ 这一条编译期拦不住，
| | 只有**运行**能发现。这正是 `windows_ports` 存在的理由 |

**还缺的那一层（不在本 plan 内）**：本机**没有任何串口设备**（`SERIALCOMM` 键不存在），
所以"枚举出**真的**带描述的端口"这条只在逻辑上成立 —— 本机执行到的分支是**空表**。
有设备的 Windows 主机上才会走到"两条来源都非空、且用 COM 名匹配上"。

**`just ready` 在本机曾红在 `lint`，与本次改动无关**：`tests/known_hosts.rs:134` 与
`tests/pipelining.rs:771` 触发 `clippy::result_large_err`（`SshError` 至少 128 字节）。
已核对：**在干净的工作树上同样红**（`cargo clippy --test known_hosts --test pipelining` 复现），
是这些测试写就之后新版 clippy 收紧的结果。故本次只逐条执行了其余五步。
**那一关后来按问题 #166 修掉了**（把 `HostKeyChanged` 的 `recorded_in` 装箱，`SshError`
128 → 88 字节，并补一条直接量 `size_of` 的用例）—— 但仍**不是**一条与串口相关的改动。
本机 `just ready` 现在停在 `test`，原因与串口同样无关，见问题 #167 / #168。

