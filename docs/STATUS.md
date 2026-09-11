# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-11

## 一句话

地基阶段：项目可编译，`just ready` 全绿；**CI 仍是坏的**；ADR-0001 待拍板。

## 已验证为绿（命令 + 实际结果）

| 命令 | 结果 |
|---|---|
| **`just ready`**（fmt-check + lint + test + deny-offline） | 退出码 **0** |
| `just check` | 退出码 **0** |
| `just lint`（clippy `-D warnings` + `ast-grep scan`） | 退出码 **0** |
| `just test`（cargo-nextest） | 通过（smoke/integration 在无 `VICTAURI_E2E` 时自行跳过） |
| `just deny-offline`（licenses / bans / sources） | `bans ok, licenses ok, sources ok` |
| `just syscheck` | webkit2gtk-4.1 2.52.6 / javascriptcoregtk-4.1 2.52.6 / gtk+-3.0 3.24.52 / librsvg-2.0 2.62.3 |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。
> 首次 `just ready` 要编译测试目标（nextest），可能超过 60 秒，别误判为卡死。

## 已知为坏

- **`.github/workflows/victauri.yml`**：在仓库根执行 `cargo build` 与 `cargo metadata`，
  但根目录没有 `Cargo.toml` → 该 job 必然失败。两种修法见 `ROADMAP.md` 阶段 0。

## 进行中 / 下一步

- [ ] **ADR-0001 定案**：待 cyrene 拍板 §3 决策二（v1 是否建 `akasha-vt`）与 §6 的未决项
- [ ] **落地决策一**（根 workspace）：步骤与验收见 [`docs/plans/0001`](./plans/0001-root-workspace.md)

## 踩过的坑（避免重复踩）

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency** ——
   它不会传递给你；缺了 `cargo check` 退出码 101。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**，
   直接启用会让 check 全红。另外它**不需要在仓库根执行** —— 只要求当前目录含 `Cargo.toml`。
3. **just 里变量写 `$p`**，不是 Make 的 `$$p`（后者会展开成 PID）。
4. **系统库缺失只会在 cargo 的构建脚本阶段暴露**（`javascriptcore-rs-sys`）。
   本机是 **CachyOS（Arch 系）**，用 `pacman -S webkit2gtk-4.1`，不是 apt。
5. **Tauri 没有 Rust 热重载，Victauri 也不提供**。迭代速度靠"逻辑下沉到 `crates/` +
   bacon 独立循环"拿回来，不靠热重载。
6. **`just` 的 shebang 配方需要可写的 runtime dir**，在受限环境会失败 —— 用普通配方。
7. **仓库根没有 `Cargo.toml`** → 一切没显式指定 manifest 的 cargo 命令在根目录失败：
   `cargo build`、`cargo metadata`（CI 因此坏掉）、`cargo fmt --all`（退出码 141）。
   `justfile` 已逐条加 `--manifest-path` 绕开，但这是需要永久保持的纪律。详见 ADR-0001 §2.2。
8. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** ——
   `fmt-check` 会红。跑一次 `just fmt` 规范化即可（已做）。

## 环境

CachyOS（Arch 系）/ rustc 1.98.1 / cargo 1.98.1 / node 26.8.2 / pnpm 12.3.4 / mise 2026.9.1
