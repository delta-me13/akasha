# akasha — 唯一命令入口。规则文件只引用这里的配方，不要另写一套命令。
# 依赖：just / bacon / cargo-nextest / cargo-deny / sccache（已安装，见 AGENTS.md §10）。

set shell := ["bash", "-uc"]

MANIFEST := "src-tauri/Cargo.toml"

default:
    @just --list

# ── 环境 ────────────────────────────────────────────────────────────────────

# 按 mise.toml 装齐全局 CLI 工具（这些不是 Cargo.toml 依赖，见 AGENTS.md §10）
tools:
    mise install

# 查看工具版本与来源
tools-ls:
    @mise ls

# 系统库前置检查（Arch 系：webkit2gtk-4.1 缺失会让 cargo 在构建脚本阶段才失败）。
# 刻意不用 shebang 配方 —— 那要求 just 能写 runtime dir，在受限/CI 环境里会无谓失败。
syscheck:
    @for p in webkit2gtk-4.1 javascriptcoregtk-4.1 gtk+-3.0 librsvg-2.0; do pkg-config --exists "$p" && echo "✅ $p $(pkg-config --modversion "$p")" || { echo "❌ $p 缺失 → Arch 系: sudo pacman -S webkit2gtk-4.1"; exit 1; }; done

# ── 开发循环 ────────────────────────────────────────────────────────────────

# 常驻开发主控：前端 HMR；Rust 改动自动增量重编译 + 重启 app。
# 只需启动一次，不要每次手动重跑（AGENTS.md §1）。
dev:
    pnpm tauri dev

# 只跑前端：不启动 app，配合 mockIPC 在浏览器里迭代 UI。
dev-web:
    pnpm dev

# Rust 秒级反馈循环，全程不启动 app。修纯逻辑时用这个，别等 app 重启。
watch:
    bacon clippy

# ── 质量门禁 ────────────────────────────────────────────────────────────────

check:
    cargo check --manifest-path {{MANIFEST}} --all-targets

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# DoD 第一项：必须全绿
lint:
    cargo clippy --manifest-path {{MANIFEST}} --all-targets -- -D warnings
    ast-grep scan

# 单元测试：纯 crate 不启动 app
test:
    cargo nextest run --manifest-path {{MANIFEST}}

# 依赖门禁：许可证 + 漏洞 + 来源。配置在 src-tauri/deny.toml。
# advisories 需要联网拉取 RustSec 数据库。
deny:
    cargo deny --manifest-path {{MANIFEST}} --config src-tauri/deny.toml check

# 同上但跳过需要联网的 advisories（离线可用）
deny-offline:
    cargo deny --manifest-path {{MANIFEST}} --config src-tauri/deny.toml check licenses bans sources

# E2E：需要 app 正在运行（just dev）。
# 用 cargo test 而非 nextest —— 这些用例要求串行且依赖真实 app 进程。
test-e2e:
    VICTAURI_E2E=1 cargo test --manifest-path {{MANIFEST}} --test smoke --test integration -- --test-threads=1

# 连接检查：确认连到的是 akasha，而不是别的 Victauri app
doctor:
    victauri doctor

# ── 类型边界 ────────────────────────────────────────────────────────────────

# Rust command/event → src/ipc/bindings.ts。改过 IPC 就必须跑，并提交产物。
gen-types:
    @echo "TODO: 接入 tauri-specta 后启用（见 AGENTS.md §5）"

# ── 提交前 ──────────────────────────────────────────────────────────────────

precommit: fmt-check lint test
