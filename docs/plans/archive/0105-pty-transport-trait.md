# Plan 0105: `crates/akasha-pty` —— 通用 `Transport` trait

- **关联**：ROADMAP 阶段 1 ·「`crates/akasha-pty`：通用 `Transport` trait + `portable-pty` 实现」
- **前置**：plan 0103（core 已立，命名与 ast-grep 规则已生效）
- **状态**：已完成（2026-09-11）
- **影响面**：新增 `crates/akasha-pty/`；`src-tauri/Cargo.toml`（依赖）；`.ast-grep/rules/`（按需补 `no-std-command-bypass`）

## 目标

落成**通用**的 `Transport` trait（字节载体），而不是 PTY 专属 trait，并提供 `portable-pty` 实现。

形态按 `scope.md` §2：

```
Transport: write(bytes) / output_stream() / resize(尽力) / shutdown() / exited()
```

- 能力差异用 **capability flag** 表达（serial 无窗口尺寸/信号/退出码；SSH 无本地进程语义），
  不要"多几个方法都得实现一遍"。
- 零 Tauri 依赖、可 mock、可脱离 app 单测。

## 非目标

- **不**做输出合批（plan 0201）—— 本步只把字节通道打通
- **不**接 IPC、**不**碰前端（阶段 2）
- **不**实现 SSH / serial 后端（阶段 5 / 8）
- **不**定义 `Session` 模型（plan 0103 已定，这里只被它拥有）

## 前置检查

```bash
cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name'   # 期望含 akasha-core
sed -n '/## 2\. /,/## 3\. /p' docs/scope.md | head -40                   # 复读后端与 Transport 约定
```

## 步骤

1. `cargo new --lib --vcs none crates/akasha-pty`。
2. 定义 `Transport` trait 与 **capability flag**（如 `resizable` / `has_exit_status` / `has_signals`）。
3. `portable-pty` 实现：spawn / write / resize / shutdown / exited。
   - shell 启动参数**集中管理**，不散落（`AGENTS.md` §3.3）
   - `shutdown` 必须做到**显式 kill + wait 回收子进程**，不能靠 drop
4. 提供**假实现**（内存管道），供单测与其他 crate 用 —— 这是"可 mock"的落地方式。
5. 单测：
   - 假实现覆盖 spawn / write / shutdown 的契约
   - 真实实现能在本机 spawn `/bin/sh` 并回显（平台相关，标 `#[cfg(unix)]`）
   - `shutdown` 后 `exited()` 返回，且**没有僵尸进程**（`wait` 被调用过）
6. 命名守 `scope.md` §1.2：不得出现 `Tab` / `Pane` / `Window` / `View`；**不叫** `PtySession`。
7. 若出现直接 `std::process::Command` 绕过本 crate 的写法，落地 `no-std-command-bypass` 规则
   （规则与代码同 PR，`AGENTS.md` §6）。

## 验收命令

```bash
cargo nextest run -p akasha-pty          # 期望全绿
cargo tree -p akasha-pty | grep -c tauri # 期望 0
cargo tree -p akasha-pty --depth 0       # 期望无 src-tauri 反向依赖
ast-grep scan                            # 期望全绿（含新增规则，若有）
```

> **反向依赖**是架构违规（`AGENTS.md` §3.1）：`crates/*` 不得依赖 `src-tauri`，也不得 `use tauri::`。

## 回滚

删除 `crates/akasha-pty`、从 members 移除、回退相关规则；无数据影响。

## 实施记录

### 验收命令的实际输出（2026-09-11）

1. `cargo nextest run -p akasha-pty` → `14 tests run: 14 passed, 0 skipped`
   （真实 PTY 3 + 假实现 9 + shell 启动参数 2）
2. `cargo tree -p akasha-pty | grep -c tauri` → `0`
3. `cargo tree -p akasha-pty --depth 0` → 只有 `akasha-pty v0.1.0`（无 `src-tauri` 反向依赖）
4. `ast-grep scan` → 退出码 0（本 crate 未触发任何规则）
5. `just ready` → `✅ 全绿（5/5）`

### 真实 PTY 的用例证明的是"执行了"，不是"回显了"

终端会把敲进去的字**原样回显**，所以"发一条带 `AKASHA_PROBE` 的命令、再看输出里有没有它"
并不能证明任何结论 —— 回显里就有。探针因此写成 `printf 'AKASHA%s\n' _PROBE`：
回显里只有 `AKASHA%s` 与 `_PROBE` 两截，**只有 shell 真的执行了**才会拼出完整的那串。

读取按**截止时间 + 读端驱动**，不用固定 `sleep`（`AGENTS.md` §0 第 5 条）。
`shutdown` 之后断言三件事：结局非空（说明 `wait` 回收过子进程）、重复 `shutdown` 幂等、
再 `write` 返回 `Closed`。

### 刻意与初稿不同的两处

- **能力位里没有 `signals`**。初稿的例子里有它，但 trait 现在**没有任何方法能发信号** ——
  没有对应方法的能力位并不生效。等有了 `signal()` 再加（`resize` / `exited` 的默认实现
  就是现成模板）。
- **不实现 `Drop`**。`AGENTS.md` §3.3 说 drop 不能代替 kill + wait；在 `Drop` 里隐式 kill
  会让"调用方忘了 `shutdown`"变成静默成功 —— 而那是要**暴露**的事，不是要掩盖的事。
  代价写进了类型文档：不调 `shutdown()` 就会留下子进程。

### 走查中发现并修掉的一个实际问题

`#[cfg(unix)] mod tests` 少了 `test` ⇒ **测试代码被编进正式产物**（只在 unix 上），
并因无引用而报 `dead_code` 警告。已改为 `#[cfg(all(test, unix))]`。
这类"缺一个 cfg 就把测试带进产物"的问题，编译能过、测试也能过，只有看警告才发现。

### 两件**不做**的事（连同理由）

- **不把 `akasha-pty` 加进 `src-tauri` 的依赖**：现在没有任何代码用它，加了就是一条无引用的依赖。
  阶段 2 的 plan 0201 / 0202 接上时再加 —— 那时也应同时删掉 `src-tauri` 里 `portable-pty` 的
  直依赖（PTY 的实现归 `akasha-pty`，壳里只留 IPC 编组）。
- **不落地 `no-std-command-bypass` 规则**：本 crate 代码里没有任何 `std::process::Command`
  （`portable-pty` 内部用它，那是依赖内部的事，不是我们的绕过）。规则要与它守护的代码同 PR
  （`AGENTS.md` §6）—— 现在没有可守的代码，落地它只会变成噪音。

