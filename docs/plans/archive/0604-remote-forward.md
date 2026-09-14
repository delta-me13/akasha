# Plan 0604: 远程转发 `-R`

- **关联**：ROADMAP 阶段 6 ·「远程转发 `-R`（`tcpip-forward` + `forwarded-tcpip`，**另一套机制**）」
- **前置**：plan 0601（隧道实体与状态机）· plan 0603（`-L` / `-D` 共用的一套形状）
- **状态**：已完成（2026-09-14）

## 目标

向服务端发 `tcpip-forward` 全局请求，由**服务端**在它那一侧监听一个端口；
每条连到那个端口的连接，由服务端开一条 `forwarded-tcpip` 通道交给我们，
我们把它接到**本机**的服务上。

方向与 `-L` / `-D` 相反：那两条是"本机监听、对端去连"（`direct-tcpip`，D9 的原语），
这一条是"服务端监听、服务端发起通道"（ADR-0003 **D10**）。**原语不复用**，
共用的只有隧道实体的那登记 / 状态 / 停止那一套。

## 非目标

- **不复用 `direct-tcpip` 原语**：方向相反，复用即为方向错误（D10 写死了这一点）
- 连接级的复用：一条 `-R` 规则仍是一条连接（D5 / D6）
- 重连循环（plan 0605）、关闭语义的完整形态（plan 0606）
- 远端绑定地址的**合规判断**：由服务端的 `GatewayPorts` 一类设置决定，我们只请求
- 规则池的增删改界面：规则仍由测试直接写库（同 plan 0601）

## 先定死的三件事

1. **先连本机服务，再接受通道。** 本机服务不可达时，我们这一侧唯一能对外说的话是
   那次**通道拒绝**（`ChannelOpenFailure::ConnectFailed`）。反过来"先接受、连不上再关"
   会让对端的客户端先看到连接建立、随后立刻被关 —— 与 OpenSSH 的
   `client_request_forwarded_tcpip` 不是同一种行为，而用户看到的是"时通时不通"。
   ⚠️ 这个连接**必须在任务里做**：`Handler` 的回调是在连接的消息循环上被 `await` 的，
   在回调里连一个不可达的地址会让整条连接无响应（连保活都停），
   而那正是"隧道看起来还活着"的形态。

2. **路由按端口认，不按地址字符串认。** 服务端回报的 `connected_address` 是"它认为在听的
   地址"，与服务端自己的配置有关（请求 `localhost` 可能回报 `127.0.0.1`）。按端口查表，
   查不到的一律拒绝（drop `reply` = `AdministrativelyProhibited`）—— 这正是 D10 那句
   "规则不活跃时拒绝"的落点。

3. **远端绑定地址原样交给服务端。** 与 plan 0603 对 SOCKS5 的回环限制**不同**：
   那一条是我们自己在这台机器上开一个无认证的入口，所以必须限制；这一条的端口开在
   **服务端**，能不能开在非回环地址上是**它的**策略（`sshd` 的 `GatewayPorts` 默认只允许回环）。
   规则里写什么就请求什么，不替对端做决定，也不把它的拒绝说成我们自己的检查。

## 步骤（每步都能独立验证）

1. **`akasha-ssh` 新增 `remote` 模块**
   - `SshConnection::tcpip_forward(address, port)`：`port = 0` 时用服务端回报的端口（D10）；
     被拒报 `SshError::RemoteListen`，与"连不上"分开
   - 入站路由：一张 `端口 → 发送端` 的表挂在连接上；`Handler` 的回调按端口查表
   - 每条入站连接：连本机目标 → 通就 `accept` + `copy_bidirectional`，不通就
     `reply.reject(ConnectFailed)`；本机目标**在本机解析**（与 `-L` 的"由对端解析"相对）
   - 停止：撤销登记 → 回收在途连接 → `cancel_tcpip_forward` → 断开那条连接
   - 验证：`just test`
2. **`SshError` 新增一档**：远端监听没拿到（`RemoteListen { address, reason }`）
   - 与 `Listen`（本机端口）分开：用户要动的地方在**服务端**（换端口 / 找管理员）
   - 验证：`just test`
3. **crate 用例：`akasha-ssh/tests/remote_forward.rs`**（进程内服务端）
   - 测试服务端补上 `tcpip_forward` / `cancel_tcpip_forward`：真的绑定端口，
     每条入站连接开一条 `forwarded-tcpip`，并按通道是否被接受记账
   - 断言：连服务端那个端口 → 本机服务收到连接且字节往返 → 服务端报的 `connected_port`
     就是它实际监听的端口 → 端口 0 时我们拿到服务端挑的那个 → 本机目标不可达时服务端
     看到通道**被拒**（不是"接受了又断"）→ 停止后远端端口释放且收到 cancel
   - 验证：`just test`（`remote_forward` 目标）
