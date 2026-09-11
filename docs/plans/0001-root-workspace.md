# Plan 0001: 落地根 workspace

- **关联**：ROADMAP 阶段 0 · `docs/adr/0001` 决策一
- **状态**：未开始（**前置条件：ADR-0001 决策一被接受 —— 当前仍是"提议"**）
- **预计影响**：`Cargo.toml`（新增）、`Cargo.lock`（移动）、`.gitignore`、`justfile`、CI、`deny.toml`

## 目标

在仓库根建立 Cargo workspace，把 `src-tauri` 降为一个成员；顺带修好当前必然失败的 CI。

## 非目标

- **不**创建 `crates/akasha-pty`（那是 ADR-0001 决策二/三的范围）
- **不**改任何业务代码（`src-tauri/src/**` 零改动）
- **不**调整 Rust 工具链（`rust-toolchain.toml` 是独立决策，见 `docs/STATUS.md`）

## 前置检查

```bash
# 确认决策一已被接受（ADR 状态不再是"提议"）
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

4. `justfile`：`MANIFEST` 相关配方切到 `--workspace`；`cargo nextest run --workspace`。
   保留 `--manifest-path` 写法也可行，但**必须统一**，不能一半一半。

5. CI（`.github/workflows/victauri.yml`）：
   - `cargo build` → 明确 `cargo build -p akasha`（`packages[0].name` 取到的不再必然是 bin）
   - 启动路径改为 `./target/debug/akasha`，不再靠 `cargo metadata | jq` 推断
   - 加 `ast-grep scan` 与 `just deny-offline`

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

**第 4 条是硬性的**：这条 planet 的全部风险集中在"Tauri CLI 在 workspace 下的
target 路径与 dev watcher 行为"，只跑 `cargo check` 证明不了它。

## 回滚

本 plan 的改动集中在一次提交里（新增 1 个文件、移动 2 个文件、改 3 个配置）：

```bash
git revert <commit>     # 或用 git checkout <commit>~ -- <files> 精确回退
```

无数据迁移、无代码改动，回滚无残留。

## 实施记录

（边做边追加，不要事后补。记录每条验收命令的**实际输出**。）
