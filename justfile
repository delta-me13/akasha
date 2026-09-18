# 项目级命令入口：用 just 管理开发流程中的脚本。
#
# 脚本架构（详见 AGENTS.md §11）：
#   - crate 级命令位于 src-tauri/justfile
#   - 项目级命令位于本文件，对 crate 级命令只做转发
#
# 相关工具依赖：just / bacon / cargo-nextest / cargo-deny（见 mise.toml、AGENTS.md §10）

set shell := ["bash", "-uc"]

SRC := "src-tauri"

default:
    @just --list

# ── 环境 ────────────────────────────────────────────────────────────────────

# 按 mise.toml 装齐全局 CLI 工具
tools:
    mise install

# 查看工具版本与来源
tools-ls:
    @mise ls

# 系统库前置检查（当前仅覆盖 Arch 系）
syscheck:
    @for p in webkit2gtk-4.1 javascriptcoregtk-4.1 gtk+-3.0 librsvg-2.0; do pkg-config --exists "$p" && echo "✅ $p $(pkg-config --modversion "$p")" || { echo "❌ $p 缺失 → Arch 系: sudo pacman -S webkit2gtk-4.1"; exit 1; }; done

# 检查 victauri 实际连接的项目标识
doctor:
    victauri doctor

# ── 开发循环 ────────────────────────────────────────────────────────────────

# 启动开发主控：前端 HMR，Rust 改动增量重编译并重启 app
dev:
    pnpm tauri dev

# 仅启动前端开发服务器
dev-web:
    pnpm dev

# ── AGENT 执行器（沙箱受限时的唯一出口；见 docs/agent-runner.md）──────────────
#
# workspace-write 沙箱禁止写入工作区与临时目录之外的任何路径（含 PTY 设备与 ~/.cargo），于是
# just dev / just test 只会拿到 openpty 的权限错误。下列配方把命令交给一个**能力表由工作区外文件决定**的执行器：
# policy.json 必须放在工作区与 /tmp 之外（两处在沙箱内可写，放进去等于把能力表交给被约束方）；动作不在表中或脚本哈希与登记值不符即拒绝启动。
# 生成样板：just runner-policy（复制到默认路径 ~/.akasha-agent-runner/policy.json）。
# 驱动分两种：runner-run 同步阻塞至结束；runner-submit 立即返回 request=<id>，之后由 runner-wait 阻塞等待。
# runner-wait 由内核唤醒而非轮询文件系统；它不可用时改用 runner-result 轮询，后者也可在运行途中查看状态。

# 生成 policy 样板（含当前脚本哈希），打印到终端并写入 docs/agent-runner.policy.json
runner-policy:
    @/usr/bin/python3 scripts/agent-runner.py --print-policy --out docs/agent-runner.policy.json

# 查看 agent-runner 服务状态与授权清单，无需提权。
runner-status:
    @/usr/bin/python3 scripts/agent-runner.py --status

# 常驻启动执行器（沙箱内的唯一提权点），需提权。
runner-start:
    /usr/bin/python3 scripts/agent-runner.py --serve

# 停止执行器并回收其名下的全部进程组，无需提权。
runner-stop:
    @/usr/bin/python3 scripts/agent-runner.py --stop

# 只回收 dev（app），保留执行器。
runner-stop-dev:
    @/usr/bin/python3 scripts/agent-runner.py --stop-dev

# 提交动作并阻塞到结束，退出码即动作退出码（默认等待 540s）。
runner-run action timeout="540":
    @/usr/bin/python3 scripts/agent-runner.py --run "{{action}}" --timeout "{{timeout}}"

# 只提交动作，立即返回 request=<id>；之后由 runner-wait 阻塞等待。
runner-submit action:
    @/usr/bin/python3 scripts/agent-runner.py --submit "{{action}}"

# 阻塞等待某请求结束（已结束则立即返回，可重复等待），默认超时 540s。
runner-wait id timeout="540":
    @/usr/bin/python3 scripts/agent-runner.py --wait "{{id}}" --timeout "{{timeout}}"

# 非阻塞读某请求的结果（未结束打印 running，退出码 4），适用于轮询。
runner-result id:
    @/usr/bin/python3 scripts/agent-runner.py --result "{{id}}"

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

# 可搬迁性验证：bin 所在文件夹搬走后数据仍在
portable:
    just --justfile {{SRC}}/justfile portable

# serial 的 libudev 只在 Linux 上（plan 0801 的判据）：按目标核对依赖图
libudev-check:
    just --justfile {{SRC}}/justfile libudev-check

# serial 的两条枚举实现都要真的执行一次（需两种 feature 配置）
serial-check:
    just --justfile {{SRC}}/justfile serial-check

# 吞吐基线（criterion），用于改动前后对比，不是门禁
bench:
    just --justfile {{SRC}}/justfile bench

# 依赖门禁：许可证 + 漏洞 + 来源（advisories 需联网）
deny:
    just --justfile {{SRC}}/justfile deny

# 依赖门禁，跳过需要联网的 advisories
deny-offline:
    just --justfile {{SRC}}/justfile deny-offline

# 由 Rust command/event 生成 src/ipc/bindings.ts
gen-types:
    just --justfile {{SRC}}/justfile gen-types

