# akasha — 唯一命令入口。规则文件只引用这里的配方，不要另写一套命令。
# 依赖：just / bacon / cargo-nextest / cargo-deny / sccache（已安装，见 AGENTS.md §10）。

set shell := ["bash", "-uc"]

MANIFEST := "src-tauri/Cargo.toml"

default:
    @just --list

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

# 依赖门禁：许可证 + CVE（配置在 deny.toml）
deny:
    cargo deny check

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
