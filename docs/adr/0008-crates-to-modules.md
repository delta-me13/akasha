# ADR-0008：crates 改为 `src-tauri/src` 下的模块

- **状态**：**已定案**（2026-09-19）
- **日期**：2026-09-19
- **决策者**：cyrene
- **影响范围**：仓库布局、`Cargo.toml` / `Cargo.lock` / `deny.toml`、`target/`、测试目标、
  ast-grep 护栏及其测例与快照、`just` 配方与 CI、`AGENTS.md` §3.1 / §6 / §11
- **取代**：[ADR-0004](./0004-rust-workspace-under-src-tauri.md)（workspace 布局）与
  [ADR-0001](./0001-crate-split-and-pty-abstraction.md) 的**决策一**（成员分层）。
  ADR-0001 的**命名规则**（后端类型名不得编码 UI 呈现方式）与**决策二**（`akasha-vt` 延后）
  仍然有效。

---

## 1. 背景

现状（ADR-0004 定案后的形态）：`src-tauri/Cargo.toml` 是 workspace root，
6 个成员住在 `src-tauri/crates/`，`src-tauri/src/` 是 18 个 IPC 薄壳模块。

| 成员 | 文件 | 行数 |
|---|---|---|
| `akasha-core` | 6 | 1,154 |
| `akasha-pty` | 9 | 2,640 |
| `akasha-serial` | 7 | 1,276 |
| `akasha-ssh` | 33 | 11,820 |
| `akasha-store` | 30 | 9,085 |
| `akasha-bw` | 12 | 3,022 |

成员边界换来的是硬约束（编译器拒绝反向依赖）。它在本仓库**没有兑现任何收益**：

1. **没有独立消费者、没有独立发布**：仓库是单应用单仓库，成员从不被外部依赖。
2. **成员之间本就在互相依赖**：`akasha-ssh` / `akasha-serial` 各自声明 `akasha-pty`，
   `akasha-bw` / `akasha-ssh` 各自声明 `akasha-store` —— 边界并未减少耦合，
   只把它变成一份份 `Cargo.toml` 里的路径依赖与版本号。
3. **摩擦是持续的**：6 份 `[lints] workspace = true`、`[features]` 分散在成员里、
   按成员选择测试目标（`cargo nextest run --package akasha-serial`）、
   `just` 配方与 CI 都要知道包名。
4. **拆分粒度对编译没有可测收益**：没有并行编译收益的量化证据，每改一处逻辑却要在
   "成员 → 壳"之间来回跳。

因此本条只保留**收益**的一半：纯逻辑仍然不与 Tauri 耦合，但约束的载体从
**crate 边界**换成**模块路径**。

## 2. 决策

**取消 Cargo workspace 与全部成员；`src-tauri` 是唯一 package，全部源码住在
`src-tauri/src/` 的模块树里。**

```
src-tauri/
├── Cargo.toml          # 唯一 manifest（不再是 workspace root）
├── Cargo.lock  deny.toml  target/
├── src/
│   ├── lib.rs main.rs bin/gen-types.rs      # 装配（可用 Tauri）
│   ├── bindings.rs lifecycle.rs tray.rs single_instance.rs prompt.rs
│   ├── session/   model.rs registry.rs event.rs ipc.rs
│   ├── config/    model.rs file.rs
│   ├── tunnel/    model.rs ipc.rs
│   ├── pty/       transport.rs batcher.rs pty.rs shell.rs teardown.rs watchdog.rs host.rs
│   ├── ssh/       ...（原 akasha-ssh）ipc.rs sftp.rs
│   ├── serial/    ...（原 akasha-serial）ipc.rs
│   ├── store/     ...（原 akasha-store）ipc.rs
│   └── bw/        ...（原 akasha-bw）ipc.rs
└── tests/              # 集成测试（含原成员的 tests/）
```

- **域内分层**：每个域模块里，纯逻辑沿用原文件名；**Tauri 侧统一叫 `ipc.rs`**
  （原 `src-tauri/src/<域>.rs`）。跨域装配留在顶层。
- **现代 mod 约定**：`foo.rs` + `foo/`，不写 `mod.rs`（现有 3 处 `mod.rs` 一并改写）。
- **同名合并**：原 `src-tauri/src` 与成员同名的 7 组（`config.rs` `session.rs`
  `serial.rs` `sftp.rs` `tunnel.rs` `watchdog.rs` `lib.rs`）各并入同一个域模块，
  不再保留两份同义文件。
