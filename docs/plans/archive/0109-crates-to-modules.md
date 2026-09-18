# Plan 0109: crates 改为 `src-tauri/src` 下的模块

- **关联**：ROADMAP 阶段 1 ·「crates 改为 `src-tauri/src` 下的模块（ADR-0008）」
- **前置**：ADR-0008（实现中）；阶段 0 的门禁基线（`just ready` 6/6）
- **状态**：已完成

## 目标 / 非目标

**目标**

1. `src-tauri` 成为唯一 package：删除 6 个成员与 workspace 段。
2. 6 个成员的源码改为 `src-tauri/src/` 下的域模块，**现代 mod 约定**（`foo.rs` + `foo/`，无 `mod.rs`）。
3. 原 `src-tauri/src/<域>.rs` 的壳并入同域模块，统一叫 `ipc.rs`；同名 7 组不再保留两份。
4. 三条 ast-grep 规则、其测例与快照改到新路径，并重做**真实路径探针**。
5. `just` 配方与 CI 不再按包名选择；成员 feature 上升为 package feature。
6. 文档同步：`AGENTS.md` §0 / §3.1 / §6 / §11、`docs/just.md`、`docs/scope.md`、`docs/STATUS.md`、`ROADMAP.md`、`docs/adr/README.md`。

**非目标**

- 不改任何**行为**：本计划是纯粹的布局重排，不改 IPC 契约、不改事件名、不改错误文案。
- 不追改已归档的 plan 与已定案 ADR 里的旧路径（ADR-0008 §4）。
- 不做与本布局无关的清理（注释、命名、依赖升级）。

## 前置检查

```bash
just ready                      # 基线：6/6 全绿（记录输出）
just test 2>&1 | tail -5        # 基线：nextest 通过数与耗时
git status --short              # 干净工作区
```

## 步骤（每步都能独立验证）

1. **捕获基线**：`just ready` / `just test` 的读数（通过数、目标数）记进「实施记录」。
2. **建骨架并搬迁（纯移动）**：`git mv` 成员的 `src/*.rs` 到 `src-tauri/src/<域>/`；
   `pools/mod.rs` → `pools.rs` 形式的现代 mod；本步只移动 + 改 `mod` 声明，不动内容。
   验证：`cargo metadata` 仍列出成员时先不删 manifest，`just check` 仍绿（靠旧的 `use` 路径）。
3. **合并 manifest**：成员的 `[dependencies]` / `[features]` / `[lints]` 并入
   `src-tauri/Cargo.toml`，删除 `[workspace]` 段与 6 份成员 manifest；
   `default = ["libudev"]` 等上升为 package features。
   验证：`cargo metadata --no-deps` 只列出 `akasha` 一个包；`just check` 绿。
4. **重写引用**：`akasha_core::` 等 261 处（源 134 + 成员间 127）→ `crate::` 路径；
   集成测试 `use akasha_store::…` → `use akasha_lib::store::…`。
   验证：`! grep -rn "akasha_\\(core|pty|ssh|store|serial|bw\\)::" src-tauri --include=*.rs`。
5. **域内合并壳**：`git mv src-tauri/src/session.rs src-tauri/src/session/ipc.rs` 等 7 组同名合并；
   跨域装配（`lib.rs` `tray.rs` `lifecycle.rs` `single_instance.rs` `bindings.rs` `prompt.rs`）留顶层。
   验证：`just check` / `just clippy` 绿；`grep -rn "mod " src-tauri/src/lib.rs` 只剩域模块。
6. **迁测试**：成员的 `tests/*.rs` 进 `src-tauri/tests/`，与现有 app 级 E2E 同名的按域加前缀；
   `just test` 与 `just test-e2e` 的目标清单一并改。
   验证：`just test` 通过数 ≥ 基线；`just test-e2e` 目标清单覆盖原有全部 E2E。
7. **改护栏**：三条规则改名与改 `files:`/`ignores:`（`no-tauri-in-core-crates` →
   `no-tauri-in-pure-modules`；`no-unsafe-outside-store` → `src-tauri/src/store/**`；
   `no-ui-vocab-in-types` → 域模块路径），同步 `scripts/ast-grep/tests/` 与 `__snapshots__/`。
   验证：`just lint` 绿；**真实路径探针**（正例 + 诱饵各一，见「验收命令」）。
8. **改配方与 CI**：`just libudev-check` / `serial-check` 的 `--package akasha-serial` 改为
   feature 组合；`cargo … --workspace` 的措辞与用法按单包收敛；CI 的 `workspaces` 路径不变。
   验证：`just libudev-check` / `just serial-check` 退出码 0。
9. **文档同步**：`AGENTS.md` 的 §0 禁止条目 3、§3.1 分层、§6 规则表、§11 `--workspace` 段；
   `docs/just.md` §2 与 §9；`docs/scope.md` §1.3；`docs/STATUS.md` 覆盖写；
   `docs/adr/README.md` 标 ADR-0001 决策一 / ADR-0004 被 0008 取代。
   ⚠️ `AGENTS.md` 按 §8 **单独提交**。
