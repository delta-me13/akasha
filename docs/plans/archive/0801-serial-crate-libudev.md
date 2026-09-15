# Plan 0801: `src-tauri/crates/akasha-serial`（`libudev` 走 Linux-only feature）

- **关联**：ROADMAP 阶段 8 ·「`src-tauri/crates/akasha-serial`，`libudev` 走 **Linux-only cargo feature**」
- **前置**：plan 0105（`Transport` trait 已定形态）
- **状态**：已完成（2026-09-15）

## 目标

serial 后端 crate：`Transport` 的串口实现，**零 Tauri 依赖**。

判据（ROADMAP 原文）：**Windows / macOS 构建不链接 libudev**。

展开之后这条判据由三件事组成：

1. 上游把 `libudev` 声明在 `[target.'cfg(all(target_os = "linux", not(target_env = "musl")))'.dependencies]`
   里，所以"链不链它"完全由**那个目标的依赖图**决定 —— 在 Linux 上换一个 `--target` 就能核对
   （新增配方 `just libudev-check`）；
2. 本 crate 在两个非 Linux 目标上仍然能编译（探针：`serialport` 带着 `libudev` feature 在
   三个目标上都编译通过）；
3. CI 的 `checks-other` 在 Windows / macOS 上**原生**执行 `just check`（= `cargo check --workspace`）
   —— 那两个平台真的编译本 crate。⚠️ **当前被问题 #149 挡住**（见「实施记录」）。

## 非目标

- **端口枚举**（列出本机串口）与**参数的用户输入校验**：plan 0802。本 plan 只保证"按一个显式
  路径打开"可用 —— 路径是手动指定的第一类输入，因此枚举不可用时这条路照常可用。
- Android 的 USB Host + JNI（Blater`，阶段 10 评估）。
- 串口协议栈（只搬字节）与热插拔自动重连。

## 前置检查（先验证后确定：读的是 4.10.1 的 manifest 与源码）

| 事实 | 结论 |
|---|---|
| `default = ["libudev"]`，而 `libudev` 这个 optional 依赖声明在 `[target.'cfg(all(target_os = "linux", not(target_env = "musl")))'.dependencies]` | **平台差异由上游的 target 段承担**：Windows / macOS 上打开这个 feature 激活的是一个"不存在的依赖"，什么都不编译。本 crate 不必自己写第二套 feature 结构 |
| Linux 上关闭 `libudev` 仍有枚举实现（`src/posix/enumerate.rs` 里 `#[cfg(target_os = "linux")]` 那一支直接读 sysfs） | 发行版缺 libudev 时**不是硬失败**：`default-features = false` 仍能枚举 —— `scope.md` §8 风险 5 的降级要求因此有更强的答案（不必只靠手动路径） |
| 读超时的形态：POSIX 走 `poll`（`src/posix/poll.rs`，超时给 `io::ErrorKind::TimedOut`）；Windows 走 `ReadFile`，读回 0 字节后同样给 `TimedOut`（`src/windows/com.rs`） | 读循环必须把 `TimedOut` 当"暂时没有数据"，**不得**当作流结束 —— 否则设备一安静，会话就被判成结束 |
| `available_ports()` 拿不到 `libudev::Context` 时返回**空表**而不是错误 | 运行期缺库的表现是"列出 0 个端口"，这也支持"手动指定路径"作为兜底 |
| `portable-pty` 的 `MasterPty::tty_name()`（unix）给出从端的设备名 | 单测可以造一对 PTY、把从端当串口打开：**不需要任何硬件**就能走通 `open()` 这条路 |

## 步骤

1. `cargo new --lib src-tauri/crates/akasha-serial` —— 自动落进 `members = ["crates/*"]` 的 glob。
2. `Cargo.toml`：依赖 `akasha-pty`（`Transport` 门面）与 `serialport`
   （**`default-features = false`**，即不直接吃上游的默认 feature）；自有 feature
   `libudev = ["serialport/libudev"]` 且 `default = ["libudev"]`。
3. `settings.rs`：`SerialSettings` 与四个参数枚举（数据位 / 停止位 / 校验 / 流控），
   取值域与 serial 配置池的 `CHECK` 对齐（数据位 5..=8、停止位 1..=2，校验位与流控各三值）；
   枚举 → `serialport` 类型的映射关在 crate 内。池行 ↔ 本类型的映射留给 plan 0802。
