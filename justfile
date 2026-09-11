# akasha — 唯一命令入口。规则文件只引用这里的配方，不要另写一套命令。
# 依赖：just（见 AGENTS.md §10.1）。未安装 just 时，可读配方正文手动执行等价命令。

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

# Rust 秒级反馈循环（bacon），全程不启动 app。需要 bacon。
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

test:
    cargo test --manifest-path {{MANIFEST}}

# 需要已安装 just/bacon/cargo-nextest 后切换为：
#   cargo nextest run --manifest-path {{MANIFEST}}

# E2E：需要 app 正在运行（just dev）
test-e2e:
    VICTAURI_E2E=1 cargo test --manifest-path {{MANIFEST}} --test smoke --test integration -- --test-threads=1

# 连接检查：确认连到的是 akasha，而不是别的 Victauri app
doctor:
    victauri doctor

# ── 类型边界 ────────────────────────────────────────────────────────────────
# Rust command/event → src/ipc/bindings.ts。改过 IPC 就必须跑，并提交产物。
gen-types:
    @echo "TODO: 接入 tauri-specta 后启用（见 AGENTS.md §5、§10.5）"

# ── 提交前 ──────────────────────────────────────────────────────────────────
precommit: fmt-check lint test
