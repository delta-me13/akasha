# Plan 0505: `direct-tcpip` 原语

- **关联**：ROADMAP 阶段 5 ·「`direct-tcpip` 原语（本阶段先用于跳板，之后三处复用）」
- **前置**：plan 0502（能连上、能认证）· plan 0504（**已完成** —— 真 app 上能开 SSH 会话了）
- **状态**：已完成（2026-09-13）
- **接口归属（0504 的结论，别两边各定一套）**：`Handle` 仍归 `SshTransport` 自己（它把句柄交给
  那条 `pump` task，收尾的唯一出口在那里），0504 **没有**改这一点。三条路里选**第二条**：
  **跳板那条路自己持有连接** —— `Handle` 因此不出 `akasha-ssh`，也不必给 `SshTransport` 开一个
  "受控借用口"（借用口把"谁在什么时候能碰这个句柄"变成一个**可以问错**的问题）。

## 目标

一次 `direct-tcpip` channel 链式串接，落在 `src-tauri/crates/akasha-ssh`，形状 = **一条
`AsyncRead + AsyncWrite` 的流**（ADR-0003 D9），**供三处复用**：跳板（本阶段）· SSH 本地转发
（阶段 6）· SFTP host↔host 的 B 档（阶段 7）。本 plan 把**跳板**走通 —— 顺手拿到另两处的地基。

## 非目标

- **远程转发 `-R`**：另一套机制（服务端发起 `forwarded-tcpip`，ADR-0003 D10），不在本原语里
- **隧道实体的状态机**（plan 0601）：本 plan 只做原语与"跳板这条连接路径"，不做隧道生命周期
- **跳板链的界面与写入路径**（池的增删改仍无界面）：本 plan 只让库里**已有的** `jump_id` 真能用

## 判据（ROADMAP 原文）

ProxyJump 可连通**只对跳板机可见**的目标。

## 前置检查

```bash
# ① 0504 那条会话真的通了（本 plan 的观察点都挂在它上面）
git log --oneline -1 --grep 'SSH 接进 IPC'     # afd3036
# ② 上游口子都在（0.63.3）：channel_open_direct_tcpip / Channel::into_stream / connect_stream
grep -n "channel_open_direct_tcpip\|pub fn into_stream" \
  ~/.cargo/registry/src/*/russh-0.63.3/src/client/mod.rs ~/.cargo/registry/src/*/russh-0.63.3/src/channels/mod.rs
# ③ 测试服务端**现在没有** direct-tcpip（所以第 4 步必须先补它，否则"跳板"无从观察）
grep -c "channel_open_direct_tcpip" src-tauri/crates/akasha-ssh/src/testing.rs   # 期望 0
```

## 步骤（每步都能独立验证）

1. **原语的形状 = 一条流**（`akasha-ssh/src/forward.rs`，新模块）
   `SshStream` 包住 `russh::ChannelStream<client::Msg>` 并实现 `AsyncRead + AsyncWrite`（手工转发
   `poll_*`）—— **不把 `russh` 的 0.x 类型漏进公开签名**（与 `HostKey` 同一条纪律）。
   `SshConnection` = 一条**已认证、没有通道**的连接：`async fn connect(options)`（直连 TCP）·
   `async fn over(&self, options)`（**在本连接上开 `direct-tcpip` 再握手**）·
   `async fn direct_tcpip(&self, host, port) -> Result<SshStream, SshError>` ← **D9 原语本体**。
   这一层**只有异步形态**：三个消费者都在 runtime 里（app 的命令边界走第 3 步的同步门面）。
   *验证*：对着测试服务端开一条通道，字节双向。
2. **一跳 = 握手 + 认证，底层流由调用方给**（`handshake.rs`）
   把 `establish` 里"配 config → `connect_stream` → 认证 → 把被拒原因换回来"抽成
   `async fn handshake<S>(options, stream) -> Result<Handle<Handler>, SshError>`，约束
   `S: AsyncRead + AsyncWrite + Unpin + Send + 'static`（`connect_stream` 的要求）。
   `establish` 变成"（可选地经跳板）拿到流 → `handshake` → 开 shell 通道"。
   *验证*：直连那条路一个字节都不变 —— plan 0502/0504 的用例全绿即为证。