4. **app 侧：`tunnel.rs` 支持 `remote`**
   - `Rule::ingress()` → `Rule::prepare()`：`Prepared::Local`（`-L` / `-D`，先绑本机端口）
     / `Prepared::Remote`（`-R`，连上之后才能请求）
   - `Tunnel.forward` → `ActiveForward`（`Local` / `Remote`）；probe 的 `bind` 两档来源不同
     （本机监听地址 / **服务端**的监听地址）
   - 远端监听没拿到时落到 `失败` **并发出事件**（托盘与界面据此看得见，`scope.md` §5.2）
   - `TunnelError` 新增 `RemoteBind`；**删除 `Unsupported`**（三个方向都支持了 ——
     留着一个永远出不来的错误档就是在文档里留一句假话）
   - 验证：`just test`（`Sessions` 既有用例不许破）
5. **前端：两处文案改动**（功能验证壳层）
   - `TunnelFailed` 去掉 `unsupported`、加 `remoteBind`
   - 验证：`pnpm build`（`TunnelError` 的穷尽 `switch` 对不上即编译不过）
6. **E2E `tunnel_remote_forward`**（真实 app）
   - 见「验收命令」。新增 `tests/*.rs` 必须登记进 `src-tauri/justfile` 的 `E2E_TARGETS`
7. **文档同步**：本 plan 置「已完成」并 `git mv` 进 `archive/`（索引与 ROADMAP 指针同步）；
   ADR-0003 D10 补「实现状态」+ §14 记一行；`docs/STATUS.md` 覆盖写

## 验收命令（可直接粘贴执行，并写出预期输出）

```bash
# 1. crate 层：入站路由与接上真服务端的那几条
just test          # 预期：退出码 0；akasha-ssh 新增 remote 单测与 remote_forward 用例全绿

# 2. 门禁（格式 / lint / 全量测试 / 依赖 / 生成物已提交 / 文档）
just ready         # 预期：6/6 全部通过，退出码 0（TunnelError 变了，生成物必须已提交）

# 3. 真实 app 上的验收（自包含：没有 app 就自己起一套）
just test-e2e      # 预期：退出码 0；清单含新增的 tunnel_remote_forward

# 4. 前端类型（不在 `just ready` 内）
pnpm build         # 预期：退出码 0
```

真实路径的对照（Victauri E2E 逐条做同一件事）：

| 断言 | 手段 | 预期 |
|---|---|---|
| **判据：远端监听端口可回连到本机服务** | 连服务端（测试进程内）那个端口，写一串字节 | 本机服务收到连接且拿到的就是那串字节，回程也成立 |
| 服务端真的在听 | 服务端记下的 `tcpip_forward` 请求 + 那个端口的状态 | 请求里的地址/端口与规则一致；端口可连 |
| 通道是服务端发起的 | 服务端的 `forwarded_tcpip` 计数与 `connected_port` | `≥ 1`，且端口就是它报给我们的那个 |
| 本机目标不可达被**拒** | 规则的目标指向一个没人听的端口 | 服务端看到 `ConnectFailed`（不是"接受了又断"） |
| 远端端口拿不到 → `失败` 且看得见 | 规则要求的端口**在服务端那一侧被占着** | probe 里 `state = failed`、事件里有 `failed`、面板上说明是哪个端口没拿到；那条**仍登记着**（可重试） |
| 停止即撤销 | 点"停止" → 再连那个远端端口 | 端口不再接受连接；服务端收到 `cancel_tcpip_forward` |
| 监听地址可见 | `app_state { probe: "tunnels" }` | 该条带 `bind = "<规则里的绑定地址>:<端口>"` |
| 连接真的断了 | 服务端的 `connections_closed` | `≥ 1` |

## 回滚

- 代码：新增集中在 `akasha-ssh/src/remote.rs` 与两个 `tests/` 目标；对既有路径的改动是四处 ——
  `handshake` 的返回值（多带回一份入站路由）、`SshConnection` 多一个字段与两个方法、
  `Handler` 多一个回调、`Rule::ingress` → `prepare`。回退即恢复这四处。
- 数据：**不涉及格式变更**（`forwards` 表的 `direction = 'remote'` 早已存在，
  且 `CHECK` 保证它一定有目标 —— 那个目标正是本机服务；没有新表、没有迁移）。
