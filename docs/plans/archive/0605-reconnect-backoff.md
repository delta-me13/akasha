# Plan 0605: 断线重连（3 次 + 指数退避）

- **关联**：ROADMAP 阶段 6 ·「断线重连：**3 次 + 指数退避**，然后标记失败」· ADR-0003 **D12 / D13**
- **前置**：plan 0601（状态机）· plan 0602 / 0603 / 0604（三个方向的转发都走得通）
- **状态**：已完成（2026-09-15）

## 目标

已连接的隧道**掉线之后**自动再连：默认 **3 次**，退避 **1s → 2s → 4s**（D13），
耗尽之后落到 `失败`（**可见**：托盘菜单与界面上的那条是 · 失败），并保留手动重试。

`重连中` 这一态从 plan 0601 起就在状态机里（连同"次数必须 ≥ 1"那条判据），
但**没有任何代码会产生它** —— 本 plan 就是那个驱动它的循环。

## 非目标

- **首次连接失败不自动重试**：那一次失败是**同步**报给用户的（命令返回里带原因，
  界面上就是那条失败文案），下一步动作在用户手上。D13 的表说的也是"传输层**断开**"
  —— 断开的前提是曾经连上过。这条口径让首次连接保持"一次一次来"的行为不变。
- 参数**不进配置文件**：退避取值落在配置模型（`akasha_core::Config`）里、有默认值，
  但 `config.json` 仍然只暴露 `close_behavior`。给一个嵌套对象定文件格式要连界面一起设计，
  而这三个值的现实用途是"排查时改小"，属配置界面那份 plan。
- 失败原因怎么**呈现**给用户（界面/托盘上的那句"为什么"）：本步只保证状态可见 + 记日志。
- 关闭 `Session` 时中止重连循环的完整形态（plan 0606）—— 本 plan 只**接出**那个中止入口。
- 无限重连、退避加抖动（D13 已否决 / 留作配置项）。

## 先定死的三件事

1. **断线靠"那条连接死了"来认，认的地方在转发任务里。** 上游 0.x 只给了同步的
   `Handle::is_closed()`（没有可 `await` 的关闭信号），所以转发任务按固定间隔看一眼
   （一次布尔读，不是等待），死了就结束自己 —— 而"转发结束"就是重连循环要的那个信号。
   ⚠️ 不这样做的话，掉线之后那条隧道会永远停在 `已连接`：端口还在听、界面还说连着，
   而每一次入站连接都开不出通道。那正是 `scope.md` §2.2 点名的那类失败模式。

2. **重连 = 重新走一遍「准备 + 连接 + 起转发」，不是"接着用"。** 连接是那条转发的命根子
   （它持有 `SshConnection`），连接没了，转发也就没了。对三个方向这意味着不同的动作：
   `-L` / `-D` 重新绑本机端口；`-R` **重新发一次 `tcpip_forward`** —— 远端监听是服务端
   那条连接的资源，连接一断它就被撤销了，不重新请求就是"重连成功了、端口却不在听"。
   ⚠️ 规则里写 `port = 0` 的 `-R` 因此**可能换一个端口**（服务端重新挑），probe 报的是新的那个。

3. **重连循环是每条隧道一条长住任务，它是这条隧道唯一的驱动者。** 停止 / 重试 / 退出
   通过**一个 `oneshot` 停止信号**让它退出（D5 的"关闭 Session 立刻关闭连接"要能到这）；
   循环自己在每一次 `await` 上回应这个信号，所以停止不必等一次握手的 10 秒。

## 步骤（每步都能独立验证）

1. **`akasha-core`：退避策略**（纯逻辑，零 Tauri）
   - `Reconnect { max_attempts, initial, factor }`：默认 `3` / `1s` / `2`；`delay(n)` 给出
     **第 n 次**重试前的等待（`1s → 2s → 4s`），`n` 超出次数返回 `None`
   - `Config` 多一个字段（D13 的"默认值写在配置模型里"）；单测钉住那三个数、序列与边界
   - 验证：`just test`
2. **`akasha-ssh`：一次转发"怎么结束的"**
   - `ForwardEnd { Stopped, ConnectionLost }` + `ForwardEnding`（`oneshot` 的接收端）
   - `SshConnection::is_closed()`；`LocalListener::serve` / `RemoteForward::open` 各返回多一份
     `ForwardEnding`；转发任务按间隔看那条连接的死活，死了就以 `ConnectionLost` 结束
   - ⚠️ 结束信号**最后**发，且本机监听先显式 drop：否则重连的第一次绑定会与那条
     还没被释放的端口撞上（表现为"重连的第一次必然失败、第二次才成"）
   - 验证：`just test`
