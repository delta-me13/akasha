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
#
# 监听范围**不需要额外配置**：workspace 就在 `src-tauri/` 里（ADR-0004），
# 而 tauri CLI 默认监听 `src-tauri` —— 成员天然被覆盖，实测见 docs/STATUS.md 坑 #21。
# ⚠️ 别把成员挪到 `src-tauri/` 外面：一旦挪出去，开发循环会**静默失效**
# （门禁全绿，但改了代码看不到效果），那时才需要 `build.additionalWatchFolders`。
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

# CI 工作流的**双 forge 可移植性**检查（Gitea + GitHub 都要能跑）。
#
# 为什么必须在这里查：这四条约束一旦破了，**恰恰是 CI 跑不起来** ——
# 而破掉的那一刻，本地没有任何门禁会红（Gitea 上是"静默不跑"或"排队等一个不存在的 runner"）。
# 规则本身与出处写在 `.github/workflows/ci.yml` 的文件头。
#
# 为什么用 grep 而不是解析 YAML：本地与 CI 都没有稳定的 YAML 解析器（js-yaml 不在依赖里），
# 而这四条在**文本层面就是确定的** —— 引入解析器只会多一个依赖和一片误报面。
# 顺带避开一个雷：配方体里不能出现两个连续花括号（just 会当插值炸掉），
# 所以模式写 `runner\.(os|arch|…)` 而不是那个上下文的字面量（坑 #23）。
# **注释行不算**（文件头必须能讨论这些约束本身）。
ci-check:
    @miss=0; \
    wf=.github/workflows/ci.yml; \
    code=$(grep -v '^[[:space:]]*#' "$wf"); \
    if printf '%s\n' "$code" | grep -qE 'runner\.(os|arch|temp|tool_cache|name|debug|environment)'; then \
      echo "❌ 用到了 runner 上下文 —— Gitea 的上下文表里没有它（坑 #25）"; miss=1; \
    fi; \
    printf '%s\n' "$code" | grep -q 'github.server_url' || { \
      echo "❌ 没有 github.server_url 门 —— Windows/macOS 分支在 Gitea 上会一直排队等一个不存在的 runner"; miss=1; \
    }; \
    for p in ubuntu-latest windows-latest macos-latest; do \
      printf '%s\n' "$code" | grep -q "$p" || { echo "❌ 平台矩阵缺 $p"; miss=1; }; \
    done; \
    if [ -e .gitea/workflows ]; then \
      echo "❌ 存在 .gitea/workflows/ —— Gitea 只读**第一个存在**的 workflow 目录，它会因此忽略 .github/workflows/ 而静默不跑（坑 #24）"; miss=1; \
    fi; \
    if [ "$miss" = "1" ]; then exit 1; fi; \
    echo "✅ CI 双 forge 检查通过（无 runner 上下文；非 Linux 分支有 github.server_url 门；三平台齐全；无 .gitea/workflows 抢占目录）"

# 可执行的 DoD（AGENTS.md §7）：提交前跑这一个。
#
# 默认安静：每步一行 + 耗时；失败时把该步输出倒出来（超长则首尾各 40 行并落盘）。
# 为什么需要这个壳：cargo deny 对 Tauri 这种依赖树会打 5000+ 行「重复版本」警告，
# 而 multiple-versions 是 warn 级、永远不让门禁失败 —— 在成功的运行里那些纯粹是噪音。
# 要看完整输出就直接跑单个配方（just deny-offline / just lint / just test ...）。
ready:
    @set -uo pipefail; \
    steps="fmt-check lint ci-check test deny-offline docs-check"; \
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

