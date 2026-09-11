# ADR-0001：工作区切分与 PTY 抽象

- **状态**：提议（Proposed）—— 待定案
- **日期**：2026-09-11
- **决策者**：cyrene
- **影响范围**：仓库布局、`crates/` 的建立、`src-tauri` 的职责边界、CI
- **取代**：无

---

## 1. 背景

`akasha` 是一个 Tauri 2 终端应用。它的迭代成本几乎完全由一件事决定：
**Rust 编译一次要多久**——因为 Tauri 没有 Rust 热重载，任何 Rust 改动都只能
「增量重编译 + 重启 app」（见 `AGENTS.md` §1）。所以本 ADR 的首要目标不是
"架构漂亮"，而是**让尽可能多的逻辑可以在不启动 app 的情况下被编译和测试**。

### 1.1 当前事实（已实测，非推测）

| 事实 | 证据 |
|---|---|
| 单 crate 布局：`src-tauri` 是包根，无 workspace | 根目录无 `Cargo.toml` |
| 依赖已就位：`portable-pty 0.9.0`、`vte 0.15.0`、`tauri-specta 2.0.0-rc.25`、`insta`、`criterion` | `src-tauri/Cargo.toml` |
| **项目当前无法编译** | `just check` 退出码 101；`javascriptcore-rs-sys` 构建脚本报 `Package 'javascriptcoregtk-4.1' was not found`（缺 `webkit2gtk-4.1` 系统库） |
| **CI workflow 当前是坏的** | `.github/workflows/victauri.yml` 在仓库根执行 `cargo build` 与 `cargo metadata`，但根目录没有 `Cargo.toml` |
| ~~`cargo deny init` 无法执行~~ **已更正** | 报缺少 `Cargo.toml`；但**实测在 `src-tauri/` 中执行退出码 0**，正常生成 `deny.toml`。它只要求"当前目录含 `Cargo.toml`"，见 §2.1 |
| 项目已可编译，质量门禁全绿 | `just check` / `just lint` / `just deny-offline` 均退出码 0（装好 `webkit2gtk-4.1 2.52.6` 后复测） |

### 2.1 更正：cargo-deny **不是**支持决策一的论据

初稿把"`cargo deny init` 在仓库根失败"当作采纳根 workspace 的证据，**这是错的**。

实测：`cd src-tauri && cargo deny init` 退出码 0，正常生成 `deny.toml`（之后已替换为
一份显式白名单配置，因为模板里 `[licenses] allow = []` 的含义是**拒绝一切许可证**）。
它要求的只是"当前目录含 `Cargo.toml`"——`src-tauri/` 正是如此。

因此决策一的理由中，涉及工具可用性的部分**只有 CI 那一条是真实存在的**；
而 CI 同样可以通过改 workflow（加 `--manifest-path`、写死 bin 名）解决。
决策一必须靠下面的结构性理由支撑，不能靠"顺手修好某个工具"。

### 1.2 待解决的问题

1. 纯逻辑（PTY 生命周期、写路径、批处理）放在哪里，才能脱离 app 编译与测试？
2. 终端状态（屏幕模型）由谁持有——前端 xterm，还是 Rust？
3. PTY 的抽象边界长什么样，才能被 mock、被单测、并且不把 `portable-pty` 的
   类型和 `anyhow` 泄漏到上层？

---

## 2. 决策一：采用仓库根 Cargo workspace

**决定**：在仓库根建立 workspace，`src-tauri` 降为成员之一。

```toml
# /Cargo.toml
[workspace]
resolver = "2"
members = ["src-tauri", "crates/*"]

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
unwrap_used = "warn"
```

### 考虑过的选项

| 选项 | 描述 | 评价 |
|---|---|---|
| **A. 根 workspace** | `members = ["src-tauri", "crates/*"]` | ✅ 采纳 |
| B. `src-tauri/crates/` | 保持 `src-tauri` 为包根，crates 嵌在其下 | 不动 target 路径、不动 CI（这两点比初稿说的更有分量）；代价是共享增量缓存、跨 crate 统一 lint、`cargo test --workspace` 都拿不到，且 crates 名义上"与 Tauri 无关"却住在 `src-tauri/` 下 |