# 检查生成物是否与 Rust 侧一致
gen-types-check:
    just --justfile {{SRC}}/justfile gen-types-check

# ── 组合门禁（跨越根与 crate，只能在根定义）──────────────────────────────

# lint = clippy（crate 级）+ ast-grep scan（仓库级，配置文件为 sgconfig.yml）+ ast-grep test
#
# 两个 ast-grep 步骤守的不是同一件事，不得合并：
#   * `scan` 扫描整体仓库检查规则是否被违反；
#   * `test` 运行 `scripts/ast-grep/tests/` 里的**正例 / 反例**，验证规则本身是否仍然正确；⚠️ 测例不是真实路径下的文件，所以它**不覆盖 `files:` / `ignores:`** —— 改路径范围时要按 AGENTS.md §6 用真实路径探针复核一次。
lint:
    just clippy
    ast-grep scan
    ast-grep test

# 默认安静：每步一行 + 耗时；失败时输出该步日志（超过 80 行则打印首尾各 40 行），完整输出留在 .just-ready-fail.log。
# 要看某一步的完整输出就直接运行该配方（just deny-offline / just lint / just test ...）。
# ⚠️ 单步日志可达数千行，查看时需截断（如 tail）。
#
# 可执行的 DoD（AGENTS.md §7），提交前运行一次。
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
        echo "   完整输出: .just-ready-fail.log（或直接运行对应指令 just ${s}）"; \
        exit 1; \
      fi; \
    done; \
    echo "✅ just ready 通过（${ok}/${total}）"

# 文档纪律（四部分，详细规则见 AGENTS.md §8 / §8.1 / §8.2，plan 规则详见 docs/plans/README.md）。
# 四部分**每轮全部执行**（非快速失败）。
#
# A. 命令未漂移 —— 防止照着一份过期规则去用已不存在的旧命令
#    * docs/just.md §2 是**权威清单**：必须覆盖**全部**配方（正向，且**只承认 §2 表格内的记录**）
#    * AGENTS.md / README.md / ROADMAP.md 与 docs/**/*.md 可以仅记录部分指令，但记录的每个命令必须真实存在（反向）
# B. 汇总类文档不放细节 —— ROADMAP 放"判据"，不放"手段"
#    * 每条 ≤3 行、无代码块、反引号里不出现命令调用（--flag / {...}）
# C. plan 的预算与索引 —— 防止归档机制将细节写入单一大文件
#    * 每份 plan（含 archive/）≤200 行；超了要拆成两份，不是继续加
#    * 索引 docs/plans/README.md 双向一致：有文件必有索引行，有索引行必有文件
#    * 标为「进行中」却没有「## 验收命令」的 plan 直接红 —— 骨架 plan 不许开工
#
# 已知问题，记录在此避免重复：
#   1. 反斜杠转义的反引号在 grep -E 里会把反引号本身吞掉，于是 sed 剥不掉 "just " 前缀。
#      改用「just <名> + 右侧边界」判定，不依赖 markdown 写法。
#   2. 正向检查若不限定在 §2 表格内就形同虚设 —— 某条命令可能只在排错段落里被顺带提及，
#      而表格里已经删掉了。所以用 awk 取出 §2 段落，只在那里面找。
#   3. 含反引号的 grep 模式必须整体放进**单引号**里，否则会被 bash 当命令替换执行。
#   4. 反向检查**不含 CLAUDE.md** —— 该文件为 AGENTS.md 的指针 + Victauri 自动生成块，块内的英文散文会被裸词正则读成配方名（实测两条："just retry" / "just the"）。
#
# D. 文档语体 —— 剥离代码块与行内代码后匹配禁用语表（规范见 AGENTS.md §8.2）
#    * 表在 docs/style.md 的 BANNED 标记之间（唯一数据源；加词步骤见该文件 §2）；
#      规则本体与术语对照见 AGENTS.md §8.2。
#    * **非快速失败**：逐份文档各查一遍，全部查完才汇总报错；单份文档命中超过 20 条时
#      列出前 20 条并写明剩余条数。
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
        echo "❌ 标为「进行中」却没有「## 验收命令」—— 对应plan不允许开始实现: $f"; miss=1; \
      fi; \
    done; \
    for id in $(grep -oE '^\| [0-9]{4} ' docs/plans/README.md | grep -oE '[0-9]{4}'); do \
      find docs/plans -name "$id-*.md" | grep -q . || { echo "❌ docs/plans/README.md 索引里的 plan 没有文件: $id"; miss=1; }; \
    done; \
    if [ "$miss" = "1" ]; then echo "❌ 文档纪律未通过 —— 四部分均已执行完毕，上面列出的是本轮全部待修项"; echo "→ 命令类问题同步 docs/just.md §2；纪律类问题见 AGENTS.md §8.1；plan 类问题见 docs/plans/README.md"; exit 1; fi; \
    echo "✅ 文档纪律通过（语体符合 AGENTS.md §8.2；命令与 justfile 同步；ROADMAP $(grep -cE '^- \[[ x~!]\]' ROADMAP.md) 个条目均在 3 行内、无代码块与命令调用；plan $(printf '%s\n' "$plans" | grep -c . ) 份 ≤200 行且索引一致）"
