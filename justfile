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

# 监听范围**不需要额外配置**：workspace 就在 `src-tauri/` 里（ADR-0004），
# 而 tauri CLI 默认监听 `src-tauri` —— 成员天然被覆盖，实测见 docs/STATUS.md 问题 #21。
# ⚠️ 别把成员挪到 `src-tauri/` 外面：一旦挪出去，开发循环会**静默失效**
# （门禁全绿，但改了代码看不到效果），那时才需要 `build.additionalWatchFolders`。
#
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

# 可搬迁性：把 bin 所在文件夹搬走之后数据还在吗（自己起 app，见 src-tauri/justfile）
portable:
    just --justfile {{SRC}}/justfile portable

# serial 的 libudev 只在 Linux 上（plan 0801 的判据）：按目标核对依赖图
libudev-check:
    just --justfile {{SRC}}/justfile libudev-check

# serial 的两条枚举实现都要真的执行一次（需两种 feature 配置）
serial-check:
    just --justfile {{SRC}}/justfile serial-check

# 吞吐基线（criterion）。**不是门禁** —— 它是用于改动前后对比的基线
bench:
    just --justfile {{SRC}}/justfile bench

# 依赖门禁：许可证 + 漏洞 + 来源（advisories 需联网）
deny:
    just --justfile {{SRC}}/justfile deny

# 依赖门禁，跳过需要联网的 advisories
deny-offline:
    just --justfile {{SRC}}/justfile deny-offline

# Rust command/event → src/ipc/bindings.ts
gen-types:
    just --justfile {{SRC}}/justfile gen-types

# 生成物是否与 Rust 侧一致（改过 IPC 忘了生成就红）
gen-types-check:
    just --justfile {{SRC}}/justfile gen-types-check

# ── 组合门禁（跨越根与 crate，所以只能在根定义）──────────────────────────────

# lint = clippy（crate 级）+ ast-grep scan（仓库级，读根 sgconfig.yml）+ ast-grep test
#
# 两个 ast-grep 步骤守的是不同的东西，别合并：
#   * `scan` 扫**真代码**——规则有没有被违反；
#   * `test` 跑 `scripts/ast-grep/tests/` 里的**正例 / 反例**——规则自己还对不对（改窄了、改宽了、
#     正则写错了都在这儿现形）。⚠️ 它**不覆盖 `files:` / `ignores:`**（测例不是真实路径
#     下的文件）：改路径范围时要按 AGENTS.md §6 用真实路径的探针复核一次。
lint:
    just clippy
    ast-grep scan
    ast-grep test

# 默认安静：每步一行 + 耗时；失败时把该步输出倒出来（超长则首尾各 40 行并落盘）。
# 为什么需要这个壳：cargo deny 对 Tauri 这种依赖树会打 5000+ 行「重复版本」警告，
# 而 multiple-versions 是 warn 级、永远不让门禁失败 —— 在成功的运行里那些纯粹是噪音。
# 要看完整输出就直接跑单个配方（just deny-offline / just lint / just test ...）。
#
# 可执行的 DoD（AGENTS.md §7）：提交前跑这一个。
ready:
    @set -uo pipefail; \
    steps="fmt-check lint test deny-offline gen-types-check docs-check"; \
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
        echo "   完整输出: .just-ready-fail.log（或直接单跑 just ${s}）"; \
        exit 1; \
      fi; \
    done; \
    echo "✅ just ready 全绿（${ok}/${total}）"

