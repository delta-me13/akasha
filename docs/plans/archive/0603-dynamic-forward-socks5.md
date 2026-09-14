# Plan 0603: 动态转发 `-D`（SOCKS5）

- **关联**：ROADMAP 阶段 6 ·「动态转发 `-D`（本地 SOCKS5 服务端）」
- **前置**：plan 0601（隧道实体与状态机）· plan 0602（本地监听 + 每条入站连接一条通道）· plan 0505（原语）
- **状态**：已完成（2026-09-13）

## 目标

本机监听端口 → 一个 **SOCKS5 服务端**（RFC 1928）→ 每个客户端在握手里说的目标，
各经一条 `direct-tcpip` 通道送出去。

它与 `-L` 的差别只有一处：**目标从哪来**。`-L` 的目标写在规则里（一条规则一个目标），
`-D` 的目标由客户端逐条说（一条规则服务任意目标）。监听、每条入站连接一条通道、
停止即回收这一整套形状沿用 plan 0602，本 plan 不重写。

⚠️ 这个 SOCKS5 服务端**自行实现**（P1：不依赖系统组件），且它对**浏览器与 `curl` 是唯一
入口** —— 协议只要错一个字节，客户端报的就是一句无从下手的失败。

## 非目标

- SOCKS5 的扩展特性：**只做无认证的 CONNECT**。UDP associate、认证协商、`BIND` 命令
  都在命令边界上明确拒绝（回对应的 REP），而不是装作没看见
- HTTP 代理 / 其他代理形态
- **非回环绑定**（见「先定死的两件事」第 2 条）
- 远程转发（plan 0604）、重连循环（plan 0605）、关闭语义的完整形态（plan 0606）
- 规则池的增删改界面：规则仍由测试直接写库（同 plan 0601）

## 先定死的两件事

1. **REP 要分类，不能一律"通用失败"。** 通道开不出来时，客户端能收到的**只有**那一个
   `REP` 字节 —— 它就是这条功能的错误消息。把"对端不允许这条转发"（0x02）与
   "对端连不上目标"（0x05）压成同一个 0x01，用户就分不清"服务端配置不允许转发"与
   "目标服务没起来"。为此 `SshError::Forward` 多带一档 **类别**（上游给的是结构化的
   `ChannelOpenFailure`，不必从字符串里猜）。

2. **SOCKS5 只允许绑回环地址。** 这一侧**无认证**（RFC 1928 的 `0x00` 是唯一接受的方法），
   绑到 `0.0.0.0` 等于把"经这台跳板机访问远端网络"的能力交给同网段的所有人。
   本版本没有取得明确同意的那一轮询问（D16 的往返只覆盖凭据），所以**拒绝**非回环绑定
   （`127.0.0.1` / `[::1]` / `localhost` 之外的都拒），错误里说清该填什么。
   检查放在**绑定之前**：不合规的监听根本建不出来，后来的调用方也无从绕过。

## 步骤（每步都能独立验证）

1. **`akasha-ssh` 新增 `socks5` 模块**（纯协议，无 Tauri）
   - 协商：只接受无认证；客户端没有 `0x00` → 回 `05 FF` 并关闭连接
   - CONNECT 请求：`ATYP` 三种地址形态（IPv4 / 域名 / IPv6）→ 交出 `ForwardTarget`；
     域名**原样**交给 `direct_tcpip`（由对端解析，同 `-L`）
   - 拒绝：`CMD != CONNECT` → `REP 0x07`；不认的 `ATYP` → `REP 0x08`
   - 回成功 `REP` 在**通道开出来之后**（RFC 1928 的顺序）；`BND.ADDR` 只能是占位
     `0.0.0.0:0` —— 通道确认里没有对端的绑定地址，编一个出来就是撒谎
   - 验证：`just test`，单测走 `tokio::io::duplex`（协商 / 三种 ATYP / 三种拒绝 /
     只写了一半的报文也会在有限时间内结束）
