# Plan 0106: Rust 成员收进 `src-tauri/`

- **关联**：ROADMAP 阶段 1 ·「Rust 成员收进 `src-tauri/`（取代根 workspace 布局）」
- **前置**：plan 0101（根 workspace，本次要把它挪进 `src-tauri/`）、plan 0104（监听范围实测）
- **状态**：已完成（2026-09-11）
- **决策**：[ADR-0004](../adr/0004-rust-workspace-under-src-tauri.md)（取代 ADR-0001 决策一）
- **影响面**：`Cargo.toml` / `Cargo.lock` / `deny.toml` / `target/` 的位置、`.gitignore`、
  `src-tauri/justfile`、`src-tauri/tauri.conf.json`、`.ast-grep/rules/*`、
  `AGENTS.md` §1/§3.1/§7/§9/§11、`ROADMAP.md`、`docs/`（scope / just / plans / STATUS / ADR）

## 目标

**仓库根不再有任何 Rust 成员或 manifest**：workspace root 改为 `src-tauri/Cargo.toml`，
纯逻辑 crate 落在 `src-tauri/crates/*`。

顺带消掉一条**静默约束**：tauri CLI 默认只监听 `src-tauri`，成员在它里面就自动被监听，
`build.additionalWatchFolders` 不再需要（它配错时只打一行警告，见 plan 0104 / 坑 #21）。

## 非目标

- **不**改任何 Rust 源码（`src-tauri/src/**`、`src-tauri/crates/**` 零改动）
- **不**改 workspace 的成员构成（仍是一个 app + 两个纯逻辑 crate）
- **不**调依赖版本，**不**动 tauri CLI 版本
- **不**重命名 crate

## 前置检查

```bash
git status --short          # 期望干净：下面的 git mv 需要精确可回滚
ls -d src-tauri/crates      # 期望：不存在（否则目标已被占）
just ready                  # 期望：全绿（作为迁移前基线）
```

## 步骤

1. `git mv crates src-tauri/crates`；`git mv Cargo.lock src-tauri/Cargo.lock`；
   `git mv deny.toml src-tauri/deny.toml`。
2. 删根 `Cargo.toml`，把 `[workspace]`（`members = ["crates/*"]`）、`[workspace.lints]`、
   `[profile.release]` 移进 `src-tauri/Cargo.toml` —— 本包即 workspace root，
   **不需要**把自己列进 members。
3. `src-tauri/justfile`：`--config ../deny.toml` → `--config deny.toml`（配置回到同目录）。
4. `.gitignore`：根删掉 `/target/`，`src-tauri/.gitignore` 加回 `/target/`。
5. `src-tauri/tauri.conf.json`：删 `build.additionalWatchFolders`（成员已在监听树内）。
6. `.ast-grep/rules/*`：`files:` 的 `crates/**/*.rs` → `src-tauri/crates/**/*.rs`。
7. `target/` 搬到 `src-tauri/target/`，**并删掉写死了旧绝对路径的构建脚本产物目录** ——
   搬运缓存会留下 `DEP_*` 里重放的旧路径，不处理会在构建脚本阶段报"文件不存在"（坑 #19）。
8. 文档同步：新 ADR-0004 + ADR-0001 追加取代指针 + ADR 索引；`AGENTS.md` §1/§3.1/§7/§9/§11；
   `ROADMAP.md`；`docs/scope.md`、`docs/just.md`、`docs/plans/**` 的路径；
   `docs/plans/archive/README.md` 说明归档 plan 的路径按当时布局书写。
9. CI 无需改路径：它一直显式用 `--manifest-path src-tauri/Cargo.toml`。

## 验收命令

```bash
# 1. workspace root 与成员
cargo metadata --manifest-path src-tauri/Cargo.toml --no-deps --format-version 1 \
  | jq -r '.workspace_root, .target_directory, (.packages[].name)'
#    期望：…/akasha/src-tauri、…/akasha/src-tauri/target、akasha-core、akasha-pty、akasha

# 2. 根目录没有 Rust 成员
test ! -e Cargo.toml && test ! -e Cargo.lock && test ! -d crates && test ! -d target \
  && echo "OK: 根目录零 Rust 成员"

# 3. 门禁仍全绿（含 workspace 全成员的 check / clippy / test）
just ready

# 4. 依赖门禁的配置路径仍找得到
just deny-offline     # 期望 bans ok, licenses ok, sources ok
```

