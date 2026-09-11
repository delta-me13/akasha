# Plan 0102: CI 平台矩阵（Linux + Windows + macOS）

- **关联**：ROADMAP 阶段 1 ·「CI 平台矩阵」
- **前置**：plan 0101（根 workspace 落地 —— 矩阵里三条都跑 workspace 级检查）
- **状态**：未开始
- **影响面**：`.github/workflows/ci.yml`；若新增配方，同步 `docs/just.md` §2

## 目标

三个平台都能通过**类型检查**；Linux 另跑完整门禁与 E2E。

把平台差异从"收尾才暴露"提前成"每次提交都暴露" —— PTY / serial / ssh 的平台差异是
**主体工作量**，留到最后等于把最大的坑留在最晚（`scope.md` §9）。

## 非目标

- **不**在 Windows / macOS **出包**：打包需要真实主机（WiX / NSIS / WebView2 bootstrapper 都不行）
- **不**引入 `cargo-xwin`：`cargo check --target` 已覆盖"挡 cfg 错误"这个 90% 诉求；
  它解决的是**链接**问题，救不了 Windows 打包
- **不**在矩阵里跑 E2E（xvfb + app 启动只在 Linux 做）

## 前置检查

```bash
sed -n '1,40p' .github/workflows/ci.yml          # 现有 job 结构
grep -nE 'runs-on|needs:' .github/workflows/ci.yml
```

## 步骤

1. 现有 `checks` job 改为 matrix：`ubuntu-latest` / `windows-latest` / `macos-latest`。
2. 每个平台跑类型检查（含 tests / benches），即 crate 级 `check` 的转发配方 —— 不要另写一套 cargo 命令。
3. Linux 分支额外装 apt 依赖（含 `libayatana-appindicator3-dev` 与 `libxdo-dev`，坑 #10），
   然后跑 `just ready`（完整门禁只跑一次，避免矩阵把时间乘三）。
4. `e2e` job 保持 Linux-only、`needs: checks`，仍在 xvfb 下真起 app。
5. 缓存 `~/.cargo` 与 `target`，**按平台分键**；不加缓存的话矩阵会把 CI 时长乘三。
6. Windows / macOS 若在 `cargo check` 阶段因 Tauri 的构建脚本失败：记录到「实施记录」并**单独决策**，
   不要就地绕开（绕过等于把平台差异藏起来，正是本 plan 要消灭的东西）。

## 验收命令

```bash
# 1. YAML 可解析，且 jobs 名字符合预期
node -e 'const y=require("js-yaml"),fs=require("fs");console.log(Object.keys(y.load(fs.readFileSync(".github/workflows/ci.yml","utf8")).jobs))'

# 2. 三个平台都出现在矩阵里
grep -cE 'ubuntu-latest|windows-latest|macos-latest' .github/workflows/ci.yml   # 期望 ≥3

# 3. 本地门禁没被改坏
just ready            # 期望退出码 0
```

**最终判据（本地证明不了）**：推送后 CI 上**三个矩阵 job 与 e2e job 全绿**。
runner 环境的差异（apt 包可用性、xvfb 能否起 app、macOS SDK）只有在那里才见分晓。

## 回滚

```bash
git revert <commit>
```

CI 配置改动只影响门禁，不影响产物与用户数据。

## 实施记录

（边做边追加；特别记录三个平台各自的**实际**失败原因与处置，这是后续阶段的参考。）