- **纯逻辑模块不得 `use tauri::`**（原 `no-tauri-in-core-crates`）由 ast-grep 按
  `files:` 路径强制，`ipc.rs` 与顶层装配文件是**显式**例外。
- **`unsafe` 的唯一放行点**由 crate 换成模块路径：`src-tauri/src/store/**`。
- **成员级 `features` 上升为 package features**（`libudev` 等）；
  按成员选择测试目标的配方改为按 feature 或直接执行。

## 3. 理由

1. **约束的载体可以换，约束本身不必丢**：编译器的 crate 边界是硬约束，模块路径规则是
   软约束 —— 但本仓库已经有 ast-grep 这一层"把禁止变成机器检查"的机制（`AGENTS.md` §6），
   而原规则本来就是按路径写的。换载体后"纯逻辑不碰 Tauri"仍然可执行、可回归。
2. **消除持续摩擦**：一份 manifest、一份 lint 基线、一份 features、依赖只声明一次。
3. **域模块把"怎么做"与"为什么"放回同一处**：原先同名的两层（纯逻辑 / 壳）分散在两个
   crate 里，读一处必须同时知道另一处；合并后一个域目录内可读完。
4. **单仓库单应用**：没有第二个消费者时，成员边界只是维护成本。

## 4. 代价与对策（照实记下）

| 代价 | 对策 |
|---|---|
| **硬边界变软边界**：模块层面的反向依赖不再被编译器拒绝 | 由 ast-grep 路径规则 + 真实路径探针守住（`AGENTS.md` §6）；规则改了 `files:` 必须重做探针 |
| **增量编译粒度变粗**：改一行逻辑要重编整个 package | 用 `just watch`（bacon `check`）做秒级反馈；必要时再评估拆回 |
| `cargo nextest run --package akasha-serial --no-default-features` 这类"按成员执行"的配方失效 | 成员 feature 上升为 package feature，配方改按 feature 组合执行；判据（不链 libudev）不变 |
| 成员的集成测试要迁进 `src-tauri/tests/`，与现有 30 个 app 级 E2E 同名者会冲突 | 迁移时按域加前缀改名；`just test-e2e` 的目标清单同步 |
| `unsafe` 放行点、`no-ui-vocab-in-types` 的 `files:` 全部指向旧路径 | 三条规则、测例、快照与探针随代码同一次改动落地 |
| `Cargo.lock` 会变更（成员合并、features 并集） | 锁文件进仓库（`AGENTS.md` §9）；改动在同一个提交里给出 |
| 历史 plan / ADR 里大量 `crates/...` 路径不再成立 | **不追改已归档的 plan 与已定案 ADR**（它们记录当时的事实）；现状口径在 `STATUS.md` 与 `AGENTS.md` |

## 5. 被否掉的替代方案

| 方案 | 为什么不选 |
|---|---|
| 保持 6 个成员（ADR-0004 的现状） | 成员边界在本仓库没有兑现收益，只留下清单与按包选择的摩擦（§1 的四条） |
| 字面"平铺"成 `src-tauri/src/*.rs` | 97 个文件一场平铺：7 组与现有壳同名、8 个 basename 在成员内部就重名，必须大范围改名，且失去域边界 |
| 保留 workspace、成员只减到两个（纯逻辑 + IO） | 仍要维护成员 manifest、`[lints]` 继承与按包选择；"按域组织"的目标没有达成 |
| 用 `mod` 重导出把成员伪装成一个 crate | 目录与 manifest 仍在，两套边界并存；读代码的人要同时理解两套 |

## 6. 复审条件

- 出现**第二个消费者**（独立 CLI、Android 端复用纯逻辑、需要单独发布）→ 重新评估拆回成员。
- 单 package 的编译时间明显变差（改一行等很久，且 `bacon` 也压不住）→ 评估按域拆回。
- 纯逻辑模块被 Tauri 依赖"污染"到规则压不住（`ipc.rs` 例外清单持续变长）→ 说明域内分层选错了，
  改为"纯模块与壳目录分离"的布局。

## 修订记录

- 2026-09-19：写下（实现中）。
- 2026-09-19：转**已定案** —— 落地它的 plan 0109 已完成并归档（六个成员全部并入 `src/`，
  护栏、`just` 配方与文档同步）。理由由仓库所有者给出：单仓库开发，crate 边界未看到收益，
  反而带来成员之间的依赖问题。
