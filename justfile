# akasha — 项目级命令入口。
#
# 分工（详见 AGENTS.md §11）：
#   * crate 级命令住在 src-tauri/justfile（那里 cwd 天然正确，无需 --manifest-path）
#   * 本文件放项目级命令，并对 crate 级命令**只做转发**
#   * 命令体永远只有一处，不复制 —— 复制出来的第二份必然漂移
#
# 依赖：just / bacon / cargo-nextest / cargo-deny / sccache（见 mise.toml、AGENTS.md §10）

set shell := ["bash", "-uc"]

SRC := "src-tauri"

default:
    @just --list

# ── 环境 ────────────────────────────────────────────────────────────────────

# 按 mise.toml 装齐全局 CLI 工具（这些不是 Cargo.toml 依赖）
tools:
    mise install

# 查看工具版本与来源
tools-ls:
    @mise ls

# 系统库前置检查（Arch 系缺 webkit2gtk-4.1 时，cargo 要到构建脚本阶段才报错）
syscheck:
    @for p in webkit2gtk-4.1 javascriptcoregtk-4.1 gtk+-3.0 librsvg-2.0; do pkg-config --exists "$p" && echo "✅ $p $(pkg-config --modversion "$p")" || { echo "❌ $p 缺失 → Arch 系: sudo pacman -S webkit2gtk-4.1"; exit 1; }; done

# 连接检查：确认连到的是 akasha，而不是别的 Victauri app
doctor:
    victauri doctor

# ── 开发循环 ────────────────────────────────────────────────────────────────

# 常驻开发主控：前端 HMR；Rust 改动自动增量重编译 + 重启 app。只需启动一次
dev:
    pnpm tauri dev

# 只跑前端：不启动 app，配合 mockIPC 在浏览器里迭代 UI
dev-web:
    pnpm dev

# ── crate 级命令（转发到 src-tauri/justfile）─────────────────────────────────

# 类型检查（含 tests / benches）
check:
    just --justfile {{SRC}}/justfile check

# clippy，警告即错误
clippy:
    just --justfile {{SRC}}/justfile clippy

fmt:
    just --justfile {{SRC}}/justfile fmt

fmt-check:
    just --justfile {{SRC}}/justfile fmt-check

# Rust 秒级反馈循环（bacon），不启动 app
watch:
    just --justfile {{SRC}}/justfile watch

# 单元测试（cargo-nextest）
test:
    just --justfile {{SRC}}/justfile test

# E2E：需要 app 正在运行（`just dev`）
test-e2e:
    just --justfile {{SRC}}/justfile test-e2e

# 依赖门禁：许可证 + 漏洞 + 来源（advisories 需联网）
deny:
    just --justfile {{SRC}}/justfile deny

# 依赖门禁，跳过需要联网的 advisories
deny-offline:
    just --justfile {{SRC}}/justfile deny-offline

# Rust command/event → src/ipc/bindings.ts
gen-types:
    just --justfile {{SRC}}/justfile gen-types

# ── 组合门禁（跨越根与 crate，所以只能在根定义）──────────────────────────────

# lint = clippy（crate 级）+ ast-grep scan（仓库级，读根 sgconfig.yml）
lint:
    just clippy
    ast-grep scan

# 可执行的 DoD（AGENTS.md §7）：提交前跑这一个。
# 含 docs-check —— 命令表一旦与 justfile 漂移就当场失败，避免 agent 用过期命令。
ready: fmt-check lint test deny-offline docs-check

# 校验 AGENTS.md §11 的命令表没有与 justfile 漂移 ——
# 防止 agent 照着一份过期的规则去用已经不存在的旧命令。
docs-check:
    @miss=0; \
    recipes=$( { just --summary; just --justfile {{SRC}}/justfile --summary; } | tr ' ' '\n' | sort -u ); \
    for r in $recipes; do \
      if [ "$r" = "default" ]; then continue; fi; \
      if ! grep -qF "\`just $r\`" AGENTS.md; then echo "❌ AGENTS.md §11 未记录: just $r"; miss=1; fi; \
    done; \
    if [ "$miss" = "1" ]; then echo "→ 请更新 AGENTS.md §11 的命令表"; exit 1; fi; \
    echo "✅ AGENTS.md §11 命令表与 justfile 同步"
