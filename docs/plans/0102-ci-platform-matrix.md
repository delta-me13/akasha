# Plan 0102: CI —— 平台矩阵（**GitHub Actions 一份**）

- **关联**：ROADMAP 阶段 1 ·「CI 平台矩阵」；阶段 0 ·「CI 变绿」
- **前置**：plan 0101 / 0106（manifest 位置；矩阵里每条都跑 workspace 级检查）
- **状态**：进行中（本地部分已完成 2026-09-11；**最终判据待 CI 实跑**，见「实施记录」）
- **影响面**：`.github/workflows/ci.yml`、`AGENTS.md` §12、`docs/just.md` §8

## 目标

`.github/workflows/ci.yml` **一份文件**：Linux 跑完整门禁与 E2E，Windows / macOS 通过类型检查。

平台矩阵是主体工作量的来源（PTY / serial / ssh 的 cfg 分支与平台 API），留到收尾等于把
最大的坑留到最晚（`scope.md` §9）；而"CI 只在没配过的 forge 上能跑"等于没有 CI。

## 非目标

- **不**在 Windows / macOS **出包**：打包需要真实主机（WiX / NSIS / WebView2 bootstrapper 都不行）
- **不**引入 `cargo-xwin`：`cargo check --target` 已覆盖"挡 cfg 错误"这个 90% 诉求；
  它解决的是**链接**问题，救不了 Windows 打包
- **不**为第二个 forge 维护兼容层 —— 试过，放弃了，理由见「放弃记录」

## 前置检查

```bash
git remote -v                                   # 现在是空 —— CI 还没在任何 forge 上跑过
grep -nE 'runs-on|needs:|if:' .github/workflows/ci.yml
just ready                                      # 本地门禁必须先是绿的
```

## 步骤

1. **Linux job 跑的就是本地那一条 `just ready`**（`checks-linux`）。门禁只有一处定义：
   在 workflow 里另写 cargo 命令，两处必然分叉，而分叉的方向总是"CI 比本地松"。
2. **Windows / macOS 只做类型检查**（`checks-other`，`runs-on: ${{ matrix.os }}`，
   矩阵 `[windows-latest, macos-latest]`，`fail-fast: false`）。走 crate 级 `check` 的转发配方。
3. **E2E 独立成 job，只 `needs: checks-linux`**（`e2e`）。平台类型检查与 E2E 是互相独立的信号：
   若 `e2e` 也 needs `checks-other`，一条 Windows 红就会顺带吃掉 E2E 的结论。
4. Linux 装 apt 依赖（含 `libayatana-appindicator3-dev` 与 `libxdo-dev`，坑 #10）后跑
   `just ready`；完整门禁只跑一次，避免矩阵把时间乘三。依赖列表只有 `env.APT_DEPS` 一处。
5. **工具安装统一走 `taiki-e/install-action`**（预编译产物 + SHA256 / attestation 校验）：
   `checks-linux` 一次装齐 `just@1.58.0,cargo-nextest,cargo-deny`，`checks-other` 只装
   `just@1.58.0`（版本写死，与本地对齐）。**不要手写"按平台选资产 + curl + 追加
   `GITHUB_PATH`"的脚本** —— 那是兼容层的遗产，理由见「放弃记录」。
6. ast-grep 走官方 npm 包，**直接 `npm install -g`**：不再需要 `--prefix "$HOME/.local"`
   绕全局写权限（那是 job 容器里的写法，GitHub runner 上没有这个问题）。
7. **把 GitHub 专属的省钱 / 提速开关用上**：`concurrency` + `cancel-in-progress` 取消同一分支
   上被取代的运行（`main` 除外 —— 合并后的结论不该被掐断）、`permissions: contents: read`、
   `defaults.run.shell: bash`（与本地配方的 bash shell 对齐，Windows 上不再逐步写 `shell:`）。
8. 缓存 `~/.cargo` 与 `target`，**按平台分键**（`Swatinem/rust-cache` 的键自动含 OS 与 rustc 版本）。
9. E2E 用 `cargo metadata` 问产物路径、在 `xvfb-run` 下起 app、跑
   `victauri-test` 与 `--test integration`，并**用 `if: always()` 收掉后台进程**。
10. Windows / macOS 若在 `cargo check` 阶段因 Tauri 的构建脚本失败：记录到「实施记录」并**单独决策**，
    不要就地绕开（绕过等于把平台差异藏起来，正是本 plan 要消灭的东西）。