2. **`relay` 从"一个固定目标"变成"入站连接怎么处理"**（`Ingress::Fixed` / `Ingress::Socks5`）
   - `LocalListener::bind(host, port, ingress)`：**绑定地址的合规范围由它决定** ——
     `Socks5` 的非回环地址在这里就被拒（`SshError::NotLoopback`）
   - `serve(runtime, connection)`：不再收 target（目标在哪一侧说，由 `Ingress` 表达）
   - 每条入站连接：`Socks5` 先握手拿目标，再开通道；**失败只影响这一条连接**
   - `SshError::Forward` 多一档 `class: ForwardFailure`（`Prohibited` / `ConnectFailed` /
     `UnknownChannelType` / `ResourceShortage` / `Other`），由 `relay` 翻成 `REP`
   - 验证：`just test`（plan 0602 的 relay 用例不许破）
3. **crate 层用例：接上真服务端**（`akasha-ssh/tests/socks5_forward.rs`）
   - 进程内起 SSH 服务端 + 回声服务端；自写 SOCKS5 客户端经监听端口往返；
     断言服务端的 `direct_tcpip` 请求、中继字节数、停止后端口释放
   - 反例：中继表里没有的名字 → 服务端拒绝通道 → 客户端收到 `REP 0x02`（不是 0x00，也不是 0x01）
   - 验证：`just test`（`socks5_forward` 目标）
4. **app 侧：`tunnel.rs` 支持 `dynamic`**
   - `Rule::local_forward()` → `Rule::ingress()`：`local` 出 `Ingress::Fixed`，
     `dynamic` 出 `Ingress::Socks5`，`remote` 仍明确拒绝
   - `TunnelError` 新增 `NotLoopback`；`Unsupported` 的文案改成"本版本支持 local 与 dynamic"
   - `bind_local` / `connect_and_attach` 跟着去掉 target 参数
   - 验证：`just test`（`Sessions` 既有用例不许破）
5. **前端：两档新文案**（功能验证壳层）
   - `TunnelFailed` 增加 `notLoopback`，并改 `unsupported` 那句
   - 验证：`pnpm build`（`TunnelError` 的穷尽 `switch` 少一档即编译不过）
6. **E2E `tunnel_dynamic_forward`**（真实 app + `curl`）
   - 见「验收命令」。新增 `tests/*.rs` 必须登记进 `src-tauri/justfile` 的 `E2E_TARGETS`
7. **文档同步**：本 plan 置「已完成」并 `git mv` 进 `archive/`（索引与 ROADMAP 指针同步）；
   ADR-0003 D9 的三处消费者进度更新 + §14 记一行；`docs/STATUS.md` 覆盖写

## 验收命令（可直接粘贴执行，并写出预期输出）

```bash
# 1. crate 层：socks5 协议单测 + 接上真服务端的一条
just test          # 预期：退出码 0；akasha-ssh 新增 socks5 与 socks5_forward 用例全绿

# 2. 门禁（格式 / lint / 全量测试 / 依赖 / 生成物已提交 / 文档）
just ready         # 预期：6/6 全部通过，退出码 0（TunnelError 变了，生成物必须已提交）

# 3. 真实 app 上的验收（自包含：没有 app 就自己起一套）
just test-e2e      # 预期：退出码 0；清单含新增的 tunnel_dynamic_forward

# 4. 前端类型（不在 `just ready` 内）
pnpm build         # 预期：退出码 0
```

真实路径的对照（Victauri E2E 逐条做同一件事；`curl` 是**第三方** SOCKS5 客户端）：

| 断言 | 手段 | 预期 |
|---|---|---|
| **判据：配置 SOCKS5 代理后能访问远端网络** | `curl --socks5-hostname 127.0.0.1:<端口> http://<只有对端认识的名字>:<端口>/` | 退出码 0，且响应体是远端服务写的那串字节 |
| 目标名**没有**在本机解析 | 用例自行解析一次并断言**失败**（RFC 2606 保留域） | 解析失败 |
| 走的是 `direct-tcpip` | 服务端的 `direct_tcpip` 请求表 | 每条入站连接各 1 条，`host` 是客户端在握手里说的那个 |
| REP 分类真的到了客户端 | 连一个中继表里没有的名字 | `REP = 0x02`（不是 `0x01`） |
| 协议边界被明确拒绝 | `CMD = BIND` / 未知 `ATYP` | `REP = 0x07` / `0x08`，且**不**产生 `direct_tcpip` 请求 |
| 监听地址可见 | `app_state { probe: "tunnels" }` | 该条带 `bind = "127.0.0.1:<绑定端口>"` |
| **非回环绑定被拒** | 库里那条规则改成 `0.0.0.0` 后点"打开" | 界面上说清是哪条地址被拒；probe 里**没有**它 |
| 停止后端口释放 | 点"停止" → 再连该端口 | 连接被拒 |
| 连接真的断了 | 服务端的 `connections_closed` | `≥ 1` |