- 命名：`TunnelError` 的变体进入 `src/ipc/bindings.ts`，与状态名同一条纪律
  （ADR-0003 §10 第 4 条）—— 改名要连同前端与生成物一起改。

## 实施记录（边做边追加）

- **2026-09-14 展开**：骨架 → 进行中（补齐三件先定死的事、步骤与可粘贴的验收命令）。
- **2026-09-14 落地**：`akasha-ssh` 新增 `remote` 模块 —— `SshConnection::remote_listen` /
  `cancel_remote_listen`，`RemoteForward::open`（登记 → 起步任务），入站路由 `Inbound`
  （**一个格子**：`open` 拿走了连接的所有权，唯一性由所有权保证）与 `Route`（drop 即撤销；
  端口对不上时不清别人的登记）；`Handler` 补上 `server_channel_open_forwarded_tcpip`
  （**只做同步派发**），`handshake` 的返回值由 `Handle<Handler>` 变成 `Authenticated`。
  新增 `SshError::RemoteListen` 与 `TunnelError::RemoteBind`，**删除 `TunnelError::Unsupported`**；
  app 侧 `Rule::ingress` → `Rule::prepare`（`Prepared::Local` / `Prepared::Remote`），
  `Tunnel.forward` → `ActiveForward`（`Local` / `Remote`）。测试服务端补上
  `tcpip_forward` / `cancel_tcpip_forward` 与 `forwarded-tcpip` 的发起，并记
  `forward_requests` / `forward_cancellations` / `forwarded_tcpip_accepted` /
  `forwarded_tcpip_rejected`（拒绝的原因也要记：`ConnectFailed` 与
  `AdministrativelyProhibited` 是两件事，而"先接受再关"的实现**什么都不留**）。
- **一处只有真实路径才暴露的错**（记入 `docs/STATUS.md` 问题 #134）：RFC 4254 §7.1 规定
  服务端**只在请求 0 端口时**才在回复里带端口，而上游把"回复里没有这个字段"表示成 `0`。
  最初直接采用返回值 —— 表现是 probe 报 `127.0.0.1:0`，且停止时拿 0 去 `cancel-tcpip-forward`
  （服务端按 `(地址, 端口)` 查，**那个监听根本撤不掉**）。修法是按"请求的是什么"解返回值；
  crate 用例补一条"请求具体端口"的断言（原先那条只用 0 端口）。
- **门禁实测**：`just ready` **6/6**；`just test` **322 passed**（akasha **71** + akasha-core 27 +
  akasha-pty 39 + **akasha-ssh 58** + akasha-store 127）；`just test-e2e` **退出码 0**
  （**25 个用例 / 18 个目标**，新增 `tunnel_remote_forward` **7.01 s**）；`pnpm build` 退出码 0
  （859.44 kB / gzip 236.54 kB）；`Cargo.lock` **零增量**（本 plan 未新增依赖）。
- **真实 app 上的判据**（`tunnel_remote_forward` E2E，测试进程内一台 SSH 服务端 + 一个 HTTP
  服务端）：界面打开那条 `remote` 规则 → 答完主机密钥与口令 → probe 报
  `bind = "127.0.0.1:<规则端口>"` → **`curl http://127.0.0.1:<那个端口>/probe`**（现成客户端）
  退出码 0 且响应体就是**本机** HTTP 服务写的那一串 → 服务端记到的 `tcpip_forward` 请求
  地址/端口与规则一致且被认下、`forwarded_tcpip_accepted` **1 → 2**、中继字节数 `> 0`、
  本机服务的请求计数 `≥ 1` → 目标指向没人听的端口那一条：服务端看到 `ConnectFailed` 而
  `accepted` **没有增加** → 点停止 → 端口不再接受连接、服务端收到 `cancel-tcpip-forward`
  （用的是它回报的那个端口）、`sessions` 的 `live`/`registered` 相等（1/1）、服务端看到
  3 条连接断开；第三条规则（绑定端口在服务端那一侧被占着）落到 `failed`：事件里有它、面板上
  写着「远端监听 … 没拿到：…」，而它**仍在册**（可重试）。
- **两处刻意未做**：远端绑定地址**不做**任何合规检查（那个端口开在服务端，合规与否是它的策略
  —— 与 `-D` 的回环限制不是同一条口径）；`port = 0` 只有 crate 层覆盖（库里 `bind_port` 的
  `CHECK` 不接受 0，app 这条路径产生不出这个请求）。