**规则的路径变了，必须重新用负例验证**（否则分不清"规则在工作"与"规则写错了"）：

```bash
cat > src-tauri/crates/akasha-core/src/negprobe.rs <<'EOF'
pub struct NegTabProbe;
pub struct Previewer;
use tauri::AppHandle;
EOF
ast-grep scan src-tauri/crates/akasha-core/src/negprobe.rs
#   期望：NegTabProbe 命中 no-ui-vocab-in-types；use tauri:: 命中 no-tauri-in-core-crates；
#         诱饵 Previewer 不命中
rm src-tauri/crates/akasha-core/src/negprobe.rs
```

**开发循环的监听范围**（本 plan 的直接收益）：

```bash
just dev            # 常驻；CLI 只打印一行 Watching …/src-tauri
printf '\n// watch-probe\n' >> src-tauri/crates/akasha-pty/src/lib.rs
#   期望：CLI 出现 `File src-tauri/crates/… changed. Rebuilding application...` + 重启
```

## 回滚

```bash
git revert <commit>
```

后续还有一步手工动作：把 `src-tauri/target/` 搬回仓库根（或用 `CARGO_TARGET_DIR` 指过去），
因为 git 不管构建产物。源码与配置本身无数据迁移，回滚无残留。

## 实施记录

### 验收命令的实际输出（2026-09-11）

1. `cargo metadata --manifest-path src-tauri/Cargo.toml …` →
   `/home/lycurgus/akasha/src-tauri`、`/home/lycurgus/akasha/src-tauri/target`、
   `akasha-core` / `akasha-pty` / `akasha`
2. `just ready` → `✅ 全绿（5/5）`（`lint` 14s、`test` 40s：缓存搬运后首次要补编译）
3. `just deny-offline` → `bans ok, licenses ok, sources ok`
4. 根目录零 Rust 成员 ✓（`Cargo.toml` / `Cargo.lock` / `crates/` / `target/` 均不在根）

### 规则负例（路径改了必须重验）

在 `src-tauri/crates/akasha-core/src/negprobe.rs` 放探针后：

| 探针 | 期望 | 实际 |
|---|---|---|
| `pub struct NegTabProbe;` | 命中 `no-ui-vocab-in-types` | ✅ 命中 |
| `pub struct Previewer;` | **不**命中 | ✅ 未命中 |
| `use tauri::AppHandle;` | 命中 `no-tauri-in-core-crates` | ✅ 命中 |
| `tauri::Builder::default()`（内联路径） | 命中 | ✅ 命中 |

探针随后删除，整树 `ast-grep scan` 退出码 0。

### 开发循环：本 plan 的主要收益

```
Info Watching /home/lycurgus/akasha/src-tauri for changes...      ← 只有一行
Info File src-tauri/crates/akasha-pty/src/lib.rs changed. Rebuilding application...
Running `target/debug/akasha`
```

成员在 `src-tauri` **里面**，默认监听就覆盖它们 —— `build.additionalWatchFolders`
已从 `tauri.conf.json` 删除。这条静默约束（配错只打一行 `Warn`）从此不存在。

### 两个坑

- **`target/` 是搬过来的，不是重建的**：13G 缓存同级 `mv` 是瞬时的，但
  `target/debug/build/*/output` 里有 13 处写死的旧绝对路径
  （`…/akasha/target/…`），cargo 会把它们当 `DEP_*` 重放给下游 —— 处置同坑 #19：
  删掉那些构建脚本产物目录让它们重跑，然后 `just ready` 转绿。
- **cargo 产物文件名带 workspace root 的痕迹**：`Running target/debug/akasha`
  从"相对仓库根"变成了"相对 `src-tauri`"，因为 CLI 的 cwd 就是那里。
  这只影响日志观感，不影响产物位置。