## 回滚

- 代码：新增集中在 `akasha-ssh/src/socks5.rs` 与两个 `tests/` 目标；对既有路径的改动是四处 ——
  `LocalListener::bind` 的签名（多一个 `Ingress`）、`SshError::Forward` 多一个字段、
  `Rule::local_forward` → `ingress`、前端两档文案。回退即恢复这四处。
- 数据：**不涉及格式变更**（`forwards` 表的 `direction = 'dynamic'` 与"目标为空"的
  `CHECK` 早已存在，没有新表、没有迁移）。
- 命名：`TunnelError` 新增变体进入 `src/ipc/bindings.ts`，与状态名同一条纪律
  （ADR-0003 §10 第 4 条）—— 改名要连同前端与生成物一起改。

## 实施记录（边做边追加）

- **2026-09-13 展开**：骨架 → 进行中（补齐两件先定死的事、步骤与可粘贴的验收命令）。
- **2026-09-13 落地**：`akasha-ssh` 新增 `socks5` 模块（无认证的 `CONNECT`：问候 → 请求 →
  目标的三种 `ATYP`；`BIND` 回 `0x07`、不认的 `ATYP` 回 `0x08`、没有 `0x00` 方法回 `05 FF`、
  版本不对什么都不回；成功 `REP` 在通道开出来之后才回，`BND.ADDR` 是占位 `0.0.0.0:0`）。
  `relay` 的"一个固定目标"变成 `Ingress`（`Fixed` / `Socks5`），`LocalListener::bind` 因此
  多收一个 `Ingress` —— 绑定地址的合规范围由它决定，`Socks5` 的非回环地址**在绑定之前**就被拒。
  `SshError::Forward` 多带一档 `class`（`ForwardFailure`，取自上游结构化的
  `ChannelOpenFailure`），新增 `SshError::NotLoopback`；`TunnelError` 新增 `NotLoopback`，
  `Unsupported` 的文案改成"只支持 local 与 dynamic"；前端补一档文案。
- **门禁实测**：`just ready` **6/6**；`just test` **315 passed**（akasha **70** + akasha-core 27 +
  akasha-pty 39 + **akasha-ssh 52** + akasha-store 127）；`just test-e2e` **退出码 0**
  （**24 个用例 / 21 个目标**，新增 `tunnel_dynamic_forward` **0.85 s**）；`pnpm build` 退出码 0
  （859.47 kB / gzip 236.60 kB）；`Cargo.lock` **零增量**（本 plan 未新增依赖）。
- **真实 app 上的判据**（`tunnel_dynamic_forward` E2E，测试进程内一台 SSH 服务端 +
  一个 HTTP 服务端）：界面打开那条 `dynamic` 规则 → 答完主机密钥与口令 → probe 报
  `bind = 127.0.0.1:<端口>` → **`curl --socks5-hostname 127.0.0.1:<端口> http://<只有对端认识的
  名字>:<端口>/probe`**（**第三方** SOCKS5 客户端）退出码 0 且响应体就是远端服务写的那一串 →
  对端记到恰好 1 条 `direct-tcpip`（`host` = curl 在握手里说的 `.invalid` 名字，本机解析失败由
  用例自己断言）、中继字节数 `> 0` → 再 curl 一次变 2 条 → 中继表里没有的名字回 `REP 0x02`
  （分类真的到了客户端，而不是通用的 `0x01`）→ 绑 `0.0.0.0` 的那条：界面显示
  「SOCKS5 监听不能绑到 0.0.0.0：这一侧无认证，只允许绑回环地址（127.0.0.1 / [::1] / localhost）」
  且 **probe 里没有它** → 点停止 → 端口不再接受连接、`sessions` 的 `live`/`registered` 相等（1/1）、
  服务端看到 1 条连接断开。
- **两处刻意未做**：`REP 0x05`（对端连不上目标）在真实 sshd 上才会出现 —— 测试服务端是
  "接受通道之后才去连"，因此库里那一档只有单元测试覆盖（见 `docs/STATUS.md` 的「待验证」）；
  SOCKS5 的认证协商 / `BIND` / UDP associate 一律明确拒绝，不做实现。
