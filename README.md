# akasha

Tauri 2 桌面应用：可搬迁的、常驻托盘的多协议终端与文件传输客户端，
identifier `fans.cyrene.akasha-terminal`。

能力：本地 PTY / SSH / 串口三个终端后端，以及在此之上的 SFTP 文件传输、
SSH 端口转发（本地 / 远程 / 动态）、凭据池与系统托盘（窗口关闭默认收托盘，可配置为直接退出）。
完整能力清单、非目标与命名约定见 [`docs/scope.md`](./docs/scope.md)。

## 文档导航

| 需要的信息 | 文档 |
|---|---|
| 规则：可做什么、不可做什么、命令入口 | [`AGENTS.md`](./AGENTS.md)：唯一规范入口 |
| 下一步做什么 | [`ROADMAP.md`](./ROADMAP.md) |
| 进度与已知问题 | [`docs/STATUS.md`](./docs/STATUS.md) |
| 某项决定的依据 | [`docs/adr/`](./docs/adr/) |
| 某个工作项怎么做 | [`docs/plans/`](./docs/plans/) |
| 命令清单、工作流与排错 | [`docs/just.md`](./docs/just.md) |
| 受限沙箱下的构建与运行 | [`docs/agent-runner.md`](./docs/agent-runner.md) |
| 文档索引与文档纪律 | [`docs/README.md`](./docs/README.md) |

## 技术栈

| 层 | 实现 |
|---|---|
| 后端 | Rust + Tauri 2：PTY / 进程 / VT 状态；纯逻辑住在不依赖 Tauri 的域模块里 |
| 前端 | React 19 + Vite，终端由 xterm.js 渲染 |
| 持久化 | SQLCipher（`src-tauri/src/store`，[ADR-0002](./docs/adr/0002-secret-storage.md)） |

## 仓库布局

| 路径 | 内容 |
|---|---|
| `src/` | 前端源码（React 19 + Vite） |
| `src-tauri/` | Rust 包（`src-tauri/Cargo.toml`）：命令、事件与状态注入；纯逻辑在 `src-tauri/src/` 的域模块里 |
| `src-tauri/src/` | 域模块：`pty` / `ssh` / `store` / `serial` / `bw` / `session` / `config` / `tunnel`。每个域里纯逻辑零 Tauri 依赖，app 侧是 `ipc.rs`（或 `ipc/`） |
| `scripts/` | 开发脚本与结构护栏：`agent-runner.py`、ast-grep 规则与测例 |
| `docs/` | 规范、决策（ADR）、计划与状态 |
| `.github/workflows/ci.yml` | 唯一的 CI 工作流 |

仓库根不含 `Cargo.toml`（[ADR-0008](./docs/adr/0008-crates-to-modules.md) 之后 `src-tauri` 是唯一 package）；
crate 级命令经 `just` 执行，两个 justfile 的分工见 [`docs/just.md`](./docs/just.md) §3。

## 环境要求

| 项 | 说明 |
|---|---|
| 系统库 | webkit2gtk-4.1 等；`just syscheck` 校验 |
| 全局 CLI 工具 | 版本固定在 [`mise.toml`](./mise.toml)，`just tools` 一次装齐 |
| Rust 工具链 | rustup 的 `stable` |
| Agent 协作（可选） | 沙箱禁止 pty 设备与 `~/.cargo` 的写，`just dev` / `just test` 会失败；先按 [`docs/agent-runner.md`](./docs/agent-runner.md) 配置 policy 文件 |

## 常用命令

| 命令 | 作用 |
|---|---|
| `just dev` | 启动 app：前端 HMR，Rust 改动自动增量重编译并重启；启动一次即保持运行 |
| `just ready` | 提交前执行：全部质量门禁，即可执行的 DoD（定义见 [`AGENTS.md`](./AGENTS.md) §7） |
| `just check` / `just test` | 类型检查 / 单元测试（workspace 全成员） |
| `just watch` | bacon 秒级反馈循环，不启动 app |
| `just doctor` | 校验 Victauri 连接的是本应用，而非其它实例 |
| `just runner-status` / `just runner-start` | 沙箱受限时的出口：前者看授权清单，后者常驻启动（先配置 policy） |

全部配方的权威清单见 [`docs/just.md`](./docs/just.md) §2。
