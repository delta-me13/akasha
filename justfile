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
#
# 默认安静：每步一行 + 耗时；失败时把该步输出倒出来（超长则首尾各 40 行并落盘）。
# 为什么需要这个壳：cargo deny 对 Tauri 这种依赖树会打 5000+ 行「重复版本」警告，
# 而 multiple-versions 是 warn 级、永远不让门禁失败 —— 在成功的运行里那些纯粹是噪音。
# 要看完整输出就直接跑单个配方（just deny-offline / just lint / just test ...）。
ready:
    @set -uo pipefail; \
    steps="fmt-check lint test deny-offline docs-check"; \
    total=0; ok=0; \
    for s in $steps; do \
      total=$((total + 1)); \
      printf '→ %-13s' "$s"; \
      start=$(date +%s); \
      if out=$(just "$s" 2>&1); then \
        ok=$((ok + 1)); \
        printf ' ✅ %ss\n' "$(( $(date +%s) - start ))"; \
      else \
        printf ' ❌ %ss\n' "$(( $(date +%s) - start ))"; \
        printf '%s\n' "$out" > .just-ready-fail.log; \
        lines=$(wc -l < .just-ready-fail.log); \
        if [ "$lines" -gt 80 ]; then \
          head -n 40 .just-ready-fail.log; \
          echo "   …（省略 $((lines - 80)) 行）…"; \
          tail -n 40 .just-ready-fail.log; \
        else \
          cat .just-ready-fail.log; \
        fi; \
        echo; \
        echo "❌ just ready 失败于: just $s"; \
        echo "   完整输出: .just-ready-fail.log（或直接单跑 just $s）"; \
        exit 1; \
      fi; \
    done; \
    echo "✅ just ready 全绿（$ok/$total）"

# 校验文档里的命令与实际 justfile 未漂移，防止照着一份过期规则去用已不存在的旧命令。
#   * AGENTS.md §11 必须覆盖**全部**配方（正向 —— 新增配方不能不记录）
#   * docs/just.md 可以只提一部分，但它提到的每个命令必须真实存在（反向 —— 防过期）
#
# 坑：这里刻意不用反引号做锚点。grep -E 里写反斜杠转义的反引号时，匹配结果里
# 反引号本身会被吞掉，于是后续 sed 剥不掉 "just " 前缀，提取出的名字全带前缀。
# 用「just <名> + 右侧边界」来判定，既躲开这个坑，也不依赖 markdown 写法。
docs-check:
    @miss=0; \
    recipes=$( { just --summary; just --justfile {{SRC}}/justfile --summary; } | tr ' ' '\n' | sort -u ); \
    for r in $recipes; do \
      if [ "$r" = "default" ]; then continue; fi; \
      grep -qE "just $r([^a-z0-9-]|$)" AGENTS.md || { echo "❌ AGENTS.md §11 未记录: just $r"; miss=1; }; \
    done; \
    if [ -f docs/just.md ]; then \
      for m in $(grep -oE "just [a-z][a-z0-9-]*" docs/just.md | sed 's/^just //' | sort -u); do \
        printf '%s\n' "$recipes" | grep -qx "$m" || { echo "❌ docs/just.md 提到了不存在的配方: just $m"; miss=1; }; \
      done; \
    fi; \
    if [ "$miss" = "1" ]; then echo "→ 请更新 AGENTS.md §11 与 docs/just.md"; exit 1; fi; \
    echo "✅ 文档命令与 justfile 同步（AGENTS.md §11 + docs/just.md）"