10. **收口**：`just ready` 6/6；本 plan 状态改「已完成」并 `git mv` 进 `archive/`。

## 验收命令

```bash
# 1) 成员与 manifest 消失，只剩一个 package
test ! -d src-tauri/crates && echo "成员目录已消失"
ls src-tauri/crates/*/Cargo.toml 2>/dev/null || echo "成员 manifest 已消失"
cargo metadata --no-deps --manifest-path src-tauri/Cargo.toml | python3 -c 'import json,sys; p=json.load(sys.stdin)["packages"]; print([x["name"] for x in p])'
# 预期：['akasha']

# 2) 旧的 crate 路径引用归零
! grep -rn "akasha_\\(core|pty|ssh|store|serial|bw\\)::" src-tauri --include='*.rs' && echo "无旧路径引用"

# 3) mod 约定：没有 mod.rs
test -z "$(find src-tauri -name mod.rs)" && echo "无 mod.rs"

# 4) 护栏：三条规则在真实路径上仍然生效（正例必须命中、诱饵不得命中）
printf 'use tauri::AppHandle;\nfn f(_: AppHandle) {}\n' > src-tauri/src/pty/probe_rule_check.rs
printf 'use tauri::AppHandle;\nfn f(_: AppHandle) {}\n' > src-tauri/src/probe_rule_decoy.rs
ast-grep scan --json | grep -c 'probe_rule_check'   # 预期：≥1
ast-grep scan --json | grep -c 'probe_rule_decoy'   # 预期：0
rm src-tauri/src/pty/probe_rule_check.rs src-tauri/src/probe_rule_decoy.rs

# 5) 门禁
just lint        # clippy + ast-grep scan + ast-grep test
just test        # nextest：通过数 ≥ 基线
just ready       # 预期：6/6
```

## 回滚

- 整个改动落在连续的提交里（骨架搬迁 / manifest / 引用 / 护栏 / 文档各一批），
  `git revert` 按批次回退到 ADR-0004 的布局即可；没有数据迁移、没有 IPC 契约变化，
  回滚不涉及用户数据。
- 编译产物：`src-tauri/target/` 在新布局下重建（成员消失会改变 fingerprint），
  回滚后第一次 `cargo check` 会重新编译一次。

## 实施记录

（边做边追加：每步的实际命令输出、nextest 读数、`just ready` 的最终读数。）

### 2026-09-19 第 1 轮

- **基线（HEAD `85c976a`）**：`#[test]` / `#[tokio::test]` 属性 451 个；`tests/` 下 64 个文件；
  `just check` 绿。沙箱里 `cargo nextest run -p akasha --lib` 有 3 个 PTY 用例失败
  （`openpty: PermissionDenied`）—— 即 `AGENTS.md` §1 记的环境权限，不是回归。
- **akasha-core** 完成（`d4dcbc1`）：6 个文件 → `session/{model,registry,event}.rs`、
  `config/model.rs`、`tunnel/model.rs`；三个壳 → 各自域的 `ipc.rs`；域根重新导出，
  `crate::session::*` / `crate::config::*` / `crate::tunnel::*` 路径不变。`just check` 绿。
- **akasha-bw** 完成（`deecc9c`）：10 个文件 → `bw/`，壳 → `bw/ipc.rs`；
  两个集成测试迁进 `src-tauri/tests/`；依赖（sha2 / zeroize / zip / ureq）并入 app manifest。
  `just check` 绿；`fake_cli` 8/8、`session_protection` 通过。
- **过程中修掉的两处**：
  1. 集成测试里的 `crate::` 一律要改成 `akasha_lib::`（测试是独立的 crate）；
  2. 壳文件被"给纯逻辑加 `crate::bw::` 前缀"的那一遍扫到，`crate::config` 一度变成
     `crate::bw::config` —— 批量替换必须把壳排除在纯逻辑那一遍之外。
- **下一步**：`ssh` → `serial` → `store` → `pty`（按依赖序：先翻依赖方，成员才一直可编译）。

### 2026-09-19 第 2 轮

- **akasha-ssh** 完成（`a2aa8b9`）：19 个源文件 → `ssh/`，两个壳 → `ssh/ipc.rs` 与
  `ssh/ipc/sftp.rs`；依赖（russh / russh-sftp / rand）并入 app manifest；13 个集成测试
  迁进 `src-tauri/tests/`，共用脚手架移到 `tests/ssh_support/`。
  `just check` 与 `just clippy`（`-D warnings`）绿；SSH 集成测试 **51/51** 通过。
- **akasha-serial** 完成（`c76e31b`）：4 个源文件 → `serial/`，壳 → `serial/ipc.rs`；
  `libudev` 上升为 package feature（`default = ["libudev"]`）；两个集成测试迁进 `tests/`。
  `just serial-check` 两次运行都通过。
