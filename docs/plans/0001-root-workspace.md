# Plan 0001: 落地根 workspace

- **关联**：ROADMAP 阶段 0 · `docs/adr/0001` 决策一
- **状态**：未开始（**前置条件已满足**：ADR-0001 已接受，决策二裁定见其 §0.3）
- **预计影响**：`Cargo.toml`（新增）、`Cargo.lock`（移动）、`.gitignore`、`justfile`、CI、`deny.toml`

## 目标

在仓库根建立 Cargo workspace，把 `src-tauri` 降为一个成员；顺带修好当前必然失败的 CI。

## 非目标

- **不**创建 `crates/akasha-pty`（那是 ADR-0001 决策二/三的范围）
- **不**改任何业务代码（`src-tauri/src/**` 零改动）
- **不**调整 Rust 工具链（`rust-toolchain.toml` 是独立决策，见 `docs/STATUS.md`）

## 前置检查

```bash
# 确认 ADR-0001 已被接受（预期看到「已接受（Accepted」）
grep -n '状态' docs/adr/0001-crate-split-and-pty-abstraction.md | head -3
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

   ⚠️ **实测点**：`crates/` 目录此刻不存在，需确认空 glob `"crates/*"` 是否被 cargo 接受。
   若报 `failed to load manifest for workspace member`，就先只写 `members = ["src-tauri"]`，
   等真正创建 crate 时再加回来。

2. 在 `src-tauri/Cargo.toml` 里引用继承 lints：`[lints] workspace = true`
   （否则 `[workspace.lints]` 形同虚设）。

3. `git mv src-tauri/Cargo.lock Cargo.lock`，并在根 `.gitignore` 加 `/target/`，
   删除 `src-tauri/.gitignore` 里的 `/target/`（保留 `gen/schemas` 那行）。

4. `justfile`：**大概率无需改动**。crate 级命令已在 `src-tauri/justfile`，
   而 just 用 justfile 所在目录作为 cwd —— 采纳根 workspace 后它照样工作。
   只需确认 `cargo fmt --all` 与 `cargo nextest run` 在新布局下覆盖全部成员。
   （本 plan 初稿写"把 `MANIFEST` 切到 `--workspace`"——那是 justfile 搬迁前的写法，已作废。）

5. CI：**无需改动**。`.github/workflows/ci.yml` 已经显式传 `--manifest-path`，
   并用 `cargo metadata` 解析 target 目录（而不是猜 `src-tauri/target`）——
   这正是为了在 A/B 两种布局下都成立。只需确认 e2e job 仍能启动
   `$TARGET_DIR/debug/akasha`。

6. `git mv src-tauri/deny.toml deny.toml`，同步 `justfile` 里 `--config` 的路径。

## 验收命令（可直接粘贴执行）

```bash
# 1. 根目录成为 workspace root，且列出 ≥2 个包（当前只有 akasha，含未来 crates）
cargo metadata --no-deps --format-version 1 | jq -r '.workspace_root, (.packages[].name)'
#    期望：/home/lycurgus/akasha 换行 akasha

# 2. 质量门禁仍全绿
just ready            # 期望退出码 0
just deny-offline     # 期望 bans ok, licenses ok, sources ok

# 3. target 目录已移到根
test -d target && echo "OK: target at root"; test ! -d src-tauri/target && echo "OK: 旧 target 已消失"

# 4. Tauri CLI 在新布局下仍能工作（ADR 未决项 1，必须实跑，不能只看编译）
just dev              # 期望能起窗口；看输出里 target 路径与重启行为是否正常
```

**第 4 条是硬性的**：本 plan 的全部风险集中在"Tauri CLI 在 workspace 下的
target 路径与 dev watcher 行为"。只跑 `cargo check` 证明不了它。

### 迁移后必须逐项对比的基线（迁移前已实测，见 `docs/STATUS.md`）

| 项 | 迁移前 | 迁移后应为 |
|---|---|---|
| 二进制落点 | `src-tauri/target/debug/akasha` | `<root>/target/debug/akasha` |
| 冷编译 / 增量 | 47.53s / 6.09s | 同量级（不应变慢） |
| dev server | Vite 1420 | 1420，HTTP 200 |
| **CLI 监听范围** | `Watching .../src-tauri for changes` | **改为 `crates/` 下的文件后仍触发重编译 + 重启** ⚠️ |
| IPC 端到端 | `greet` 返回预期字符串 | 同样返回 |
| Victauri | 连上，35 工具 | 同样连上 |

⚠️ **监听范围是本次迁移最大的隐性风险**：CLI 打印的监听路径是 `src-tauri`，
而迁移后 `crates/*` 在它之外。如果 crate 改动不再触发重启，开发循环会**静默失效** ——
`cargo check` / `just ready` 全绿，但你改了代码看不到效果。
验证方法：跑起 `just dev`，改一下 `crates/` 下任意文件的注释，观察 CLI 是否重编译并重启。
若失效，就在 `src-tauri/Cargo.toml` 或 `.taurignore` 层面想办法（而不是默默接受）。

## 回滚

本 plan 的改动集中在一次提交里（新增 1 个文件、移动 2 个文件、改 3 个配置）：

```bash
git revert <commit>     # 或用 git checkout <commit>~ -- <files> 精确回退
```

无数据迁移、无代码改动，回滚无残留。

## 实施记录

（边做边追加，不要事后补。记录每条验收命令的**实际输出**。）
