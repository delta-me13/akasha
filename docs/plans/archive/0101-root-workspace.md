# Plan 0101: 落地根 workspace

- **关联**：ROADMAP 阶段 1 ·「落地 ADR-0001 决策一」（`docs/adr/0001` 决策一）
- **前置**：ADR-0001 已接受（**已满足**，决策二裁定见其 §0.3）
- **状态**：已完成（2026-09-11）
- **影响面**：新增根 `Cargo.toml`、根 `deny.toml`；移动 `Cargo.lock`；`.gitignore`；根 `justfile`（`--config` 路径）
- **后继**：plan 0104（迁移后必须复测开发循环，本 plan **不做**这件事）

## 目标

在仓库根建立 Cargo workspace，把 `src-tauri` 降为一个成员。

同时消失的一类问题：仓库根没有 `Cargo.toml`，导致一切未显式指定 manifest 的 cargo 命令在根目录失败
（`cargo build` / `cargo metadata` / `cargo fmt --all`，见问题 #8 与 `docs/adr/0001` §2.2）。

## 非目标

- **不**创建 `crates/akasha-core` / `crates/akasha-pty`（那是 plan 0103 / 0105）
- **不**改业务代码（`src-tauri/src/**` 零改动）
- **不**调 Rust 工具链，**不**动 Tauri CLI 版本（那是独立决策）

## 前置检查

```bash
# 期望看到「已接受（Accepted」
grep -n '状态' docs/adr/0001-crate-split-and-pty-abstraction.md | head -3
# 期望干净：未提交改动会让下面的 git mv 难以精确回滚
git status --short
```

## 步骤

1. 新建根 `/Cargo.toml`：

   ```toml
   [workspace]
   resolver = "2"
   members = ["src-tauri", "crates/*"]

   [workspace.lints.rust]
   unsafe_code = "forbid"

   [workspace.lints.clippy]
   unwrap_used = "warn"
   ```

   ⚠️ **实测点**：`crates/` 此刻不存在。确认空 glob `"crates/*"` 是否被 cargo 接受；
   若报 `failed to load manifest for workspace member`，先只写 `members = ["src-tauri"]`，
   等 plan 0103 创建首个成员时再加回。

2. `src-tauri/Cargo.toml` 加 `[lints] workspace = true` —— 否则 `[workspace.lints]` 形同虚设。

3. `git mv src-tauri/Cargo.lock Cargo.lock`；根 `.gitignore` 加 `/target/`，
   `src-tauri/.gitignore` 去掉 `/target/`（保留 `gen/schemas` 那行）。

4. `git mv src-tauri/deny.toml deny.toml`，同步根 `justfile` 里 `--config` 的路径。

5. 根 `justfile`：crate 级命令已在 `src-tauri/justfile`，而 just 用 justfile 所在目录作 cwd，
   **大概率无需改动**。只需确认 `cargo fmt --all` 与 `cargo nextest run` 覆盖全部成员。
   （本 plan 初稿曾写"把 manifest 切到 workspace" —— 那是 justfile 搬迁前的写法，已作废。）

6. CI：`.github/workflows/ci.yml` 已显式传 manifest 并用 `cargo metadata` 解析 target 目录，
   A / B 两种布局都成立。只需确认 e2e job 仍能启动 `$TARGET_DIR/debug/akasha`。

## 验收命令

```bash
# 1. 根目录成为 workspace root
cargo metadata --no-deps --format-version 1 | jq -r '.workspace_root, (.packages[].name)'
#    期望：/home/lycurgus/akasha 换行 akasha

# 2. 质量门禁仍全绿
just ready            # 期望退出码 0
just deny-offline     # 期望 bans ok, licenses ok, sources ok

# 3. target 已移到根
test -d target && echo "OK: target 在根"
test ! -d src-tauri/target && echo "OK: 旧 target 已消失"
```

