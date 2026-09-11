# Plan 0105: `crates/akasha-pty` —— 通用 `Transport` trait

- **关联**：ROADMAP 阶段 1 ·「`crates/akasha-pty`：通用 `Transport` trait + `portable-pty` 实现」
- **前置**：plan 0103（core 已立，命名与 ast-grep 规则已生效）
- **状态**：未开始
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
   - `shutdown` 必须做到**显式 kill + wait 收尸**，不能靠 drop
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

（边做边追加；记录假实现与真实实现各自的**实际输出**。）