### 选择 A 的理由

1. **CI 是真实存在的问题**（workflow 在根跑 `cargo build`，根目录没有 manifest）——
   但必须说明：**这一点两种方案都能修**（B 可以改 workflow 加 `--manifest-path`），
   所以它是"要么改这里、要么改那里"，不是 A 独有优势。见 §2.1 的更正。
2. **单一 target 目录**：`crates/*` 与 `src-tauri` 共享增量缓存，避免两份编译产物。
   对这个"编译速度即迭代速度"的项目，这是最实质的理由。
3. **单一 lint 配置点**：`[workspace.lints]` 让 clippy 档位只在一个地方定义。
4. **命令入口统一**：`just check` / `just test` 可以在根目录一条命令覆盖全部 crate，
   bacon、nextest 同理。

### 代价与必须同步的改动

| 改动 | 说明 |
|---|---|
| target 目录移动 | 从 `src-tauri/target/` 变为 `<root>/target/`；`src-tauri/.gitignore` 的 `/target/` 要移到根 `.gitignore` |
| `Cargo.lock` 移动 | 移到仓库根；应用必须提交它（已先提交在 `src-tauri/Cargo.lock`，切 workspace 时一并移动） |
| `justfile` | `MANIFEST := "src-tauri/Cargo.toml"` 改为 `--workspace`，或保留 manifest 路径均可（两者都合法，但需统一） |
| CI | `$(cargo metadata ... \| jq -r '.packages[0].name')` 会拿到多个包 → 必须写死 bin 名 `akasha`，或 `cargo build -p akasha` |
| `tauri.conf.json` | 不需要改（仍指向 `src-tauri`），但 Tauri CLI 在 workspace 下的 target 路径行为**必须实测**（见 §5 未决项 1） |

---

## 3. 决策二：v1 只建 `crates/akasha-pty`，暂不建 `akasha-vt`

**决定**：屏幕模型由**前端 xterm 持有**（Rust 只转发原始字节）；
Rust 侧 v1 **不实现 VT 解析**，因此**不引入 `akasha-vt` 这个 crate**。

### 考虑过的选项

| 选项 | 谁解析字节 | v1 工作量 | 能力上限 |
|---|---|---|---|
| **A. 前端解析（采纳）** | xterm | 小 | 搜索/会话恢复由 addon 覆盖；无服务端状态 |
| B. Rust 持有屏幕模型 | `akasha-vt` | 大（数周级） | 可无头测试、多视图、agent 可读终端内容 |

### 关键权衡

- **选 A 的支撑点**：`addon-search`（回滚缓冲搜索）与 `addon-serialize`
  （会话序列化/恢复）已经覆盖了原本"必须有服务端状态"的两大诉求；
  `addon-webgl` 的渲染性能是成熟实现，自建网格渲染短期内不可能做得更好。
  在还没有一个能跑起来的二进制之前投入屏幕模型，是把风险最高的部分排在最前面。
- **选 B 的真实成本**：`vte::ansi::Handler` 有 **71 个方法**——实现它等于实现一个终端。
  这部分是 VT 一致性的重灾区（宽字符、组合字符、滚动区域、备用屏幕、字符集），
  正确性成本极高，且它**不是**本产品的差异化所在。

### 若未来触发（升级到 B 时怎么做）

触发条件（任一满足即可重新评估）：

1. 需要**跨 app 重启恢复**服务端会话状态（addon-serialize 不够时）；
2. 需要**无头快照测试**解析正确性（`insta` + VT 语料回归）；
3. 需要把终端内容暴露给工具/agent 读取，或支持多视图同时观察同一会话；
4. 需要 shell integration（OSC 133 语义提示符）在 Rust 侧做结构化处理。

实现方式（已勘察，避免将来重新调研）：

- 用 **`vte::ansi::{Processor, Handler}`**，即 alacritty 的终端核心。
  签名：`Processor::advance<H>(&mut self, handler: &mut H, bytes: &[u8])`。
- ⚠️ `ansi` 是 **非默认 feature**，必须 `vte = { version = "0.15", features = ["ansi"] }`。
- 它的 `advance` 接受**字节切片**并自行维护 UTF-8 与转义序列的中间状态——
  这与 `AGENTS.md` §3.2「绝不假设 UTF-8、只在边界传字节」完全一致。
