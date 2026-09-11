# akasha

Tauri 2 桌面终端应用：本地 PTY / SSH / serial 三个后端，外加 SFTP、凭据池、
SSH 端口转发（L/R/D）与常驻托盘。identifier `fans.cyrene.akasha-terminal`。

## 先读哪一份

| 想知道 | 看 |
|---|---|
| **规则**：什么能做什么不能做、命令入口 | [`AGENTS.md`](./AGENTS.md) —— 唯一规范入口 |
| **下一步做什么** | [`ROADMAP.md`](./ROADMAP.md) |
| **现在到哪了、有哪些坑** | [`docs/STATUS.md`](./docs/STATUS.md) |
| **某个决定为什么这样定** | [`docs/adr/`](./docs/adr/) |
| **某个工作项怎么做** | [`docs/plans/`](./docs/plans/) |

## 布局

前端与文档在仓库根（`src/`、`package.json`、`vite.config.ts`、`docs/`）；
**Rust 侧全部收在 `src-tauri/`** —— workspace root 就是 `src-tauri/Cargo.toml`，
纯逻辑 crate 在其 `src-tauri/crates/` 下
（[ADR-0004](./docs/adr/0004-rust-workspace-under-src-tauri.md)）。

仓库根**没有** `Cargo.toml`，所以在根目录直接跑 `cargo …` 会失败 ——
cargo 命令一律走 `just`（根 `justfile` 转发到 `src-tauri/justfile`，那里的 cwd 就是 manifest 所在处）。

## 常用命令

| 命令 | 干什么 |
|---|---|
| `just dev` | 起 app（**常驻一个就别关**）：前端 HMR + Rust 改动自动重编译并重启 |
| `just ready` | **提交前跑这一个**：全部质量门禁（可执行的 DoD） |
| `just check` / `just test` | 类型检查 / 单测（workspace 全成员） |
| `just watch` | bacon 秒级反馈循环，不启动 app |
| `just doctor` | 确认 Victauri 连的是本项目，而不是别的实例 |

完整清单（20 个配方 + 典型工作流 + 排错）见 [`docs/just.md`](./docs/just.md) §2；
环境前置（系统库、工具链）见 [`AGENTS.md`](./AGENTS.md) §10 与 `just syscheck`。