- **搬迁暴露并修掉的四处**（都不在"搬文件"本身的预期里）：
  1. `known_hosts` 用例按 `CARGO_MANIFEST_DIR/../../target` 定位临时目录 —— 成员在
     `crates/` 下时那正好是 `src-tauri/target`，搬进 `tests/` 后指到了工作区**之外**，
     沙箱直接拒绝（`PermissionDenied`）。改为 `target/…`。
  2. tauri / specta 宏要求命令注册指向**定义命令的模块**：`crate::ssh::open_ssh_session`
     必须写成 `crate::ssh::ipc::open_ssh_session` —— 模块根的重导出满足不了宏生成的伴生项。
  3. `pub use ipc::*;` 会让 `ipc` 里的 `pub mod sftp` 与纯逻辑的私有 `mod sftp;` 相撞
     （`hidden_glob_reexports` 警告，在 `-D warnings` 下就是错误）—— 改成显式再导出。
  4. `lib.rs` 里对已移出该模块的 `pub mod sftp;` 的声明，以及未加 `crate::` 限定的 `sftp::` 引用。
- **环境读数（不是回归）**：`just libudev-check` 在本机（macOS 宿主）红在"依赖图里没有
  libudev" —— 该判据的前提是 Linux 宿主（`libudev` 只声明在 linux 的 cfg 下）；
  `cargo nextest run -p akasha --lib` 的 3 个 PTY 用例仍是 `openpty: PermissionDenied`。
- **下一步**：`store` → `pty`（剩下的两个成员），然后是三条例规则与探针、`Cargo.toml` 注释删除、
  `AGENTS.md` §0/§3.1/§6/§11 与 docs 同步、`just ready` 全绿。

### 2026-09-19 第 3 轮 —— 六个成员全部成为模块

- **akasha-store** 完成（`2f697b1`）：6 个源文件与 `pools/` 进 `store/`（`pools/mod.rs` 改为
  `pools.rs`），两个壳成为 `store/ipc/{vault,pools}.rs`；14 个集成测试与 `tests/store_common/`
  迁进 `tests/`。**护栏与代码同批落地**：`no-tauri-in-core-crates` →
  `no-tauri-in-pure-modules`（`files` 覆盖全部 `src-tauri/src/**`，靠 `ignores` 列出
  允许碰 Tauri 的 app 侧文件 —— 新增模块默认被检查），`no-unsafe-outside-store` 的 `ignores`
  改到 `src-tauri/src/store.rs` / `store/**` 与两个契约测试，`no-ui-vocab-in-types` 的
  `files` 收敛到 `src-tauri/src/**`。真实路径探针（正例命中、诱饵不命中）已做并删除探针。
  读数：`just check` / `clippy` / `lint`（6 条规则）绿；store 集成测试 **114/114**。
- **akasha-pty** 完成（`d851ad1`）：7 个文件进 `pty/`，`pty.rs` 因与父模块同名
  （`clippy::module_inception`）改名 `local.rs`；`benches/batching.rs` 进
  `src-tauri/benches/`，`rustix` 的 `cfg(unix)` 依赖与 `[[bench]]` 并入 app manifest；
  `[workspace]` 去掉 `members` glob（成员已不存在，空 glob 是硬错误）。
- **`Cargo.toml` 注释删除**（`b035b0f`）：只留可执行配置（107 行 → 105 行的净结果）。
- **环境读数（不是回归）**：`just test` 里 4 个 `pty::local` 用例失败于
  `openpty: PermissionDenied` —— `AGENTS.md` §1 记的沙箱限制；`libudev-check` 需要 Linux 宿主。
  这两条使 `just ready` 在本沙箱**不可能全绿**，判据改为逐条核对可执行的各项。
- **下一步（收尾）**：docs 同步（`AGENTS.md` §0/§3.1/§6/§11 单独提交、`docs/just.md` §2/§9、
  `docs/scope.md`、`docs/STATUS.md`、`ROADMAP.md` 阶段 1、`docs/adr/README.md`）、
  ADR-0008 转「已定案」、本 plan 归档、逐条执行 `just ready` 的各项。

### 2026-09-19 收尾（已完成）

- 文档同步：`AGENTS.md`（§0 / §1 / §3.1 / §3.4 / §6 / §7 / §11，单独提交 `13871dc`）、
  `README.md`、`docs/just.md`、`docs/scope.md`、`ROADMAP.md`（阶段 1 标题与条目）、
  `docs/adr/README.md`；ADR-0008 转「已定案」。
- `just ready` 的逐条读数：`fmt-check` / `lint`（clippy + ast-grep scan + ast-grep test）/
  `docs-check` / `gen-types-check` 绿；`test` 在本沙箱有 4 个 `pty::local` 用例失败于
  `openpty: PermissionDenied`（AGENTS.md §1 的环境限制），`deny-offline` 需要联网取 advisory 库。
  两者都不是本次改动引入的。
- 归档：本文件整份移入 `docs/plans/archive/`，索引保留一行。