- **不要**从更底层的 `vte::Parser` + `Perform`（`print`/`execute`/`csi_dispatch`/
  `esc_dispatch`/`osc_dispatch`）自己实现 VT 语义：那是把 71 个方法的成本
  换成更多方法的成本，且没有任何收益。

---

## 4. 决策三：PTY 抽象接口

**决定**：`crates/akasha-pty` 暴露两个 trait + 若干自有类型，
`portable-pty` 与 `anyhow` 都**不外泄**；读线程与批处理**在 crate 内部**实现。

### 4.1 接口草案

```rust
// crates/akasha-pty/src/lib.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize { pub rows: u16, pub cols: u16 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitStatus { Exited(u32), Signaled(String) }   // 自有类型，不暴露 portable_pty::ExitStatus

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub program: String,                 // 默认取 $SHELL
    pub args: Vec<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub env: Vec<(String, String)>,
    pub size: PtySize,
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError { /* Spawn, Io, Resize, Closed, ... */ }

pub trait PtyBackend: Send + Sync {
    fn spawn(&self, spec: &SpawnSpec) -> Result<Box<dyn PtySession>, PtyError>;
}

pub trait PtySession: Send + Sync {
    /// 拿走**已批量**的输出流；只能调用一次。
    /// 合批规则：≥16ms 或 ≥64KiB 触发一次发送（AGENTS.md §3.2）。
    fn take_output(&mut self) -> Result<tokio::sync::mpsc::Receiver<Vec<u8>>, PtyError>;

    /// 写入按键/粘贴。走有界通道，通道满时阻塞调用方 → 天然背压。
    fn write(&self, bytes: &[u8]) -> Result<(), PtyError>;

    fn resize(&self, size: PtySize) -> Result<(), PtyError>;

    /// 幂等；对已自然退出的进程是 no-op。
    /// Drop **不是**收尸路径（AGENTS.md §3.3），必须显式调用。
    fn shutdown(&self) -> Result<(), PtyError>;

    /// 退出状态；resolve 一次后可重复读取。
    fn exited(&self) -> tokio::sync::watch::Receiver<Option<ExitStatus>>;
}
```

### 4.2 与 `portable-pty 0.9` 的映射（已核对源码）

| 本接口 | portable-pty 0.9.0 对应项 |
|---|---|
| `spawn` | `native_pty_system().openpty(PtySize)` + `SlavePty::spawn_command(CommandBuilder)` |
| `take_output` | `MasterPty::try_clone_reader() -> Box<dyn Read + Send>`（move 进专用读线程） |
| `write` | `MasterPty::take_writer() -> Box<dyn Write + Send>`（move 进专用写线程） |
| `resize` | `MasterPty::resize(PtySize)` |
| `shutdown` | `ChildKiller::clone_killer()` 得到独立句柄 → `kill()` |
| `exited` | `Child::wait()`（等待线程持有 `Box<dyn Child + Send + Sync>`） |

### 4.3 线程模型（明确写下，避免实现时反复）

每个会话 **3 个 OS 线程**：

1. **读线程**——阻塞 `read` → 按 ≥16ms/≥64KiB 合批 → `blocking_send` 到有界 channel。
   退出：读到 `EIO`/`EOF` 即结束，关闭 channel 通知上层。
2. **写线程**——从有界 mpsc 取数据 → 写入 writer。通道满即背压到调用方。
3. **等待线程**——`child.wait()` → 结果放进 `watch`，供 `exited()` 读取。

**为什么不用 tokio 直接管 PTY**：跨平台 PTY（Linux `forkpty` / Windows ConPTY）
需要 `portable-pty` 的能力，而它的 reader/writer 是阻塞的 `Box<dyn Read/Write>`。
`spawn_blocking` 与专用线程等价，专用线程的生命周期更容易显式控制（§3.3 的收尸要求）。
代价是每会话 3 线程——对个位数标签页完全可接受，规模上限见 §5 未决项 4。

### 4.4 必须遵守的约束