3. **`akasha-ssh/testing.rs`：测试服务端要能被切断**
   - `Running::cut_connections()`（监听留着，只切已建立的连接 —— 模拟线路中断）
     与 `Running::shutdown()`（连监听一起停 —— 模拟服务端消失）
   - 每台测试服务端的转发监听**随连接一起消失**（真实 `sshd` 就是这样：转发属于那条连接）；
     不这样做，重连时对同一个端口的 `tcpip_forward` 会被自己上一次留下的监听顶掉
   - crate 用例：连接被切断 → 转发以 `ConnectionLost` 结束且端口随之释放
   - 验证：`just test`
4. **app 侧：重连循环**（`tunnel.rs`）
   - `Tunnel` 多一份"看护任务"的停止信号；`drive` 拆成「连一次」与「首次尝试 + 起看护」
   - 循环：等这次转发结束 → **只有实体仍在 `已连接`** 才重连（停止 / 重试已经把它推走了）→
     回收那次转发 → 逐次 `重连中(n)` → 退避 → `连接中` → 再连一次；连上就回到"等结束"，
     D13 说不重试的类别（认证 / 主机密钥 / 配置）**直接** `失败`
   - 耗尽次数 → `失败` + 一条带原因的日志（原因这一步只进日志）
   - 手动重试的合法性检查**提到绑定之前**：不在终态就直接拒，不先绑一个端口再报错
   - 验证：`just test`
5. **前端：无改动**（`TunnelPanel` 从 plan 0601 起就渲染 `重连中（第 n 次）`，事件带 `attempt`）
   - 验证：`pnpm build` 退出码 0（穷尽 `Record` 对不上即编译不过）
6. **E2E `tunnel_reconnect`**（真实 app）：见「验收命令」。新增目标必须登记进
   `src-tauri/justfile` 的 `E2E_TARGETS`
7. **文档同步**：本 plan 置「已完成」并移入 `archive/`（索引与 ROADMAP 指针同步）；
   ADR-0003 D12 / D13 补「实现状态」+ §14 记一行；`docs/STATUS.md` 覆盖写

## 验收命令（可直接粘贴执行，并写出预期输出）

```bash
# 1. crate 层：退避策略 + 转发结束的那两条路
just test          # 预期：退出码 0；akasha-core 与 akasha-ssh 各多出本步的用例

# 2. 门禁（格式 / lint / 全量测试 / 依赖 / 生成物已提交 / 文档）
just ready         # 预期：6/6 全部通过，退出码 0

# 3. 真实 app 上的验收（自包含：没有 app 就自己起一套）
just test-e2e      # 预期：退出码 0；清单含新增的 tunnel_reconnect

# 4. 前端类型（不在 `just ready` 内）
pnpm build         # 预期：退出码 0
```

真实路径的对照（Victauri E2E 逐条做同一件事）：

| 断言 | 手段 | 预期 |
|---|---|---|
| **判据：掉线进入「重连中」** | 切断那条 SSH 连接（测试服务端监听不变） | 事件里按序出现 `reconnecting`（`attempt = 1`），probe 里那条也是 |
| **判据：连得回来** | 等它自己重连 | 事件里随后有 `connected`；**转发端口又能用**（真 `curl` 一趟，字节到本机 HTTP 服务） |
| 重连是"重新连一次" | 服务端的 `direct_tcpip` / `connections_closed` | 请求数增加、旧的连接计数也涨 —— 新连接另起一条 |
| **`-R` 重连要重新请求监听** | 同一条 `remote` 规则也切一次连接 | 服务端的 `tcpip_forward` 请求数**变成 2**，且重连后那个端口又能 `curl` 通 |
| **判据：耗尽次数变「失败」且可见** | 停掉整个测试服务端，再切一次 | 事件序列 `reconnecting(1) → reconnecting(2) → reconnecting(3) → failed`，全程 ≥ 7 s（1+2+4），probe 里 `state = failed` |
| 失败之后仍可手动重试 | `tunnel_retry` 那条 | 状态回到 `connecting`，再落回 `failed`（服务端仍未回来） |
| 停止要能中止循环 | 重连途中点"停止" | 那条从 probe 里消失，且**不再出现** `connected`（循环真的停了） |
| 连接真的断了 | 服务端的 `connections_closed` | `≥ 1` |

## 回滚

- 代码：新增集中在 `akasha-core` 的退避策略、`akasha-ssh` 的 `ForwardEnd` 检测与 app 侧的重连循环；
  对既有路径的改动是四处 —— `serve` / `open` 的返回值（多带回一份结束信号）、
  `SshConnection` 多一个方法、`Tunnel` 多一个停止信号、`TunnelError` 多一个判据方法。
  回退即恢复这四处；没有它，隧道的行为回到 plan 0604 的"掉线后停在 `已连接`"。
