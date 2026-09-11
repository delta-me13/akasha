# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-11

## 一句话

地基阶段收尾：项目可编译、`just ready` 全绿、CI 已重写完成；**CI 的真实验证要等首次推送**；
ADR-0001 待拍板。

## 已验证为绿（命令 + 实际结果）

| 命令 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + docs-check） | 退出码 **0** |
| `just docs-check` | `✅ AGENTS.md §11 命令表与 justfile 同步` |
| `just check` | 退出码 **0** |
| `just lint`（clippy `-D warnings` + ast-grep scan） | 退出码 **0** |
| `just deny-offline`（licenses / bans / sources） | `bans ok, licenses ok, sources ok` |
| `just syscheck` | webkit2gtk-4.1 2.52.6 / javascriptcoregtk-4.1 2.52.6 / gtk+-3.0 3.24.52 / librsvg-2.0 2.62.3 |
| `.github/workflows/ci.yml` YAML | 用 `js-yaml` 解析通过；2 个 job，`e2e.needs = checks` |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。
> 首次 `just ready` 要编译测试目标（nextest），可能超过 60 秒，别误判为卡死。

## 待验证（本地跑不了）

- **CI 是否真能变绿** —— 需要首次 push。本地只能校验 YAML 合法性、命令存在性、
  依赖/action 版本可获取性；runner 环境（apt 包可用性、xvfb 起 app）必须在 CI 上见分晓。
- **`just dev` 在根 workspace 布局下能否起窗口** —— ADR-0001 决策一的验收项，
  见 `docs/plans/0001` 第 4 条。当前布局下尚未跑过 `tauri dev`。

## 预跑基线（2026-09-11 实测，当前布局）

ADR-0001 决策一（根 workspace 迁移）**迁移前**的实测记录 —— 迁移后必须拿它逐项对比。

| 项 | 实测结果 |
|---|---|
| 二进制落点 | `src-tauri/target/debug/akasha`（日志 `Running target/debug/akasha`，cwd = `src-tauri`），393 MB |
| 冷编译 | **47.53s** |
| 增量重编译 | **6.09s** |
| dev server | Vite 就绪于 `http://localhost:1420`（实测 HTTP 200） |
| Rust 监听范围 | CLI 打印 `Watching /home/lycurgus/akasha/src-tauri for changes` |
| app 数据目录 | `~/.local/share/fans.cyrene.akasha-terminal/`（CacheStorage / hsts-storage.sqlite） |
| Victauri | 连上，`identifier = fans.cyrene.akasha-terminal`，35 个工具，端口 7373 |
| **IPC 端到端** | `invoke('greet', {name:'preflight'})` → `Hello, preflight! You've been greeted from Rust!` ✅ |
| 前端渲染 | `dom_snapshot` 拿到完整模板 UI（heading / link / form / textbox / button） |
| 前提条件 | **需要能写 `$HOME`**；受限环境下会在 `Failed to setup app: 只读文件系统 (os error 30)` panic |

> 当前有一个 `just dev` 正在运行（我以完整权限启动的）。停止：结束该后台作业即可；
> 注意它占用 1420 端口，另起一个 `tauri dev` 前先停掉它。

**预跑得出的、迁移后必须复测的点**（已写入 `docs/plans/0001`）：
CLI 打印的监听路径是 `src-tauri`。根 workspace 迁移后 `crates/*` 落在 `src-tauri` 之外，
**必须确认改 `crates/` 下的文件仍会触发重编译与重启** —— 否则开发循环会静默失效，
而 `cargo check` 完全看不出来。

## 进行中 / 下一步

- [ ] **ADR-0001 定案**：待 cyrene 拍板 §3 决策二（v1 是否建 `akasha-vt`）与 §6 的未决项
- [ ] **落地决策一**（根 workspace）：步骤与验收见 [`docs/plans/0001`](./plans/0001-root-workspace.md)

## 结构现状（容易找错地方）

- 命令入口分两处：项目级在根 `justfile`，crate 级在 `src-tauri/justfile`。
  根只**转发**，命令体只有一处。对照表见 `AGENTS.md` §11，由 `just docs-check` 强制同步。
- CI 只有一个文件 `.github/workflows/ci.yml`（原 `victauri.yml` 已删除并合并进来）。

## 踩过的坑（避免重复踩）

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency** ——
   它不会传递给你；缺了 `cargo check` 退出码 101。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**，
   直接启用会让 check 全红。另外它**不需要在仓库根执行** —— 只要求当前目录含 `Cargo.toml`。
3. **just 的变量写 `$p`，不是 Make 的 `$$p`**（后者会展开成 PID）。
4. **just 用 justfile 所在目录作为配方工作目录** —— 这是把 crate 级命令搬进
   `src-tauri/justfile` 后可以彻底去掉 `--manifest-path` 的原因。
5. **系统库缺失只会在 cargo 的构建脚本阶段暴露**（`javascriptcore-rs-sys`）。
   本机是 **CachyOS（Arch 系）**，用 `pacman -S webkit2gtk-4.1`，不是 apt。
6. **Tauri 没有 Rust 热重载，Victauri 也不提供**。迭代速度靠"逻辑下沉到 `crates/` +
   bacon 独立循环"拿回来，不靠热重载。
7. **just 的 shebang 配方需要可写的 runtime dir**，在受限环境会失败 —— 用普通配方。
8. **仓库根没有 `Cargo.toml`** → 一切没显式指定 manifest 的 cargo 命令在根目录失败：
   `cargo build`、`cargo metadata`（原 CI 因此坏掉）、`cargo fmt --all`（退出码 141）。
   详见 `docs/adr/0001` §2.2。若采纳决策一，这类问题会整体消失。
9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** ——
   `fmt-check` 会红。跑一次 `just fmt` 规范化即可（已做）。
10. **CI 里 `libappindicator3-dev` 在 ubuntu-latest 上已不存在**，要用
    Tauri 官方列表里的 `libayatana-appindicator3-dev`（还漏了 `libxdo-dev`）。

## 环境

CachyOS（Arch 系）/ rustc 1.98.1 / cargo 1.98.1 / node 26.8.2 / pnpm 12.3.4 / mise 2026.9.1
