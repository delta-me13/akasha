# Plan 0102: CI —— 平台矩阵 + **双 forge（Gitea 先验，GitHub 后推）**

- **关联**：ROADMAP 阶段 1 ·「CI 平台矩阵」
- **前置**：plan 0101 / 0106（manifest 位置；矩阵里每条都跑 workspace 级检查）
- **状态**：进行中（本地部分已完成 2026-09-11；**最终判据待 CI 实跑** —— 先在 Gitea 上验，见「实施记录」）
- **影响面**：`.github/workflows/ci.yml`、根 `justfile`（新增 `ci-check`）、`docs/just.md` §2、`AGENTS.md` §7/§9

## 目标

一份工作流，**Gitea 与 GitHub 都能跑**；Linux 跑完整门禁与 E2E，Windows / macOS 通过类型检查。

两条都重要：平台差异（PTY / serial / ssh）是**主体工作量**，留到收尾等于把最大的坑留在最晚
（`scope.md` §9）；而"CI 只在 GitHub 上能跑"意味着**日常验证发生在一个你还没推过去的 forge 上**。

## 非目标

- **不**在 Windows / macOS **出包**：打包需要真实主机（WiX / NSIS / WebView2 bootstrapper 都不行）
- **不**引入 `cargo-xwin`：`cargo check --target` 已覆盖"挡 cfg 错误"这个 90% 诉求；
  它解决的是**链接**问题，救不了 Windows 打包
- **不**维护两份工作流文件：两份必然漂移（见「步骤 1」的替代方案与取舍）

## 前置检查

```bash
git remote -v                                   # 现在是空 —— CI 还没在任何 forge 上跑过
grep -nE 'runs-on|needs:|if:' .github/workflows/ci.yml
just ci-check                                   # 双 forge 约束的机器检查
```

## 步骤

1. **一份文件，两个 forge**。Gitea 的 `[actions] WORKFLOW_DIRS` 默认是
   `.gitea/workflows,.github/workflows`，且**只读第一个存在的目录** —— 所以只要不创建
   `.gitea/workflows/`，`.github/workflows/ci.yml` 两边都生效。**不要**为了"兼容 Gitea"
   去建那个目录：它会反过来让 Gitea 忽略 GitHub 目录，CI **静默不跑**（坑 #24）。
2. **避开 Gitea 不支持的东西**（出处：Gitea 的 *Compared to GitHub Actions* 与 *Variables* 文档）：
   - 不用 `${{ runner.* }}` 上下文（Gitea 的上下文表里只有 `github.*` / `gitea.*`）→
     OS / 架构改用矩阵值、`uname`、`GITHUB_*` 环境变量；
   - `runs-on` 只用简单形式（`xyz`）；
   - 表达式函数只用 `always()`（`contains()` / `format()` / `fromJSON()` 都不要用）。
3. **Windows / macOS 独立成 job，用 `github.server_url == 'https://github.com'` 整条门住**。
   Gitea runner 是 Linux 容器、**没有** Windows/macOS runner：放进常规矩阵的话，
   那边会一直排队等一个永远不来的 runner。门在 **job 级**（不是 step 级）——
   step 级拦不住"job 已经被调度、在等 runner"。
4. **E2E 只 `needs` Linux 那个 job**。被跳过的 job 会让依赖它的 job 一并跳过 ——
   若 `e2e` 同时 needs 被门住的 job，Gitea 上连 E2E 都不会跑。
5. Linux 装 apt 依赖（含 `libayatana-appindicator3-dev` 与 `libxdo-dev`，坑 #10）后跑
   `just ready`（完整门禁只跑一次，避免矩阵把时间乘三）；其余平台只跑 `just check`。
6. 缓存 `~/.cargo` 与 `target`，**按平台分键**（`Swatinem/rust-cache` 的键自动含 OS 与 rustc 版本）。
7. **把约束变成机器检查**：新增 `just ci-check`（并入 `ready`）。破了约束的那天，
   本地不会有任何门禁变红 —— 恰恰是"CI 跑不起来但没人知道"的经典形态。
8. Windows / macOS 若在 `cargo check` 阶段因 Tauri 的构建脚本失败：记录到「实施记录」并**单独决策**，
   不要就地绕开（绕过等于把平台差异藏起来，正是本 plan 要消灭的东西）。