- 数据：**不涉及格式变更**（没有新表、没有迁移，`forwards` 表不动）。
- 命名：没有新的 IPC 命令与事件字段（状态名与事件从 plan 0601 起就是这一套）；
  `ForwardEnd` 是 crate 内部类型，不进生成物。

## 实施记录（边做边追加）

- **2026-09-15 展开**：骨架 → 进行中（补齐三件先定死的事、步骤与可粘贴的验收命令）。
- **2026-09-15 落地**：`akasha-core::config::Reconnect`（默认 3 / 1s / 2，`delay(n)` 一处算出
  `1s → 2s → 4s`，`budget()` = 7s；`Config` 多一个字段，`config.json` **不动**）；`akasha-ssh`
  新增 `ending` 模块（`ForwardEnd` / `ForwardEnding`）与 `SshConnection::is_closed()`，
  `serve` / `open` 改为返回**两半**（转发本体 + 结束通知），转发任务按 500 ms 看一眼那条连接的
  死活；app 侧 `tunnel.rs` 的**看护任务**（`start_watch` / `watch` / `reconnect` / `attempt_again`
  / `connect_once`）与 `TunnelRun`（一条 `oneshot` 的停止入口 —— `tunnel_stop` / `tunnel_retry` /
  `shutdown_all` 都从它进去）；`Sessions` 多两个方法（`attach_tunnel_run` / `take_tunnel_run`）。
  测试服务端多两件：`Running::cut_connections()` / `shutdown()` 与**按连接持有**的远端监听
  （连接一断它随之消失 —— 真实的 `sshd` 就是这样）。新增 E2E 目标 `tunnel_reconnect`。
- **一处只有真实路径才暴露的错**（记入 `docs/STATUS.md` 问题 #135）：**托盘从来没被重推过** ——
  `Sessions::set_tunnel_state` 只往注册表发事件，没有调用 `notify_changed`，而托盘菜单是一份
  快照（`tray::refresh` 挂在 `on_change` 上）。于是 `scope.md` §5.2 指定的"失败可见"落点永远停在
  隧道刚登记时那一行。修法在 `set_tunnel_state`（放锁之后通知），并新增 `tray` probe 报出
  **菜单上那几行文字**（与菜单共用 `tunnel_labels`），好让"托盘可见"这条判据**可被断言**。
  ⚠️ 本沙箱里 `ready = false`（建不起托盘），所以用例**显式跳过**那一处断言并打印原因。
- **两处刻意的取舍**：① **首次连接失败不自动重试**（D13 的表说的是"断开"，而断开的前提是曾经连上
  过；那次失败本来就是同步报给用户的）；② **失败原因只进日志**，不进事件（`tunnel_state` 的载荷
  没有原因字段，加它就是改一份从 plan 0601 起就已经发出的契约）—— 所以"耗尽之后界面说得清
  为什么放弃"仍是未做的工作。
- **门禁实测**：`just ready` **6/6**；`just test` **333 passed**（akasha **74** + akasha-core **30** +
  akasha-pty 39 + **akasha-ssh 63** + akasha-store 127，比 plan 0604 多 **11** 条）；
  `just test-e2e` **退出码 0**（**26 个用例 / 19 个目标**，新增 `tunnel_reconnect` **19.19 s**）；
  `pnpm build` 退出码 0（859.44 kB / gzip 236.54 kB，±0 —— 本轮没有前端改动）；
  `Cargo.lock` **零增量**（本 plan 未新增依赖）。
- **真实 app 上的判据**（`tunnel_reconnect` E2E，测试进程内一台 SSH 服务端 + 一个 HTTP 服务端）：
  两条隧道（`-L` 与 `-R`）都连上、两个端口都能 `curl` 通 → 服务端把会话断开 → 事件里各出现
  `reconnecting(1)`、面板上写着「重连中（第 1 次）」→ 两条都自己回到 `connected`、两个端口又能
  `curl` 通 → 服务端的 `tcpip_forward` 请求数 **1 → 2**（第二次的端口仍是规则里那个）→
  重连途中点"停止" → 那条从 probe 里消失且 3 秒内没有新的 `connected` → 服务端**整个消失** →
  事件序列 `reconnecting(1) → reconnecting(2) → reconnecting(3) → failed`，从消失到 `failed`
  实测 **7.55 s**（预算 ≥ 7 s = 1+2+4）→ `tunnel_retry` 再走一遍 `connecting → failed`。
- **一处刻意未做**：`Reconnect` 的三个数**不进 `config.json`**（它仍只认 `close_behavior`）——
  给一个嵌套对象定文件格式要连界面一起设计，而这三个值的现实用途是"排查时改小"。