### 还想要的省钱手段（未启用，留个记录）

- **跳过 draft PR**（`if: github.event.pull_request.draft == false`）：省一次完整运行，
  代价是"draft 打开时 CI 不跑"这件事本身要记得住，先不加。
- **按改动路径过滤**（`paths-ignore`）：文档改动其实**会影响** `docs-check`，
  忽略它等于把一条门禁藏起来，不做。

## 验收命令

```bash
# 1. YAML 能解析，job 结构符合预期（用临时的 js-yaml，不进仓库）
node -e 'const y=require("/tmp/yaml"),fs=require("fs");const w=y.load(fs.readFileSync(".github/workflows/ci.yml","utf8"));for(const [k,v] of Object.entries(w.jobs))console.log(k, JSON.stringify(v["runs-on"]), JSON.stringify(v.if??null), JSON.stringify(v.needs??null))'
# 期望：checks-linux / ubuntu-latest / null / null
#       checks-other / "${{ matrix.os }}" / null / null
#       e2e / ubuntu-latest / null / "checks-linux"

# 2. 三平台齐全，且非 Linux 那条只做类型检查
grep -cE 'ubuntu-latest|windows-latest|macos-latest' .github/workflows/ci.yml   # ≥ 3

# 3. GitHub 专属开关在位
grep -qE '^concurrency:' .github/workflows/ci.yml && grep -q 'cancel-in-progress' .github/workflows/ci.yml
grep -q 'contents: read' .github/workflows/ci.yml

# 4. 兼容层遗产已清干净（只查非注释行）
grep -v '^[[:space:]]*#' .github/workflows/ci.yml | grep -cE 'uname|GITHUB_PATH|--prefix'   # 期望 0

# 5. 本地门禁没被改坏
just ready            # 期望退出码 0
```

**最终判据（本地证明不了）**：推上去之后 Linux / Windows / macOS 三个 job 全绿
（E2E 含 Victauri 冒烟与集成测试）。runner 环境的差异 ——
apt 包可用性、xvfb 能否起 app、macOS SDK、Windows 上 Tauri 构建脚本 ——
只有在那里才见分晓。

## 回滚

```bash
git revert <commit>
```

CI 配置改动只影响门禁，不影响产物与用户数据。

## 实施记录

### 本地部分（2026-09-11）

1. YAML 解析通过（`js-yaml` 4.1.0 临时取到 `/tmp`，**未进仓库**）：
   `checks-linux`（`ubuntu-latest`）、`checks-other`（`${{ matrix.os }}`，矩阵 windows/macos）、
   `e2e`（`needs: checks-linux`）
2. `just ready` → 退出码 0
3. 四个 just 资产 URL 全部 **HTTP 200**（Linux musl / macOS aarch64 / macOS x86_64 / Windows msvc）

### 与初稿不同的三处（都有理由）

- **从"一个矩阵 job"拆成"Linux job + 其余平台矩阵 job"**：E2E 只挂在 Linux 上，
  拆开才能让"平台检查红"与"E2E 红"互不吞并。
- **删掉原先单列的 `just docs-check` 步骤**：`just ready` 本来就包含它 —— 同一件事只定义一处。
  超时 30 → **45 分钟**。
- **E2E 里的 `needs` 只留 `checks-linux`**，理由见步骤 3。

### 第二轮：把 GitHub 专属能力吃透（2026-09-11）

放弃兼容层之后才腾得出手，逐条都是"省时间 / 省钱"，其中第一条是**省钱的主力**：

| 改动 | 为什么 |
|---|---|
| `concurrency` + `cancel-in-progress`（`main` 除外） | 连推三次只跑完最后一次；旧运行立刻让出 runner |
| `permissions: contents: read` | 最小权限（本工作流本来就只读仓库） |
| `defaults.run.shell: bash` | 与本地配方的 bash shell 对齐；Windows 上不再逐步写 `shell:` |
| 两个 job 的手写 just 安装脚本 → `install-action` | **净删 50 行 shell**；顺带拿到 SHA256 / attestation 校验，版本 `just@1.58.0` 写死 |
| `npm install -g @ast-grep/cli`（去掉 `--prefix`） | 那是 job 容器里绕全局写权限的写法，GitHub runner 无此问题 |

（`JUST_VERSION` 环境变量随之删除：版本现在写在 `tool:` 里。）