## 验收命令

```bash
# 1. 双 forge 约束（无 runner 上下文 / 非 Linux 有 github.server_url 门 / 三平台齐全 / 无抢占目录）
just ci-check

# 2. YAML 能解析，job 结构符合预期
node -e 'const y=require("js-yaml"),fs=require("fs");const w=y.load(fs.readFileSync(".github/workflows/ci.yml","utf8"));for(const [k,v] of Object.entries(w.jobs))console.log(k, JSON.stringify(v["runs-on"]), JSON.stringify(v.if??null), JSON.stringify(v.needs??null))'

# 3. 本地门禁没被改坏
just ready            # 期望退出码 0
```

**最终判据（本地证明不了）**：**Gitea 上 CI 全绿之后**，GitHub 上三个 job 也全绿。
runner 环境的差异（apt 包可用性、xvfb 能否起 app、macOS SDK、Windows 上 Tauri 构建脚本）
只有在那里才见分晓。顺序是刻意的：**先在 Gitea 上把流程跑通，再推 GitHub**。

## 回滚

```bash
git revert <commit>
```

CI 配置改动只影响门禁，不影响产物与用户数据。`ci-check` 一并回退即可。

## 实施记录

### 本地部分（2026-09-11）

验收命令的实际输出：

1. `just ci-check` → 通过；**两条负例都如实报红**：临时加一行 `${{ runner.os }}` → 红；
   建了一个空的 `.gitea/workflows/` 目录 → 红（都是 `cp` 备份 / `rmdir` 还原，不用 `git checkout`）
2. YAML 解析通过（`js-yaml` 4.1.0 临时取到 `/tmp`，**未进仓库**）：
   `checks-linux`（`runs-on: ubuntu-latest`）、
   `checks-other`（`${{ matrix.os }}`，`if: github.server_url == 'https://github.com'`，
   矩阵 = windows/macos）、`e2e`（`needs: checks-linux`）
3. `just ready` → `✅ 全绿（5/5）`
4. 四个 just 资产 URL 全部 **HTTP 200**（Linux musl / macOS aarch64 / macOS x86_64 / Windows msvc）

### 与初稿不同的四处（都有理由）

- **资产按 `uname -s`-`uname -m` 选，不用 `${{ runner.arch }}`**：既避开 Gitea 的上下文缺口，
  又能正确处理 `macos-latest` 已是 arm64 这件事 —— 装错架构会以
  `bad CPU type in executable` 失败，而那个报错里根本不提架构。未覆盖的组合**直接 exit 1**，
  不退回一个"大概能用"的二进制。
- **从"一个矩阵 job"拆成"Linux job + 仅 GitHub 的矩阵 job"**：Gitea 上矩阵里的
  windows/macos 分支会排队等不到的 runner，而**步骤级 `if` 拦不住排队**（job 已被调度）。
- **`e2e` 的 `needs` 从 `checks` 收窄为 `checks-linux`**：否则在被门住的 job 上
  连带跳过整个 E2E（见步骤 4）。
- **删掉原先单列的 `just docs-check` 步骤**：`just ready` 本来就包含它 ——
  同一件事只定义一处。缓存交给 `Swatinem/rust-cache`；超时 30 → **45 分钟**。

### 最终判据仍未达成 —— 所以状态是「进行中」

- **仓库当前没有配置任何 git remote**（`git remote -v` 为空）：这不是"等一次推送"，
  而是等托管到位。
- 三条**只在真 runner 上才见分晓**的风险，留在这里备查：
  1. Gitea 是否按预期在**调度前**求值 job 级 `if`（若它先把 job 排上队，Windows/macOS 分支
     仍会挂住 —— 届时改用 `vars.CI_EXTRA_PLATFORMS` 开关，并在 `ci-check` 里加一条）；
  2. `actions/checkout` 与 `Swatinem/rust-cache` 在 Gitea 的缓存服务上是否如预期工作；
  3. E2E 在 Gitea 的 job 容器里能否起 webkit2gtk + xvfb 的窗口。
- 阶段 0 的「CI 变绿」条目（`ROADMAP.md` 里标 `[~]`）挂的是同一条。