3. **跳板链：一条连接、一条链**（`transport.rs`）
   `SshTransport::connect_via(runtime, hops: Vec<SshConnect>, target: SshConnect)`，`hops` 从
   **最外层**到最内层（第一台是 app 直接连的）；`connect(runtime, options)` = 空链的那一次。
   - 每一跳是一个 `SshConnection`，它们随 `Established.carriers` **move 进 `pump` task** ——
     "task 结束 = 整条链结束"，收尾的唯一出口不变
   - 收尾顺序 = **最内层先断、再往外**（`while let Some(c) = carriers.pop()`）：反过来会把
     还活着的通道踩掉
   - originator 用**最外层那条 TCP 的本地地址**（我们唯一真知道的），往下每一跳复用；
     拿不到就空串 + 0（RFC 4254 允许）
   - 新错误 `SshError::Forward { target, reason }`：跳板**拒绝 / 连不到**目标（上游
     `ChannelOpenFailure` 的原话带上）。**它与 `Connect` 分开** —— "跳板机够不着那台"与
     "我够不着跳板机"对用户是两件事
   *验证*：`connect_via` 在 tokio 上下文里仍然返回 `BlockingInsideRuntime`（与 `connect` 同一条）。
4. **测试服务端支持 `direct-tcpip`**（`testing.rs`）
   `Observed.direct_tcpip: Vec<ForwardRequest>`（host / port / originator）= 判据的**服务端那一半**；
   `ServerOptions.relay: Vec<(String, u16, SocketAddr)>`（"我可以替你连到哪"的映射）；
   `channel_open_direct_tcpip` 记一条 → 有映射就 `accept` + `copy_bidirectional` 中继、没有就
   **drop `reply`**（= 拒绝，真实服务端的表现）。再加 `Observed.relayed_bytes`：
   "通道真的通了"不能只听客户端那一半说。
   *验证*：库内用例里跳板服务端能拒绝（无映射）也能中继（有映射），两条分支各有一测。
5. **库内验收：两个真服务端 + 一个不可解析的名字**（`akasha-ssh/tests/jump_host.rs`）
   目标地址**只在跳板的映射表里**；客户端拿到的是 `akasha-jump-only.invalid:22`（RFC 2606 保证
   不可解析）。于是"直连"这条路**在构造上就不存在**。
   - 正例：`connect_via(hops=[jump], target)` → 终端发字节 → **目标服务端**收到同一串
   - 负控：不经跳板直连同一个目标名**必须失败**（`connect_timeout` 设 2 s，别让它拖）——
     没有这一条，正例分不清"经了跳板"与"这个名字其实能直连"
   - 跳板那一半：`observed().direct_tcpip` **恰好一条**，host / port 与配置一致
   - 收尾：`shutdown()` 之后**目标**看到通道关闭，**跳板**上的中继也收工
     （`relays_finished`）且搬过字节（`relayed_bytes`）
   *验证*：`cargo nextest run -p akasha-ssh --test jump_host`（`--test` 而不是过滤函数名，坑 #110）。
6. **app 侧：池里的跳板链**（`akasha-store` + `src/ssh.rs` + 前端）
   - `hosts::jump_chain(conn, id) -> Result<Vec<Host>, StoreError>`：**目标在前**（`[target, jump, …]`），
     有界（`MAX_JUMP_DEPTH`）、成环报 `StoreError::JumpChain`。读路径也要防：写路径挡住的是
     **我们的**写入，挡不住有人手工改库
   - `src/ssh.rs`：`plan()` 解出整条链（每一跳各一份 `SshConnect`，各问各的凭据），
     `open_ssh_session` 走 `connect_via`
   - `HostEntry` 加 `jump_id`（库里本来就有这一列），选主机的列表把"经跳板 X"显示出来 ——
     否则这条能力在界面上**没有痕迹**，E2E 也只能从服务端那半边看
   - `SshFailureKind` 加 `Jump`（跳板拒绝转发 / 链成环），前端能给出不一样的提示
   *验证*：`just gen-types` 后 `bindings.ts` 的差异只有新字段与新变体。