4. `transport.rs`：`SerialTransport::open` + `Transport` 实现 + 读端包装 `SerialReader`。
   能力位是 `Capabilities::NONE`（没有窗口尺寸、没有结局、没有本地进程）；
   `shutdown` 幂等且只立停止标志（串口没有子进程要回收）。
5. `tests/pty_roundtrip.rs`：真实 tty 上的往返（写出去 / 读回来 / 收尾让读端结束 / 收尾幂等 /
   写端已关），并断言能力位。
6. `just libudev-check`（按目标核对依赖图，加进 `docs/just.md` §2 与根 `justfile` 的转发）。

## 验收命令

预期输出写在各条之后。第 ②–④ 条在 `src-tauri/` 下执行（它是 workspace 根；根 `justfile` 的转发
配方同此）。

```bash
# ① 两侧依赖图
just libudev-check
# 预期：
# ✅ Linux：依赖图里有 libudev
# ✅ x86_64-pc-windows-msvc：依赖图里没有 libudev
# ✅ aarch64-apple-darwin：依赖图里没有 libudev

# ② 本 crate 在 Linux 上编译（含 tests）
cargo check --package akasha-serial --all-targets
# 预期：Finished

# ③ 关闭 feature 仍能编译（发行版缺 libudev 的降级路径）
cargo check --package akasha-serial --no-default-features
# 预期：Finished

# ④ 单测，含真实 PTY 上的往返
cargo nextest run --package akasha-serial
# 预期：13 tests run: 13 passed

# ⑤ 负例自检 —— 证明 ① 的两种判据都不是永真式（执行后还原）
#   A：把 default = ["libudev"] 改成 default = []      → Linux 那一行必须报 ❌、退出码 1
#   B：在 [dependencies] 里无条件加 libudev = "0.3"     → 两个非 Linux 目标必须报 ❌、退出码 1
```

## 回滚

删掉 `src-tauri/crates/akasha-serial/` 与 `Cargo.lock` 里随之出现的条目（`serialport` /
libudev / libudev-sys / unescaper / nix 0.26 / scopeguard / cfg-if / bitflags 那一组），
再把 `just libudev-check` 这套配方与 `docs/just.md` §2 的那一行一起撤掉。
本 plan 不改 app 的 `Cargo.toml`、不加 command / event，所以没有需要一并撤销的契约。

## 实施记录

**2026-09-15（本 plan 全部落地）**

- crate 与 feature 结构按步骤 1–4 落地；`just ready` 六步全绿（含 `deny-offline`：
  `serialport` 是 MPL-2.0，`libudev` / `libudev-sys` / `unescaper` / `nix` 都在 `deny.toml`
  的放行列表内）。
- 步骤 5 的用例：**13 条全过**（`akasha-serial` 的 9 条单测 + `pty_roundtrip` 的 4 条）。
  ⚠️ PTY 那 4 条**没有走跳过分支** —— 带 `--nocapture` 重新执行一次确认过：
  一次"全部通过"区分不了"用例在工作"与"用例在第一行就 return 了"。
- 步骤 6 的配方用**两条负例**验过（见「验收命令」⑤）：A 让 Linux 那一行报 ❌，B 让两个非 Linux
  目标报 ❌，还原之后回到三条 ✅。这一步不是形式：**探针自检时曾经假通过一次** ——
  `cargo tree --target aarch64-apple-darwin` 第一次执行时因 `~/.cargo` 只读而**解析失败**
  （问题 #105），而"没有 libudev"与"没有输出"在 `grep` 眼里一样，于是配方报了一次 ✅。
  从那以后每条分支都先看 `cargo tree` 自己的退出码 —— 这正是配方里那两条注释的来由。
- **发现（问题 #149）**：`cargo check --package akasha-serial --target x86_64-pc-windows-msvc`
  **编译不过**，但坏在 `akasha-pty` 而不是本 crate —— 它用了 `rustix::process`
  （上游 `#[cfg(not(windows))]`）而没有 `cfg` 守卫，于是 Windows 目标一开始就红。
  因此本 plan 对"Windows / macOS 构建"这一侧的**原生编译**证据来自 CI 的 `checks-other`
  （待 #149 修复）。本地证据是两条：①的依赖图，外加一次**一次性探针** —— 用一个临时
  workspace 成员让 `serialport` 带 `libudev` feature 在三个目标上都编译一次（Windows / macOS
  上都是 Finished），验完即删（`crates/` 下已无残留、`Cargo.lock` 里也没有它的条目）。修 #149 属于别的工作项：Windows 上"回收整个会话"
  没有 POSIX 进程组语义，是计划级的活，不混进本 plan。