> 启动窗口、热重载、IPC 的复测**不在这里** —— 见 plan 0104。
> 本 plan 只保证"结构对了且门禁绿"，**不得**用它的验收冒充开发循环验过。

## 回滚

```bash
git revert <commit>
```

新增 1 个文件、移动 2 个、改 3 个配置；无数据迁移、无代码改动，回滚无残留。

⚠️ 不得用 `git checkout <file>` 去"还原"**未提交**的改动 —— 它是破坏性操作（问题 #12）；
负例自检请用 `cp` 备份 + `cp` 还原。

## 实施记录

### 验收命令的实际输出（2026-09-11）

1. `cargo metadata --no-deps --format-version 1 | jq -r '.workspace_root, (.packages[].name)'`
   → `/home/lycurgus/akasha`、`akasha`；`.target_directory` = `/home/lycurgus/akasha/target`
2. `just ready` → `✅ just ready 全绿（5/5）`
3. `just deny-offline` → `bans ok, licenses ok, sources ok`
4. `test -d target` → OK；`test ! -d src-tauri/target` → OK
5. CI（e2e job）的产物路径解析仍成立：
   `cargo metadata --manifest-path src-tauri/Cargo.toml … | jq -r .target_directory`
   → `/home/lycurgus/akasha/target`，`target/debug/akasha` 存在 → **e2e job 无需改动**

### 实测点：plan 预判的

- ⚠️ **空 glob 是硬错误，不是空匹配**：`crates/` 不存在时
  `members = ["src-tauri", "crates/*"]` 报 `failed to load manifest for workspace member …/crates/*`。
  按步骤 1 的回退先只写 `["src-tauri"]`；`crates/*` 由 plan 0103 建出首个成员后加回。
- ✅ `resolver = "2"` 在 edition 2024 成员下被接受，无警告。
- ✅ `--config` 的**实际位置**不在根 justfile，而在 `src-tauri/justfile`（crate 级命令住在那里，
  `AGENTS.md` §11）→ 改为 `--config ../deny.toml`。plan 步骤 4 的措辞与实际结构不符，按实际改。

### 实测点：plan 没预判的两个

- ⚠️ **`[profile.release]` 留在成员里会被静默忽略**：cargo 只认 workspace root 的那份
  （`warning: profiles for the non root package will be ignored`）→ 上移到根 `Cargo.toml`。
  不迁等于**静默丢掉 lto / strip / panic=abort**，且没有任何门禁会红（问题 #18）。
- ⚠️ **整体 `mv` 构建缓存会留下写死的绝对路径**：`mv src-tauri/target target` 省下 9.8G 重建，
  但 14 个包的 `target/debug/build/<pkg>/output` 里记录着 `…/src-tauri/target/…`，
  而 cargo 会把这些 `DEP_*` 原样重放给下游 —— 于是 tauri 的构建脚本去读一个已不存在的
  permissions 目录，报错看起来像"代码坏了"。处置：删掉那 14 个构建脚本产物目录让它们重新运行
  （不必全量重建）。**下次迁 target 应直接删掉重建**，不得为了省时间而搬运缓存（问题 #19）。

### workspace lints 已实际生效

`unsafe_code = "forbid"` + `clippy::unwrap_used = "warn"`，配上 `just clippy` 的 `-D warnings`
= **clippy 里 warn 即错误**。7 处违规全部落在两个 E2E 测试文件
（`tests/smoke.rs` 5 处、`tests/integration.rs` 2 处）→ 在那两个文件加
`#![allow(clippy::unwrap_used)]` 并写明理由：测试里 unwrap 就是断言手段，不属于 `AGENTS.md` §0
说的「command 边界或长驻任务」。**生产代码零豁免** —— 正是这条 lint 想要的区分。

### 一并清掉的

`src-tauri/target`（9.8G）整体移到根后旧目录消失，`.gitignore` 的 `/target/` 规则也随之
从 `src-tauri/.gitignore` 上移到根。