# 文档纪律（四部分，规则见 AGENTS.md §8 / §8.1 / §8.2，plan 规则见 docs/plans/README.md）。
# 四部分**每轮全部执行**（非快速失败）：第一类失败不终止其余三类 —— 一轮给出全部待修项。
#
# A. 命令未漂移 —— 防止照着一份过期规则去用已不存在的旧命令
#    * docs/just.md §2 是**权威清单**：必须覆盖**全部**配方（正向，且**只认 §2 表格内的记录**）
#    * AGENTS.md / README.md / ROADMAP.md 与 docs/**/*.md 可以只提一部分，但提到的每个命令必须真实存在（反向）
# B. 汇总类文档没长细节 —— ROADMAP 放"判据"，不放"手段"
#    * 每条 ≤3 行、无代码块、反引号里不出现命令调用（--flag / {...}）
#    * 只拦"细节泄漏"，**不拦能力条目本身的增长**：条目数该随能力涨，行数不该随细节涨
# C. plan 的预算与索引 —— 防止归档机制把细节堆进一个大文件
#    * 每份 plan（含 archive/）≤200 行；超了要拆成两份，不是继续加
#    * 索引 docs/plans/README.md 双向一致：有文件必有索引行，有索引行必有文件
#    * 标「进行中」却没有「## 验收命令」的 plan 直接红 —— 骨架 plan 不许开工
#
# 已知问题，记录在此避免重复：
#   1. 反斜杠转义的反引号在 grep -E 里会把反引号本身吞掉，于是 sed 剥不掉 "just " 前缀。
#      改用「just <名> + 右侧边界」判定，不依赖 markdown 写法。
#   2. 正向检查若不限定在 §2 表格内就形同虚设 —— 某条命令可能只在排错段落里被顺带提及，
#      而表格里其实已经删掉了。所以用 awk 取出 §2 段落，只在那里面找。
#   3. 含反引号的 grep 模式必须整体放进**单引号**里，否则会被 bash 当命令替换执行。
#   4. 反向检查**不含 CLAUDE.md** —— 那个文件是 AGENTS.md 的指针 + Victauri 自动生成块，
#      块内的英文散文会被裸词正则读成配方名（实测两条："just retry" / "just the"）。
#
# D. 文档语体 —— 剥离代码块与行内代码后匹配禁用语表（规范见 AGENTS.md §8.2）
#    * 表在 docs/style.md 的 BANNED 标记之间（唯一数据源；加词步骤见该文件 §2）；
#      规则本体与术语对照见 AGENTS.md §8.2。片段拼成一条正则后逐份文档 grep -E。
#    * **非快速失败**：逐份文档各查一遍，全部查完才汇总报错 —— 一轮修完全部命中，
#      不必"改一份再执行一次"；单份文档命中超过 20 条时列出前 20 条并写明剩余条数。
#    * 由 docs-check 调用时同样不终止它的其余三类检查（那是**调用方**的性质）。
#    * `paste` 要**显式写输入操作数 `-`**：BSD 的 paste（macOS）不给文件操作数就报 usage，
#      而 GNU 的 paste 默认读 stdin —— 这两行的差别只在 macOS 上暴露，本机实测。
docs-style:
    @pat=$(awk '/<!-- BANNED:BEGIN -->/{f=1;next} /<!-- BANNED:END -->/{f=0} f' docs/style.md \
             | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' \
             | grep -vE '^(#|```|$)' | paste -s -d'|' -); \
    if [ -z "$pat" ]; then echo "❌ 读不到禁用语表 —— 检查 docs/style.md 的 BANNED 标记与内容"; exit 1; fi; \
    bad=0; hit_files=""; \
    for f in AGENTS.md CLAUDE.md README.md ROADMAP.md $(find docs -name '*.md' | sort); do \
      hits=$(awk 'BEGIN{n=0} /^[[:space:]]*```/{n=!n;next} n==0{print}' "$f" | sed -E 's/`[^`]*`//g' | grep -nE -e "$pat" || true); \
      if [ -n "$hits" ]; then \
        bad=1; hit_files="$hit_files $f"; count=$(printf '%s\n' "$hits" | wc -l); \
        echo "❌ $f 命中禁用语（表见 docs/style.md，规则见 AGENTS.md §8.2）:"; \
        if [ "$count" -gt 20 ]; then \
          printf '%s\n' "$hits" | head -n 20; \
          echo "   …（本文件另有 $((count - 20)) 条命中未列出）"; \
        else \
          printf '%s\n' "$hits"; \
        fi; \
      fi; \
    done; \
    if [ "$bad" != "0" ]; then echo "❌ 文档语体未通过：下列文档命中禁用语"; printf '%s\n' $hit_files; echo "→ 术语对照见 AGENTS.md §8.2；加词与收窄见 docs/style.md §2"; exit 1; fi; \
    echo "✅ 文档语体通过（docs/style.md 的禁用语表无命中）"

docs-check:
    @miss=0; \
    if ! just docs-style; then miss=1; fi; \
    recipes=$( { just --summary; just --justfile {{SRC}}/justfile --summary; } | tr ' ' '\n' | sort -u ); \
    sec2=$(awk '/^## 2\. /{f=1} /^## 3\. /{f=0} f' docs/just.md); \
    for r in $recipes; do \
      if [ "$r" = "default" ]; then continue; fi; \
      printf '%s\n' "$sec2" | grep -qE "just $r([^a-z0-9-]|$)" || { echo "❌ docs/just.md §2 表格未记录: just $r"; miss=1; }; \
    done; \
    for f in AGENTS.md README.md ROADMAP.md $(find docs -name '*.md'); do \
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
    if [ "$miss" = "1" ]; then echo "❌ 文档纪律未通过 —— 四部分均已执行完毕，上面列出的是本轮全部待修项"; echo "→ 命令类问题同步 docs/just.md §2；纪律类问题见 AGENTS.md §8.1；plan 类问题见 docs/plans/README.md"; exit 1; fi; \
    echo "✅ 文档纪律通过（语体符合 AGENTS.md §8.2；命令与 justfile 同步；ROADMAP $(grep -cE '^- \[[ x~!]\]' ROADMAP.md) 个条目均在 3 行内、无代码块与命令调用；plan $(printf '%s\n' "$plans" | grep -c . ) 份 ≤200 行且索引一致）"