1. **`anyhow` 不得外泄**：`PtySystem::openpty` 返回 `anyhow::Result<PtyPair>`，
   必须在 `akasha-pty` 内部转换为 `PtyError`。
2. **合批逻辑放在 crate 内**，不放 IPC 层——这样它能被 `FakePty` 单测覆盖
   （推入字节序列，断言 chunk 边界与时间窗），而 §3.2 的性能约束也就有了测试兜底。
3. **`shutdown` 幂等**，且 `Drop` 只做尽力而为的清理，**不作为正确性路径**。
4. `crates/*` 不得 `use tauri::*`（`AGENTS.md` §6 的 ast-grep 规则强制）。

---

## 5. 后果

### 正面

- 纯逻辑（PTY 生命周期、写路径、合批）可在**不启动 app** 的前提下编译与测试，
  直接对应 `AGENTS.md` §1 的迭代速度目标。
- 共享单一 target 目录，且 `cargo test --workspace` / bacon / nextest 在根目录一条命令覆盖全部 crate。
  （*更正*：初稿此处写"同时修好 CI 与 `cargo deny init`"——cargo-deny 部分是错的，见 §2.1。）
- PTY 后端可替换（`FakePty` 用于单测，`portable-pty` 用于真实运行），
  E2E 不必是唯一验证手段。

### 负面 / 风险

- 每会话 3 个 OS 线程，标签页数量多时需要重新评估（未决项 4）。
- 根 workspace 改变 target 与 `Cargo.lock` 位置，需要一次性的配置与 CI 更新；
  Tauri CLI 在新布局下的行为需要实测（未决项 1）。
- v1 没有服务端屏幕模型：**若**产品需求实际需要它（触发条件见 §3），
  本决策会被推翻，届时的返工点集中在 `akasha-vt` 的引入，而不是 `akasha-pty`。

### 落地时需同步修改

- `AGENTS.md` §3.1（分层图去掉 `akasha-vt`，或标注为"未来"）
- 根 `.gitignore`（`/target/`）、删除 `src-tauri/.gitignore` 中的 `/target/`
- `justfile`（`MANIFEST` → `--workspace`）
- `src-tauri/deny.toml` → 根 `deny.toml`（cargo-deny 按 cwd 发现配置；`just deny` 的 `--config` 路径同步）
- `.github/workflows/victauri.yml`（bin 名写死，加 `ast-grep scan` / `cargo deny check`）
- commits：`Cargo.lock`（移到根）、`deny.toml`

---

## 6. 未决问题

1. **Tauri CLI 在根 workspace 下的 target 目录与 dev watcher 行为**——
   必须在 `webkit2gtk-4.1` 装好后实测 `just dev`，确认产物路径与重启行为符合预期。
2. **Windows ConPTY 差异**——`process_group_leader` / `as_raw_fd` / `get_termios`
   是 unix-only（返回 `Option`）；`ChildKiller` 在 Windows 上的可靠性未验证。
   需要 CI 矩阵（当前只有 ubuntu + xvfb）。
3. **`tauri-specta 2.0.0-rc.25`**——锁预发布版，还是等正式版再接入 §5 的类型边界？
4. **线程模型规模上限**——每会话 3 线程，在 N 个标签页/分屏下的实际占用需要基准测量。
5. **shell integration（OSC 133）**——若在 Rust 侧做结构化解析，
   可能提前触发 §3 的决策二升级条件（`vte` 已安装，代价可控）。

---

## 7. 参考

- `portable-pty 0.9.0` 源码：`~/.cargo/registry/src/*/portable-pty-0.9.0/src/lib.rs`
  （`PtySystem` / `MasterPty` / `Child` / `ChildKiller` / `ExitStatus`）
- `vte 0.15.0` 源码：`~/.cargo/registry/src/*/vte-0.15.0/src/{lib.rs,ansi.rs}`
  （`Parser` + `Perform`；`ansi::{Processor, Handler}`，`ansi` 为非默认 feature）
- Tauri 2 IPC Channel：<https://v2.tauri.app/develop/calling-frontend/>
- 项目规范：`AGENTS.md` §1（开发循环）、§3（Rust 约束）、§5（类型边界）