本轮的本地验证：YAML 解析通过；`top-level: name, on, concurrency, permissions, defaults, env, jobs`；
`checks-linux` 7 步（工具三件套一次装机）、`checks-other` 5 步、`e2e` 10 步；
非注释行里 `uname|GITHUB_PATH|--prefix` 计数为 **0**；`just ready` 退出码 0。

新增的两条**只在真 runner 上才见分晓**的风险：

1. `install-action` 装 `just`（它确实在支持列表里，源码是
   `taiki-e/install-action` 的 `TOOLS.md`）—— 第一次在本仓库这么用，注意看它是否落在
   `$CARGO_HOME/bin` 且能被后续步骤找到；
2. `permissions: contents: read` 是否够 `victauri-test` 那个 composite action 用
   （不够就在这里补一条并写明理由，别直接删掉整个 `permissions`）。

### 放弃记录：双 forge 兼容层（2026-09-11）

**试过什么**：让同一份 `.github/workflows/ci.yml` 同时喂 Gitea Actions 与 GitHub Actions
（Gitea 先验、GitHub 后推），并用一个 `ci-check` 配方（曾并入 `ready`）把约束变成机器检查。

**为什么放弃**：代价落在整份工作流上，收益只是一次本地预演。为了让工作流待在两边
**共有的子集**里，必须长期遵守下面四条，而它们**与 GitHub 无关**，纯粹是给兼容性交的税；
约束不是被违反了才疼，而是**写的时候就不能用**（`runner.*`、复杂 `runs-on`、除 `always()`
之外的表达式函数、常规矩阵里的 Windows/macOS）。第四条更尴尬：Windows / macOS 需要用
`github.server_url == 'https://github.com'` 在 **job 级**门住（步骤级 `if` 拦不住排队），
于是 GitHub 与 Gitea 上的 job 结构**本来就不一样** —— "一份文件两个 forge"名存实亡。

**保留的约束清单**（万一将来真要在别的 forge 上跑，先读这段）：

1. **不要建 `.gitea/workflows/`**：Gitea 的 `[actions] WORKFLOW_DIRS` 默认是
   `.gitea/workflows,.github/workflows`，**只读第一个存在的目录** —— 建了它反而让 Gitea
   忽略 `.github/workflows/`，CI **静默不跑**（出处：go-gitea/gitea PR #36619）。
2. **不用 `runner.*` 上下文**（Gitea 的上下文表里只有 `github.*` / `gitea.*`）；
   OS / 架构改用矩阵值、`uname`、`GITHUB_*` 环境变量。
3. **`runs-on` 只用简单形式**（`xyz` / `[xyz]`）；**表达式函数只用 `always()`**
   （`contains()` / `format()` / `fromJSON()` 都不可用）。
4. **没有 Windows / macOS runner**（Gitea runner 是 Linux 容器）：放常规矩阵会一直排队等
   一个永远不来的 runner，只能用 `github.server_url` 整条 job 门住。

**当时的机器检查（已删除）**：`ci-check` 只覆盖文本层能确定的部分，
**真实行为仍以实跑为准** —— 它给出的是"没写错"的下界，不是"跑得通"的证明。
删掉它是这个决定的一部分：留着就等于承认兼容层还在维护清单里。

**跟着一起删掉的还有那些"迁就写法"**：手写的按平台选资产脚本（因为当时不能用
`${{ runner.arch }}`）、`runs-on` 的简化形式、job 级 `github.server_url` 门、
`npm --prefix` 绕权限。它们的共同点是**看起来更"稳"，其实只是在替兼容层还债** ——
见到这类写法先问一句"它是为哪个 forge 写的"。

### 最终判据仍未达成 —— 所以状态是「进行中」

- **仓库当前没有配置任何 git remote**（`git remote -v` 为空）：这不是"等一次推送"，
  而是等托管到位。
- 三条**只在真 runner 上才见分晓**的风险：
  1. `actions/checkout` 与 `Swatinem/rust-cache` 的缓存是否如预期命中；
  2. E2E 能否在 runner 上起 webkit2gtk + xvfb 的窗口；
  3. Windows 上 Tauri 的构建脚本能否过 `cargo check`（不过就按步骤 10 单独决策）。
- 阶段 0 的「CI 变绿」条目（`ROADMAP.md` 里标 `[~]`）挂的是同一条。
