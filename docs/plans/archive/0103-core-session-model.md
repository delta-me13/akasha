# Plan 0103: `crates/akasha-core` —— `Session` 模型骨架

- **关联**：ROADMAP 阶段 1 ·「`crates/akasha-core` 骨架：`Session` 模型」
- **前置**：plan 0101（根 workspace）
- **状态**：未开始
- **影响面**：新增 `crates/akasha-core/`；根 `Cargo.toml` 的 members；`.ast-grep/rules/`（两条规则随代码落地）

## 目标

把 `Session` 模型立起来：`SessionId` / `SessionKind` / `SessionRegistry` / `SessionEvent`。
**零 Tauri 依赖**，可脱离 app 单测。

它必须**先于任何后端**：否则第一个后端就会把"终端 = 应用"的假设写进架构，之后拆它很贵
（`scope.md` §5.1）。

## 非目标

- **不**实现任何后端（本地 PTY 在 plan 0105，SSH 在阶段 5）
- **不**做持久化（阶段 4）
- **不**引入 async runtime —— 本步只立数据结构与路由语义
- **不**定义 `Transport` trait（那是 plan 0105 的产物，避免两处各写一半）

## 前置检查

```bash
cargo metadata --no-deps --format-version 1 >/dev/null && echo "workspace OK"
ls .ast-grep/rules/          # 现有规则（no-println 应已在）
```

## 步骤

1. `cargo new --lib --vcs none crates/akasha-core`；把 `crates/*` 写回根 members
   （若 plan 0101 因空 glob 问题临时去掉了它）。
2. 定义四个类型：
   - `SessionId`：**新类型**，不要裸 `u64` 到处传
   - `SessionKind`：`Terminal` / `Sftp` / `Tunnel` / `Vault`（`scope.md` §5.1 的表）
   - `SessionRegistry`：增删查 + 关闭语义（**关闭一个不影响另一个**）
   - `SessionEvent`：**按 `SessionId` 路由**，不是全局广播后由前端过滤
3. **命名守 `scope.md` §1.2**：后端类型名不得出现 `Tab` / `Pane` / `Window` / `View`。
4. 两条 ast-grep 规则**与代码同 PR 落地**（`AGENTS.md` §6：规则先红、代码补上后转绿）：
   - `no-tauri-in-core-crates` —— `crates/**` 里出现 `use tauri::`
   - `no-ui-vocab-in-types` —— 类型名里出现 UI 词汇
5. 单测覆盖：ID 分配唯一；关闭一个 `Session` 不影响另一个；关闭后不再向它投递事件；
   同时注册 / 关闭多个不串号。

## 验收命令

```bash
# 1. 单测全绿
cargo nextest run -p akasha-core

# 2. 核心 crate 不依赖 Tauri（0 = 干净）
cargo tree -p akasha-core | grep -c tauri
cargo tree -p akasha-core --depth 0          # 期望只有 akasha-core

# 3. 结构护栏生效
ast-grep scan
```

**规则必须用负例验证**（否则无法区分"规则在工作"与"规则写错了、什么都没匹配"）：

```bash
cp crates/akasha-core/src/lib.rs /tmp/lib.rs.bak
printf '\nuse tauri::AppHandle;\n' >> crates/akasha-core/src/lib.rs
ast-grep scan                                 # 期望报告 no-tauri-in-core-crates
cp /tmp/lib.rs.bak crates/akasha-core/src/lib.rs   # ⚠️ 用 cp 还原，别用 git checkout（坑 #12）
```

## 回滚

删除 `crates/akasha-core`、从 members 移除、回退两条 ast-grep 规则；无数据影响。

## 实施记录

（边做边追加，记录 `cargo tree` 与负例检查的**实际输出**。）
