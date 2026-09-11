# ADR-0004：Rust workspace 收在 `src-tauri/` 下

- **状态**：**已接受**（Accepted，2026-09-11）
- **日期**：2026-09-11
- **决策者**：cyrene
- **影响范围**：仓库布局、`Cargo.toml` / `Cargo.lock` / `deny.toml` / `target/` 的位置、
  CI 的 manifest 路径、`just` 配方的 cwd、开发循环的监听范围
- **取代**：[ADR-0001](./0001-crate-split-and-pty-abstraction.md) 的**决策一**（根 workspace）。
  ADR-0001 的其余部分（分层、命名、依赖方向、决策二）**仍然有效**。

---

## 1. 背景

ADR-0001 决策一选择了"仓库根作 workspace root、crates 放仓库根 `crates/`"（2026-09-11 落地，
见 plan 0101 与 `docs/plans/archive/0101-root-workspace.md`）。落地后出现两件事：

1. **根目录因此堆着 Rust 的东西**：`Cargo.toml`、`Cargo.lock`、`target/`（迁移时 9.8G），
   而仓库根同时是前端（`src/`、`package.json`、`vite.config.ts`）与文档的家。
2. **开发循环差点静默失效**：tauri CLI 默认只监听 `src-tauri`，而成员在根 `crates/` ——
   改 `crates/` 不触发任何重编译。当时靠 `build.additionalWatchFolders` 补上
   （实测与排错见 plan 0104、`STATUS.md` 坑 #21）。

## 2. 决策

**Rust 成员全部收进 `src-tauri/`，仓库根不再有任何 Rust 成员或 manifest。**

```
src-tauri/
├── Cargo.toml          # ← workspace root 就是它（同时是 app 包）
├── Cargo.lock          # workspace 的锁文件
├── deny.toml           # cargo-deny 配置（与 manifest 同目录）
├── target/             # cargo 产物
├── crates/             # 纯逻辑 crate（零 Tauri 依赖）
│   ├── akasha-core/
│   └── akasha-pty/
├── src/                # IPC 薄壳
└── justfile            # crate 级命令（cwd = src-tauri，天然找得到 manifest）
```

- `src-tauri/Cargo.toml` 里写 `[workspace] members = ["crates/*"]`；本包即 workspace root，
  不需要把自己列进 members。
- 仓库根**没有** `Cargo.toml`。

## 3. 理由

1. **开发循环的监听范围自动成立**：tauri CLI 监听 `src-tauri`，成员在它**里面**，
   于是"改成员 → 重编译 + 重启"不需要任何额外配置。这消除了一个
   **静默失效**面（门禁全绿但改了代码没反应），比省一条配置重要得多。
2. **根目录回归它本来的职责**：前端、文档、脚本。Rust 的东西一件都不散在根。
3. **`cargo` 的默认行为与目录结构一致**：在 `src-tauri/` 跑任何 `cargo …` 都指向同一个
   workspace；`just` 的 cwd 规则（justfile 所在目录）与 manifest 位置重合，
   crate 级配方继续不需要 `--manifest-path`。

## 4. 代价与对策（照实记下）

| 代价 | 对策 |
|---|---|
| 仓库根没有 manifest，在**根目录直接跑 `cargo …` 会失败**（`docs/STATUS.md` 坑 #8 回归） | 一律走根 `justfile` 的转发配方（`just check` / `just test` / …）；CI 里显式 `--manifest-path src-tauri/Cargo.toml` |
| 纯逻辑 crate 在物理上位于 app 目录内部，容易被误读为"属于 Tauri" | **依赖方向**才是架构约束，不是目录位置：`crates/*` 不得 `use tauri::`，由 `.ast-grep/rules/no-tauri-in-core-crates.yml` 强制（AGENTS.md §3.1） |
| 迁移 `target/`（搬运会留下写死的旧绝对路径） | 见 `STATUS.md` 坑 #19 的处置；新机器上直接重建更省事 |

## 5. 被否掉的替代方案

| 方案 | 为什么不选 |
|---|---|
| 保持 ADR-0001 决策一（根 workspace + 根 `crates/`） | 根目录堆 Rust 成员；且必须额外配 `additionalWatchFolders` 才能让开发循环生效，而那是一条**静默**约束（配错只是警告） |
| 根只留一个**虚拟** `Cargo.toml`（`members = ["src-tauri", "src-tauri/crates/*"]`） | 保住了"根目录 cargo 可用"，但根目录仍然有一个 Rust 文件，与本次要求相悖；且 `Cargo.lock` / `target/` 仍在根 |
| 成员放 `src-tauri/crates/`，但 workspace root 设在根 | 多一份 manifest 要维护，且 `Cargo.lock`/`target` 位置又与 `src-tauri` 分离 —— 复杂度换不到实际好处 |

## 6. 复审条件

- 若出现**第二个可独立构建的应用**（例如 Android 端的独立 crate 树），
  重新评估"一个 workspace 覆盖全部"是否还成立。
- 若 `src-tauri` 下的无关成员多到让 `cargo check` 明显变慢，
  再考虑拆成多个 workspace（而不是把成员挪回根目录）。
