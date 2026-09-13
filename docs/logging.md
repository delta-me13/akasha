# 日志（tracing）

> 规则本体在 [`../AGENTS.md`](../AGENTS.md) §3.4；本文件是展开：形态、字段词汇、级别，
> 以及**为什么**这么定。日志改动的自查表就在下面。

**一句话**：日志是**给人排障、给机器检索的数据**，不是文档。一行一个事件，变量进字段。

## 1. 消息（message）

| 要 | 不要 |
|---|---|
| 英文、小写、**常量**：`session retired` | 中文长句、把变量拼进消息 |
| 事件名（`<主语> <动词>`）：`watchdog registration failed` | 解释为什么、写后果（"否则…"） |
| 一种事件一个 grep 模式 | 括号里塞说明、反引号、命令、示例路径、plan/ADR 号 |
| 变体进字段：`trigger = "exit"` | 为每个变体写一条新消息 |
| 中文写在**注释**里 | 全角标点 `：。、（）`、结尾句号 |

**为什么消息必须是常量**：常量才能被 grep、被计数、被告警；一旦把变量拼进去，
"这个事件出现过几次"就再也数不出来了。

### 反面例子（都在这个仓库里真实存在过）

| 改前 | 改后 |
|---|---|
| `看门狗已启动：app 被 SIGKILL（\`tauri dev\` 重载 / \`kill -9\`）时由它收掉会话` `watchdog=Some(524494)` | `watchdog started` `pid=524494` |
| `会话自己结束：已收掉并从登记簿摘牌` `retired.status=Some(Code(0))` | `session retired` `handle=8 session=8 exit_code=0` |
| `退出：会话已全部显式回收（kill + wait）` `shut_down=2` | `sessions reclaimed` `reclaimed=2 trigger=exit` |
| `看门狗：登记会话失败（这条会话少一道兜底）` | `watchdog registration failed` `leader=1422 err=…` |
| `panic：已尽力回收会话` `failures=[(1, "…")]` | `sessions not reclaimed` `reclaimed=1 failed=1 trigger=panic` |

三个反复出现的根因，都是同一个：**把"给人读的说明"写进了日志**。

1. **括号里解释**（`（这条会话少一道兜底）`）—— 属于注释的职责。
2. **在日志里引用文档**（`见 plan 0205`）—— 日志会被归档、被丢弃，链接会烂，读者也点不开。
3. **`Debug` 倒进字段**（`Some(Code(0))` / `[(1, "…")]`）—— 那是 Rust 的类型包装，
   不是数据。要打 `Option` 就展开成值或分两支，要打集合就打 `len()`（细节各自成行）。

## 2. 字段（key = value）

| 字段 | 含义 | 取值 |
|---|---|---|
| `handle` | IPC 会话句柄 | `u32` |
| `session` | 注册表里的会话 id | `u64` |
| `leader` | 会话首领进程 pid（看门狗认会话的键） | `u32` |
| `pid` | 看门狗自己的 pid | `u32` |
| `exit_code` | 正常退出的退出码 | `u32` |
| `signal` | 终止会话的信号名（与 `exit_code` **二选一**；上游给的显示名，见 §6） | str |
| `reclaimed` / `failed` | 退出路径上已回收 / 未回收的会话数 | usize |
| `trigger` | 哪条退出路径 | `exit` / `panic` / `tray` |
| `reason` | 自造的变体或失败原因 | 稳定字面量 |
| `event` | 发不出去的事件名 | str |
| `err` | 错误本体，用 `%err`（Display） | 错误类型 |

应用级（配置 / 托盘 / 单实例）：

| 字段 | 含义 | 取值 |
|---|---|---|
| `window` | 窗口 label —— **唯一**表示"哪个窗口"的字段 | str |
| `close_behavior` | 配置里写的行为 | `tray` / `exit` |
| `close_action` | **判据实际决定**的动作（与 `close_behavior` 不同 = 降级了，例如托盘建不起来） | `hide` / `exit` |
| `path` | 本地路径：配置文件（`config loaded` / `config not found`）或**便携数据目录**（`portable data dir not writable`）。取不到就**不写**这个字段 | str |
| `activations` | 本进程被第二个实例叫起来的次数 | u64 |
| `step` | 多步动作里失败的那一步 | `unminimize` / `show` / `focus` |

- 命名：`snake_case`、名词 + 单位（`exit_code` / `leader`），不加 `ctx_` / `my_` 前缀。
- **一个概念一个字段名**：同一个意思不许换名字（窗口一律 `window`，不得再写 `label` / `win`）——
  换了名，一次 grep 就变成两次，而漏掉的那次不会有任何提示。
- **拿不到就不写这个字段**（`watchdog started` 不带 `pid`），**不填 0 / `unknown` 顶替** ——
  字段值不撒谎，比字段齐更重要。
- 二选一的字段（`exit_code` / `signal`）用分支写；`ExitStatus` 的 `Display` 是**给界面看的中文**、
  `Debug` 会带上包装，两个都不适合直接当字段 —— 见 `src-tauri/src/session.rs` 的 `log_ended`。

## 3. 级别

| 级别 | 报什么 | 例 |
|---|---|---|
| `info` | 正常路径上的状态变化，一次事件一行 | `session retired` |
| `warn` | **降级但照常可用**：少一道兜底、一次清理没做成 | `watchdog registration failed` |
| `error` | **收尾没做成**，可能留下残留 | `sessions not reclaimed` |

不得用 `info` 报失败（会被掩盖），也不得用 `error` 报预期内的降级（会让告警变噪音）。

## 4. 为什么用英文

- 日志是机器接口：`target` 与依赖库（tauri / wry / rmcp）本来就是 ASCII，
  grep、管道、告警规则都不必跟编码打交道。
- 更实际的一条：**英文这个约束本身会排除说明文体**。中文长句读起来像文档，
  于是就会越写越长 —— 这不是巧合，是上述根因的成因。

## 5. 由什么强制

- `just lint` 里的 ast-grep 规则 [`no-non-ascii-log-message`](../.ast-grep/rules/no-non-ascii-log-message.yml)：
  拦"消息不是 ASCII"这一半（`tracing::*!` 里的中文字符串）。`tests/` 豁免。
- 规则拦不住的（啰嗦、后果叙述、`Debug` 泄漏）**只能靠本文件 + review** ——
  改日志时对着 §1 §2 两张表自查。

## 6. 边界（不在本条内）

- **字段值可能非 ASCII** —— 那是上游给的**数据**，不是我们的话术（消息仍必须 ASCII）：
  - `%err`：错误类型自己的文本（`TransportError` 等）。
  - `signal`：portable-pty 0.9 的名字取自 `libc::strsignal`，**随 locale 变**
    （zh_CN 下 `SIGKILL` 会写成 `已杀死`），而且**信号编号在 portable-pty 内部就被丢掉**了 ——
    从它的公开 API 拿不回 `SIGKILL`。修法与代价见 `STATUS.md` 问题 #59（要动依赖）。
- 日志**格式**（`[日期][时间][target][级别]`、**无毫秒**）是 `tauri-plugin-log` 默认 formatter 的行为，
  本仓库没有自定义 —— 跨进程（app + 看门狗）的先后顺序不得依赖时间戳判断。
- 事件 payload 里的文案（如 `SessionEnded.status`）是**给界面看**的，不是日志，可以中文。