7. **E2E：真 app 上经跳板开会话**（`src-tauri/tests/ssh_jump.rs`，新目标）
   进程内起跳板 + 目标两台；库里种两行（目标的 `host` 是不可解析的名字、`jump_id` 指向跳板）；
   从界面选目标那行 → **按类型循环回答提示**（跳板的密钥 / 跳板的口令 / 目标的密钥 / 目标的口令）
   → 连上 → 敲命令 → **目标**服务端收到 → 断言**跳板**服务端记到一条 `direct-tcpip`。
   `ssh_session.rs` 里的驱动助手提成 `tests/support/`（两个目标共用，别抄第二份）。
   *验证*：`just test-e2e` 退出码 0，且 `── E2E: ssh_jump ──` 那段 1 passed。

## 验收命令

```bash
# ① DoD（含 gen-types-check / docs-check）
just ready                     # 期望 6/6

# ② 原语与跳板的库内验收：两个真服务端，正例 + 负控
cd src-tauri && cargo nextest run -p akasha-ssh --test jump_host   # 期望 2 passed
cd src-tauri && cargo nextest run -p akasha-ssh                    # 期望：0502–0504 的用例全绿 + 新的

# ③ 池这一层：链读得出、有界、成环报错（store 的用例）
cd src-tauri && cargo nextest run -p akasha-store --test pools_roundtrip   # 期望 11 passed（+3）

# ④ 真 app 上的判据（`ssh_jump` 必须排在 `ssh_session` 之后 —— 两段都自己造库）
just test-e2e                  # 期望退出码 0；`── E2E: ssh_jump ──` 1 passed

# ⑤ 生成物与前端
just gen-types                 # 期望 bindings.ts 已提交、无未预期差异
pnpm build                     # 期望退出码 0
```

## 回滚

本 plan 只新增（原语 / 测试脚手架 / store 的一个读函数）与**一处**改动既有路径：`establish` 拆成
`handshake` + 尾巴。回滚 = 反向 `git revert` 这个提交即可 —— 池里就算有 `jump_id`，
旧代码也只是**忽略**它（直连），不会读出一个错的地址。

## 与 0601 / 0703 的接口对齐

形状一旦定下，另两处**只能适配它**：0601/0602 拿的是 `SshConnection::direct_tcpip` 那条流
（隧道 Session 自己持有连接，与跳板同一条路）；0703 的 B 档同理。`SshStream` 是
`AsyncRead + AsyncWrite`，所以 `client::connect_stream` 与 `copy_bidirectional` 都直接吃得下 ——
不需要任何一方再发明一条路径。

## 实施记录（边做边追加，记录验收命令的实际输出）

**形状**（第 1–3 步）：`forward.rs` 落 `SshStream`（自己实现 `AsyncRead + AsyncWrite`，不把
`russh::ChannelStream` 漏进公开签名）+ `SshConnection`（已认证、没有通道）+ `direct_tcpip`；
`handshake.rs` 里 `establish` 拆成 `handshake<S: AsyncRead + AsyncWrite + Unpin + Send>` 与
shell 尾巴；`transport.rs` 新增 `connect_via(runtime, hops, target)`（`connect` 就是空链的那次）。
carriers 跟着 `Established` move 进 `pump`，收尾按"最内层先断"。

