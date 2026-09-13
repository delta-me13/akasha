# Plan 0601: 隧道实体 + 状态机

- **关联**：ROADMAP 阶段 6 ·「隧道实体（独立于终端 `Session`）+ 状态机」
- **前置**：plan 0505（`direct-tcpip` 原语）· plan 0103（`Session` 模型）
- **状态**：已完成（2026-09-13）

## 目标

隧道是**独立实体**，不是终端 `Session` 的属性：一条转发规则一个 `Session`，
两者各自持有连接、互不影响（`scope.md` §2.2）。

状态机五态：`连接中 / 已连接 / 重连中(第 n 次) / 失败 / 已停止`，
**状态变化发事件**（UI 未实现不影响后端先行发送 —— 不得等到实现 UI 时才补）。

本 plan 的产物是"**一条已连接的隧道**"：它持有一条真实的、已认证的 SSH 连接
（ADR-0003 D5 / D6 的"一实体一连接"），**但还不转发任何字节** ——
三种转发机制分别在 plan 0602 / 0603 / 0604，它们都在这条连接上开通道。

## 非目标

- 三种转发各自的机制（plan 0602 / 0603 / 0604）—— 本 plan 只立实体与状态机
- 重连策略本身（plan 0605）：`重连中(n)` 在**转移表里可达**，但本 plan 不产生它 ——
  真实路径上没有任何循环去驱动那条边；实现它的是 0605
- 关闭语义的完整形态（plan 0606）：本 plan 只做到"命令级停止 + 退出时随 `shutdown_all` 回收"
- 托盘菜单的点击聚焦（plan 0301 预留了 id 命名法）：本 plan 只让**名称与状态**可见
- 池的增删改界面：转发规则**只能**由测试直接写库（同 plan 0505 构造跳板链的先例）

## 前置检查（开工前已核对）

- `ADR-0003` §10 第 4 条把**隧道状态机的状态名与事件名**列为不可逆点（它们会进入
  `src/ipc/bindings.ts` 与前端）—— 因此命名先于代码定死（见下一节）。
- D12 定状态集合、事件与手动重试；D13 定"哪些失败重连"的判据（本 plan 只用它来
  **分类**失败，重试本身留给 0605）。
- D6 定"隧道 Session 与终端 Session **共用同一个 `SessionId` 空间与注册表**"——
  因此不新建第二个注册表，也不新建第二把锁（两把锁必然产生锁序问题）。
- D9 的 `SshConnection`（已认证、没有通道）就是隧道要持有的连接；
  `SshTransport` 是"连接 + shell 通道"，隧道不用它。

## 命名（不可逆点，先定死）

| 项 | 取值 | 落点 |
|---|---|---|
| 状态（IPC 名） | `connecting` · `connected` · `reconnecting` · `failed` · `stopped` | `akasha-core::TunnelState`，`serde` camelCase |
| 状态的载荷 | `reconnecting` 携带 `attempt`（第几次，从 1 起）；其余不携带 | 同上 |
| 事件 | `tunnel_state`，载荷 `{ handle, state, attempt }` | `src/ipc/bindings.ts` / 前端 |
| 命令 | `tunnel_open(forwardId)` · `tunnel_retry(handle)` · `tunnel_stop(handle)` | 同上 |
| 只读命令 | `vault_forwards()`（界面据此列出规则） | 同上 |
| probe | `tunnels` → `[{ handle, ruleId, name, state, attempt }]` | `app_state` |

`handle` 复用 `SessionHandle`：隧道就是一个 `Session`，不新造一套 id 空间。

## 步骤（每步都能独立验证）

1. **`akasha-core::TunnelState` + 转移表**（纯逻辑，零 Tauri）
   - 五态枚举 + `advance(from, to) -> Result<TunnelState, TunnelTransitionError>`：
     合法边白名单、非法边报错、同态转移报错。
   - 合法边：`连接中 → {已连接, 失败, 已停止, 重连中(1)}` ·
     `已连接 → {重连中(n), 失败, 已停止}` · `重连中(n) → {连接中, 失败, 已停止}` ·
     `失败 → {连接中, 已停止}` · `已停止 → {连接中}`。
   - `重连中` 的 `attempt` **必须 ≥ 1**（0 应当用 `连接中` 表示）。
   - 验证：`cargo test -p akasha-core tunnel`（在 `src-tauri/` 下），
     逐条断言上表每一格 + 非法边（含"同态"与"已连接 → 连接中"）。