# 文档纪律（三部分，规则见 AGENTS.md §8 与 §8.1，plan 规则见 docs/plans/README.md）。
#
# A. 命令未漂移 —— 防止照着一份过期规则去用已不存在的旧命令
#    * docs/just.md §2 是**权威清单**：必须覆盖**全部**配方（正向，且**只认 §2 表格内的记录**）
#    * 顶层文档与 docs/**/*.md 可以只提一部分，但提到的每个命令必须真实存在（反向）
# B. 汇总类文档没长细节 —— ROADMAP 放"判据"，不放"手段"
#    * 每条 ≤3 行、无代码块、反引号里不出现命令调用（--flag / {...}）
#    * 只拦"细节泄漏"，**不拦能力条目本身的增长**：条目数该随能力涨，行数不该随细节涨
# C. plan 的预算与索引 —— 防止归档机制把细节堆进一个大文件
#    * 每份 plan（含 archive/）≤200 行；超了要拆成两份，不是继续加
#    * 索引 docs/plans/README.md 双向一致：有文件必有索引行，有索引行必有文件
#    * 标「进行中」却没有「## 验收命令」的 plan 直接红 —— 骨架 plan 不许开工
#
# 踩过的坑，写在这里免得重蹈：
#   1. 反斜杠转义的反引号在 grep -E 里会把反引号本身吞掉，于是 sed 剥不掉 "just " 前缀。
#      改用「just <名> + 右侧边界」判定，不依赖 markdown 写法。
#   2. 正向检查若不限定在 §2 表格内就形同虚设 —— 某条命令可能只在排错段落里被顺带提及，
#      而表格里其实已经删掉了。所以用 awk 取出 §2 段落，只在那里面找。
#   3. 含反引号的 grep 模式必须整体放进**单引号**里，否则会被 bash 当命令替换执行。
docs-check:
    @miss=0; \
    recipes=$( { just --summary; just --justfile {{SRC}}/justfile --summary; } | tr ' ' '\n' | sort -u ); \
    sec2=$(awk '/^## 2\. /{f=1} /^## 3\. /{f=0} f' docs/just.md); \
    for r in $recipes; do \
      if [ "$r" = "default" ]; then continue; fi; \
      printf '%s\n' "$sec2" | grep -qE "just $r([^a-z0-9-]|$)" || { echo "❌ docs/just.md §2 表格未记录: just $r"; miss=1; }; \
    done; \
    for f in AGENTS.md ROADMAP.md $(find docs -name '*.md'); do \
      for m in $(grep -oE "just [a-z][a-z0-9-]*" $f | sed 's/^just //' | sort -u); do \
        printf '%s\n' "$recipes" | grep -qx "$m" || { echo "❌ $f 提到了不存在的配方: just $m"; miss=1; }; \
      done; \
    done; \
    grep -q '```' ROADMAP.md && { echo "❌ ROADMAP.md 出现代码块 —— 步骤与命令属于 docs/plans/"; miss=1; }; \
    hits=$(grep -nE '`[^`]*(--[a-zA-Z]|\{[^`]*\})[^`]*`' ROADMAP.md || true); \
    if [ -n "$hits" ]; then echo "❌ ROADMAP.md 粘进了命令调用 —— 验证手段属于 plan 的「验收命令」:"; echo "$hits"; miss=1; fi; \
    over=$(awk '/^- \[[ x~!]\]/{if(n>3)print st; st=NR;n=1;next} /^[[:space:]]/{if(n>0){n++;next}} {if(n>0&&n>3)print st; n=0} END{if(n>0&&n>3)print st}' ROADMAP.md); \
    if [ -n "$over" ]; then echo "❌ ROADMAP.md 条目超过 3 行上限（起始行号）: $over"; miss=1; fi; \
    plans=$(find docs/plans -name '[0-9][0-9][0-9][0-9]-*.md' | sort); \
    for f in $plans; do \
      n=$(wc -l < "$f"); \
      if [ "$n" -gt 200 ]; then echo "❌ plan 超过 200 行预算（$n 行）: $f —— 拆成两份，别把细节堆进一个文件"; miss=1; fi; \
      id=$(basename "$f" | cut -c1-4); \
      grep -qE "^\| $id " docs/plans/README.md || { echo "❌ docs/plans/README.md 索引缺行: $id ($f)"; miss=1; }; \
      if grep -qE '\*\*状态\*\*：进行中' "$f" && ! grep -q '^## 验收命令' "$f"; then \
        echo "❌ 标为「进行中」却没有「## 验收命令」—— 骨架 plan 不许开工: $f"; miss=1; \
      fi; \
    done; \
    for id in $(grep -oE '^\| [0-9]{4} ' docs/plans/README.md | grep -oE '[0-9]{4}'); do \
      find docs/plans -name "$id-*.md" | grep -q . || { echo "❌ docs/plans/README.md 索引里的 plan 没有文件: $id"; miss=1; }; \
    done; \
    if [ "$miss" = "1" ]; then echo "→ 命令类问题同步 docs/just.md §2；纪律类问题见 AGENTS.md §8.1；plan 类问题见 docs/plans/README.md"; exit 1; fi; \
    echo "✅ 文档纪律通过（命令与 justfile 同步；ROADMAP $(grep -cE '^- \[[ x~!]\]' ROADMAP.md) 个条目均在 3 行内、无代码块与命令调用；plan $(printf '%s\n' "$plans" | grep -c . ) 份 ≤200 行且索引一致）"