**库内验收**（第 5 步）：`cargo nextest run -p akasha-ssh --test jump_host` → **4 passed / 0.25 s**：
`jumping_reaches_a_host_only_the_bastion_can_see`（正例，跳板恰好收到 1 条
`direct-tcpip → akasha-jump-only.invalid:22`）、`a_direct_attempt_at_that_name_fails`（**负控**）、
`a_bastion_that_refuses_the_forward_says_so`（`SshError::Forward` 而非 `Connect`）、
`the_sync_facade_refuses_to_be_called_inside_a_runtime`。

**store**（第 6 步）：`hosts::jump_chain`（目标在前、有界、成环报 `StoreError::JumpChain`）；
`cargo nextest run -p akasha-store --test pools_roundtrip` → **11 passed**（+3）。

**E2E**（第 7 步）：`just test-e2e` **退出码 0**（20 个用例 + 第三段 3 条）。`ssh_jump` 实测输出：

```
跳板：127.0.0.1:41727 指纹 SHA256:DlfGQwC98as7…      目标：akasha-e2e-inner.invalid:22（真实 127.0.0.1:38055）
构造前提：akasha-e2e-inner.invalid 在本机解析失败（RFC 2606）—— 直连这条路不存在
主机池：[{"host":"akasha-e2e-inner.invalid","id":2,"jumpId":1,…}, {"host":"127.0.0.1","id":1,"jumpId":null,…}]
界面：目标那一行显示「经跳板 e2e-ssh-jump」
提示问答（按发生顺序）：["hostKey:DlfG…", "credential:e2e@127.0.0.1:41727", "hostKey:mG/Q…",
                        "credential:e2e@akasha-e2e-inner.invalid:22"]
跳板服务端：收到 1 条 direct-tcpip → akasha-e2e-inner.invalid:22
终端回声：via-the-bastion（目标收到了同一串；跳板中继搬过 5240 字节）
关标签页：sessions probe = {"live":1,"registered":1}；目标服务端看到 1 条连接断开
```

**门禁**：`just ready` 6/6；`cargo nextest run --workspace` **243 passed**（+8：跳板 4 + store 3 +
E2E 目标 1）；`pnpm build` 退出码 0。

**落地时踩到的四件事**（已进 `docs/STATUS.md`，括号里是坑号）：

- `Config::nodelay` 在 `connect_stream` 这条路上**是句空话**（上游只在 `client::connect` 里看它）
  —— 0502 那行 `config.nodelay = true` 从来没生效过；现在改成自建 TCP 时显式
  `set_nodelay(true)`（#120）。
- 测试服务端的 `Handler::data()` **对所有通道**都会被调用（上游把数据同时交给通道自己的接收端
  与 handler），于是一条 `direct-tcpip` 通道会被"回声"打回来，客户端读到的是**自己刚写的
  id 行**（`Bad packet size: 1397966893`，那串数字就是 `"SSH-"`）。改成只对 shell 通道回声（#121）。
- `copy_bidirectional` **出错时不交出**已搬的字节数，而收尾那一下报错是常态 —— 计数器因此
  要边搬边记（`Counted` 包装），否则"搬过字节"这条判据在最需要它的时候永远是 0（#122）。
- `rusqlite` 不是 `akasha` 的 dev-dependency（0504 的用例靠类型推导避开它）；`tests/support/`
  要写 `Connection` 就得走 `akasha_store::Connection` 这个再导出（#123）。

**照实记的三条边界**：

1. **没有真正的网络隔离**：无特权环境里换 netns / 加防火墙都要 root，所以"只对跳板机可见"是
   靠**名字**（`.invalid` + 跳板侧映射表）构造的，用例自己解析一次那个名字并断言失败。这与
   "跳板机能看见、我看不见"在行为上等价，但**不是**同一件事。
2. **跳板链今天只能靠写库配出来**：池的增删改界面仍未规划（`jump_id` 只做了**读**这一侧）。
   界面上能看见"经跳板 X"，但改不了它。
3. **`-L` / SFTP B 档还没接上这个原语**：形状已按"一条流"定下（这一步是计划里说的"顺手拿到地基"），
   真正的适配在 plan 0602 / 0703。