2. **`akasha-core::SessionEvent` 增加 `TunnelStateChanged`**
   - 验证：注册表那条既有用例扩展为"按 `SessionId` 路由"，两个隧道的状态互不串号。
3. **`akasha-ssh`：链建立只留一份实现**
   - `SshConnection` 增加同步门面 `connect_via(&Handle, hops, options)`（与
     `SshTransport::connect_via` 对称），两者共用同一个"建链"内部函数。
   - `SshConnection::disconnect` 由 `pub(crate)` 改为 `pub`（不暴露 `Handle`）。
   - 验证：`cargo test -p akasha-ssh`（既有跳板用例必须全绿，证明重构未改行为）。
4. **app：隧道实体与三条命令**（`src-tauri/src/tunnel.rs`）
   - 实体：`{ id: SessionId, rule_id, name, host_id, state, attempts, connection }`。
   - 登记进 `Sessions` 的**同一张注册表与同一把锁**（`Inner` 增加 `tunnels`；
     `len()` 与 `registered()` 仍必须相等 —— probe `sessions` 的既有断言不许破）。
   - `tunnel_open` 是 **async 命令**：读池规则 → 建实体（`连接中` + 发事件）→
     建立已认证连接 → `已连接`（发事件）；失败 → `失败`（发事件）并把原因回给前端。
   - `tunnel_retry` 清空 `attempts` 后按 `失败 / 已停止 → 连接中` 转移并重连；
     `tunnel_stop` 走 `→ 已停止`，断开连接并注销注册。
   - `Sessions::shutdown_all` 一并收隧道（退出路径不许留下活连接与登记）。
   - 验证：`just test`（app 的单元用例）+ Victauri `invoke_command` 真路径。
5. **probe `tunnels` 与事件接线**
   - probe 读的是**同一份实体表**（不另立一张账）。
   - 事件在**锁外**发（`Sessions` 的既有纪律：握着会话表的锁调订阅者会自己等自己）。
6. **托盘隧道子菜单显示 `名称 · 状态`**（`scope.md` §5.2 的"失败必须可见"）
   - 列表从 `Sessions` 读，**与 probe 同源**。
7. **前端：隧道面板**（功能验证壳层，`AGENTS.md` §4.0）
   - 列出转发规则 → "打开" → 显示状态与尝试次数 → "重试" / "停止"。
   - 每次收到 `tunnel_state` 事件追加到 `window.__akashaTunnels`（**测试接口**），
     供 E2E 断言"事件真的到了前端"。
8. **E2E `tunnel_state`**（真实 app + 测试进程内的 SSH 服务端）
   - 见「验收命令」。
9. **文档同步**：本 plan 置「已完成」并 `git mv` 进 `archive/`（索引与 ROADMAP 指针同步）；
   `ADR-0003` §12 删除已定条目 + §14 记一行；`docs/STATUS.md` 覆盖写。

## 验收命令（可直接粘贴执行，并写出预期输出）

```bash
# 1. crate 层：状态机与既有 SSH 行为（workspace root 是 src-tauri/，一律走 just）
just test          # 预期：退出码 0；新增 akasha-core 的隧道用例全绿

# 2. 门禁（格式 / lint / 全量测试 / 依赖 / 生成物已提交 / 文档）
just ready         # 预期：6/6 全部通过，退出码 0

# 3. 真实 app 上的验收（自包含：没有 app 就自己起一套）
just test-e2e      # 预期：退出码 0；清单含新增的 tunnel_state
```

真实路径的对照（Victauri，`just test-e2e` 里的用例逐条做同一件事）：

| 断言 | 手段 | 预期 |
|---|---|---|
| 五态模型完整 | `just test` 的转移表用例 | 五态齐全，非法边全被拒 |
| `连接中 → 已连接` | `invoke_command tunnel_open` → 答完主机密钥与口令 | probe `tunnels` 的 `state = connected` |
| 状态变化发事件 | 前端把收到的 `tunnel_state` 记进 `window.__akashaTunnels` | 事件序列含 `connecting` 与 `connected`，`handle` 与 probe 一致 |
| `→ 已停止` | `invoke_command tunnel_stop` | probe 里那条消失；`sessions` 的 `live` 与 `registered` 相等 |
| `连接中 → 失败` | 规则指向不可达端口 | 命令回错误、probe `state = failed`、**事件仍发出** |
| 手动重试（D12） | `invoke_command tunnel_retry` | 状态回到 `connecting`（随后再次失败）；尝试次数清零 |
| 不含重连 | 上述全程 | **没有任何** `reconnecting` 事件（0605 之前它不由真实路径产生） |

## 回滚

- 代码：本 plan 只新增（`akasha-core/tunnel.rs`、`src-tauri/src/tunnel.rs`、
  前端隧道面板与 E2E 目标），对既有路径的改动集中在两处 ——
  `akasha-ssh` 的"建链抽成一份"（行为由既有用例守住）与 `Sessions::Inner` 增加一张表
  （`len()` / `registered()` 的对等关系由既有 probe 断言守住）。回退即删除新增内容。
- 数据：**不涉及格式变更**（没有新表、没有迁移）。
- 命名：状态名与命令名一旦进过 `bindings.ts` 就当作契约对待；若要改，
  必须连同前端与生成物一起改（ADR-0003 §10 第 4 条）。

## 实施记录（边做边追加）

- **2026-09-13 展开**：骨架 → 进行中（补齐步骤、命名与可粘贴的验收命令）。
- **2026-09-13 落地**：`akasha-core::TunnelState` 五态 + 转移表（12 条用例，含正例表、
  反例表、同态、`attempt ≥ 1`、重试面、"五态从 `连接中` 都走得到"）；`SessionEvent`
  增加 `TunnelStateChanged`（注册表侧多一条按 `SessionId` 路由的用例）；`akasha-ssh`
  的建链抽成 `hops_chain` 一份实现 + `SshConnection::connect_via` 同步门面 +
  `disconnect` 改为 `pub` 并**从最内层往外**断链；app 侧新增 `src-tauri/src/tunnel.rs`
  （实体、三条命令、`tunnel_state` 事件、`tunnels` probe）与 `vault_forwards` 只读命令；
  托盘隧道子菜单改为「名称 · 状态」；前端新增隧道面板（`src/tunnels/TunnelPanel.tsx`
  + `src/ipc/tunnels.ts`）与 `window.__akashaTunnels` 事件探针。
- **门禁实测**：`just test` **292 passed**（akasha 68 + akasha-core 27 + akasha-pty 39 +
  akasha-ssh 31 + akasha-store 127）；`just test-e2e` **退出码 0**（22 个用例，
  新增 `tunnel_state` 0.90 s）；`just ready` **6/6**；`pnpm build` 退出码 0
  （859.11 kB / gzip 236.37 kB）；`Cargo.lock` **零增量**（本 plan 未新增依赖）。
- **真实 app 上的判据**（`tunnel_state` E2E，测试进程内一台 SSH 服务端）：
  界面点开面板 → 列出池里的两条规则 → 打开那条能连通的 → 主机密钥与口令各答一轮 →
  probe `tunnels` 的 `state = connected`、界面 `data-tunnel-state="connected"`、
  事件序列 `["connecting", "connected"]` 且 `handle` 与 probe 一致 →
  点"停止" → probe 里那条消失、`sessions` 的 `live`/`registered` 相等（1/1）、
  **服务端看到 1 条连接断开**、事件里有 `stopped`；连不上的那条：
  `tunnel_open` 回 `{handle, failure:{kind:"failed",…}}` → probe `state = failed` →
  `tunnel_retry` 再走一遍 `connecting → failed`（**全程没有 `reconnecting`**）。
- **计划之外的三条发现**（已写进 `docs/STATUS.md` 的已知问题）：
  1. **登记本身就是进入「连接中」，那不是一次转移**。最初在命令里对它再走一次状态机，
     被 `connecting → connecting` 这条非法边正确拦下 —— 修正为"登记处发那条事件"。
  2. **`target_host` / `target_port` 在本 plan 不参与连接**：隧道只连规则所属主机
     （`forwards.host_id`）—— 因此"连不上"的构造点在**主机**那一层，
     而不是目标端口（plan 0602 的 `-L` 才让目标参与）。
  3. **测试服务端原先只有通道级的断开计数**（`sessions_closed`），而隧道**没有通道**
     （D4）—— 新增连接级的 `connections_closed`（handler 的 `Drop` 计数），
     "停下来之后连接真的断了"才有外部证据。它同时是 plan 0606 要用的观察点。
- **一处刻意未做**：`重连中(n)` 在转移表里可达，真实路径不产生它（驱动它的循环是
  plan 0605）—— 托盘与 probe 都能显示这个状态，但没有代码会走到它，文档与判据都
  不假装它已被验证。
