# ADR-0003：SSH 栈与资源模型

- **状态**：**实现中**（Implementing，2026-09-12）
- **日期**：2026-09-12
- **决策者**：cyrene
- **影响范围**：新增 crate `src-tauri/crates/akasha-ssh`；`Transport` 在 SSH 上的映射与其
  capability 取值；`Session` 与连接的**所有权**关系；隧道状态机与事件；`akasha` 侧的
  tokio runtime；阶段 6 / 7 的接口形状
- **关联**：[plan 0501](../plans/archive/0501-adr-0003-ssh-stack.md)（本文的落地）、
  [0502](../plans/archive/0502-ssh-connect-auth.md) / [0503](../plans/archive/0503-known-hosts.md) /
  [0504](../plans/archive/0504-ssh-into-ipc-frontend.md) / [0505](../plans/archive/0505-direct-tcpip-primitive.md) /
  [0506](../plans/archive/0506-ssh-config-subset-import.md)、阶段 6 全部
- **取代**：无

---

## 1. 背景与范围

`docs/scope.md` §2.1 / §2.2 / §5.1 已经定案 SSH 这一层的**产品级**结论。本文把它们转换
成可实现、可核对的架构决策，并钉死那些**改动代价为一整层**的点：连接模型、`Transport`
映射、`direct-tcpip` 原语的形状。

本文只定义形状与约束，不含实现步骤 —— 实现记录在 plan 0502–0506（均已完成并归档）。
按 [`docs/adr/README.md`](./README.md) 的三态规则，本文在实现期间可就地修订，每次改动记入 §14。

| `scope.md` 已定案的条目 | 本文对应的决策 |
|---|---|
| §2.1 纯 Rust（`russh`），不调用系统 `ssh` | D1 / D2 |
| §2 能力差异用 capability flag 表达，不引入新 trait | D3 / D4 |
| §2.2 三条转发路径 + 一个原语服务三处 | D9 |
| §2.2 远程转发是另一套机制 | D10 |
| §2.2 转发是独立 Session | D6 |
| §2.2 连接生命周期 = 拥有它的 Session 的生命周期 | D5 |
| §2.2 不复用连接 + 三条副作用对策 | D5 / D7 / D8 / D11 |
| §2.2 有限次重连、状态机、失败必须可见 | D12 / D13 |
| §5.1 资源挂 Session、事件按 `SessionId` 路由、类型互不依赖 | D5 / D6 / D12 |
| §8 风险 4 `~/.ssh/config` 受限子集 | D14 |

**决策索引**（编号顺序与文档顺序不一致，D15 / D16 是后续修订补入的）：

| 决策 | 内容 | 所在小节 |
|---|---|---|
| D1–D3 | 版本与 feature、runtime 归属、同步门面 | §3 |
| D15 | 连接取值：超时 / 保活 / `nodelay` | §3 |
| D4 | `Transport` 的 capability 映射 | §4 |
| D5 / D6 | 连接所有权、隧道 Session 粒度 | §5 |
| D7 / D8 | 认证顺序、内存凭据缓存 | §6 |
| D16 | 交互式提问的往返形状 | §6 |
| D9 / D10 / D11 | `direct-tcpip` 原语、远程转发、known_hosts 策略 | §7 |
| D12 / D13 | 隧道状态机、重连参数 | §8 |
| D14 | `~/.ssh/config` 受限子集 | §9 |

## 2. 事实依据（调研，2026-09-12）

以下每条均可核对。来源：crates.io API 与从 `static.crates.io` 下载的 `russh-0.63.3` 源码
（引用的行号为解包后的路径）；本机侧为本仓库的 `Cargo.lock` 与 `rustc --version`。

| 事实 | 值 | 出处 |
|---|---|---|
| `russh` 最新稳定版 | **0.63.3**（2026-09-09 发布，共 130 个版本） | crates.io `/api/v1/crates/russh` |
| 许可证 | Apache-2.0（`deny.toml` 的 allow 已含） | 同上 |
| edition / MSRV | edition 2024，`rust-version = "1.89"` | 解包后的 `Cargo.toml` |
| 本机工具链 | rustc **1.98.1** | `rustc --version` |
| crypto 后端 | `ring` 与 `aws-lc-rs` 二选一必开，均不开启则 `compile_error!` | `src/lib.rs:90-94` |
| `ring` 依赖 | `0.17.14` | `Cargo.toml` |
| `aws-lc-rs` 依赖 | `1.16.2` | `Cargo.toml` |
| `ssh-key` 依赖 | **`=0.7.0-rc.11`（预发布，由上游钉死）** | `Cargo.toml` |
| 上游 feature 集 | `default = ["flate2", "aws-lc-rs", "rsa"]` | `Cargo.toml` `[features]` |
| 本仓库 lock 中已有的 `ring` | **0.17.14**（由 `rustls` / `quinn-proto` / `reqwest` 引入） | `src-tauri/Cargo.lock` |
| 本仓库 lock 中已有的辅助 crate | `bytes` 1.12.1 / `tokio-util` 0.7.19 / `zeroize` 1.9.0 / `memsafe` 1.0.2 | 同上 |
| 重连 / 退避设施 | 上游**不提供**（全源码搜索无 `reconnect`） | 全源码 grep |
| 本机 sshd | `/usr/bin/sshd` 存在，可用作真机验收；阶段 5 未使用（见 `docs/STATUS.md`「待验证」） | `which sshd` |

**上游 API 面**（取自 0.63.3 源码）：

| 需要的能力 | API | 位置 |
|---|---|---|
| 建立连接 | `client::connect(Arc<Config>, addrs, handler) -> Handle<H>`；`connect_stream(…)` 可接任意 `AsyncRead + AsyncWrite` | `client/mod.rs:1083` / `:1102` |
| 服务器密钥校验 | `Handler::check_server_key(&PublicKeyOrCertificate)` —— **默认实现拒绝一切**，必须覆写 | `client/mod.rs:2368` |
| 密码 / 公钥 / 键盘交互 | `authenticate_password` / `authenticate_publickey` / `authenticate_publickey_with(user, key, alg, &mut S)` / `authenticate_keyboard_interactive_start` | `client/mod.rs:335` / `:456` / `:493` / `:356` |
| 认证结果 | `AuthResult::{Success, Failure { remaining_methods, partial_success }}`；`partial_success` 即 2FA 的续接点 | `src/auth.rs` |
| agent | `keys::agent::client::AgentClient::{connect_env, connect_pageant, connect_named_pipe}`；且 `impl Signer for AgentClient<R>`（可直接传给 `authenticate_publickey_with`） | `keys/agent/client.rs:75` / `:96` / `:104`；`auth.rs:258` |
| 终端会话 | `channel_open_session()` → `request_pty(want_reply, term, cols, rows, px, px, &[(Pty, u32)])` → `request_shell()`；`window_change`；`data` / `data_bytes`；`ChannelMsg::{Data, ExtendedData, Eof, Close, ExitStatus, ExitSignal, …}` | `client/mod.rs:795`；`channels/mod.rs:205` / `:299` / `:326` |
| 本地转发 / 跳板 / SOCKS5 | `channel_open_direct_tcpip(host, port, originator_addr, originator_port) -> Channel<Msg>`；`Channel::into_stream() -> ChannelStream`（`impl AsyncRead + AsyncWrite`） | `client/mod.rs:838`；`channels/mod.rs:661`；`channels/channel_stream.rs:28` / `:41` |
| 远程转发 | `tcpip_forward(address, port) -> u32`（`port = 0` 时由服务端分配）/ `cancel_tcpip_forward`；入站入口 `Handler::server_channel_open_forwarded_tcpip(…, reply: ChannelOpenHandle)`（`reply.accept()` 确认，drop 即拒绝） | `client/mod.rs:885` / `:911` / `:2477` |
| known_hosts | `keys::known_hosts::{check_known_hosts_path, known_host_keys_path, learn_known_hosts_path}`；密钥变化报 `Error::KeyChanged { line }` | `keys/known_hosts.rs:24` / `:63` / `:137` |
| 保活 | `Config::{keepalive_interval: Option<Duration>, keepalive_max: usize}`，默认 `None` / 3；`Handle::send_keepalive` | `client/mod.rs:2284` 起的 `Config` 与其后的 `Default` |

**上游默认值**（会影响本 ADR 的取值）：`window_size = 2 MiB`、`maximum_packet_size = 32768`、
`channel_buffer_size = 100`、`keepalive_interval = None`、`inactivity_timeout = None`、`nodelay = false`。

## 3. 版本、feature 与运行时（D1–D3、D15）

### D1 —— `russh = "=0.63.3"`，`default-features = false`，features = `["ring", "rsa"]`

- **决定**：精确钉版本；crypto 后端取 `ring`；启用 `rsa`（旧密钥仍常见）；不启用
  `aws-lc-rs` 与 `flate2`；明确不要 `dsa` / `des`。
- **理由**：
  - `ring` 0.17.14 已是 lock 中存在的版本（由 `rustls` / `quinn-proto` / `reqwest` 引入），
    不引入第二个 crypto 后端；`aws-lc-rs` 需要 `aws-lc-sys`（cmake + bindgen，Windows 上还需
    NASM），会提高四平台与将来 Android 的交叉编译成本，而换来的性能与本产品的瓶颈
    （网络 RTT）无关。
  - 上游仍在 0.x、累计 130 个版本，API 变动频繁；`=` 与 `tauri-specta` 采用同一口径（`AGENTS.md` §5）。
  - 不启用 `flate2`：压缩可协商，客户端不提供即退化为不压缩，而压缩本身是已知的侧信道面。
    若遇到仅支持压缩的服务端，再开启该 feature。
- **否决的替代路**：`ssh2` / libssh2（绑定 OpenSSL，`scope.md` §2.1 已否决）、调用系统 `ssh`
  （同上）、使用上游默认 features（会引入 `aws-lc-rs`，与本条的交叉编译理由冲突）。
- **遗留风险**：上游把 `ssh-key` 钉在预发布版 `0.7.0-rc.11`，我们的 lock 因此含一个预发布版本。
  该选择不由我们控制；升级 `russh` 时一并确认上游是否已改用正式版。

### D2 —— `akasha-ssh` 不自建 runtime，只接收 `tokio::runtime::Handle`

- **决定**：库不创建线程池、不创建 runtime；`akasha`（app 侧）在启动时创建**唯一一个**专用
  tokio runtime，并将 `Handle` 注入。单测用 `#[tokio::test]`（或自建 runtime）提供 handle。
- **理由**：库自建 runtime 是公认的反模式（谁 drop、几个 worker、何时停止都会成为库的隐式行为）；
  `Handle` 注入使"进程内 runtime 只在一处配置"成立，也让 `akasha-ssh` 在无 app 时可测。
- **否决的替代路**：① 库内懒建单例 runtime（drop 语义与测试隔离都会变差）；
  ② 使用 tauri 的 `async_runtime`（`crates/*` 不得 `use tauri::*`，且需依赖其未承诺的接口）。
- **代价**：`akasha` 的 `tokio` 从 dev-dependency 升为真依赖
  （features：`rt-multi-thread` / `time` / `sync` / `io-util` / `net`）。
- **实现状态**：已落地（plan 0504）。`src-tauri/src/ssh.rs` 在启动时创建**唯一一个**专用
  runtime（worker 数 = 4 —— 提问会占用一个 worker，见 D16），把 `Handle` 注入 `akasha-ssh`；
  创建失败时降级（记录一条 `error` 日志 + SSH 命令返回明确错误），**不阻塞启动**。

### D3 —— 同步 `Transport` 是门面，异步在其后

- **决定**：`akasha-pty` 的 `Transport` 保持**同步**不变；`akasha-ssh` 把连接作为 task 运行在注入的
  `Handle` 上，两个方向各一条有界 `tokio::sync::mpsc`：
  - **写**：`Transport::write(&[u8])` → 入队 → task `recv().await` → `Channel::data_bytes()`
    （背压由 russh 自身的窗口机制提供）；`resize` 同理映射到 `window_change`。
  - **读**：task 接收 `ChannelMsg::Data` → 入队 → 同步侧用 `blocking_recv()` 实现
    `std::io::Read`（`output_stream()`），交给现有的合批器 `akasha_pty::spawn_batcher`。
- **理由**：会话层已经持有 `Box<dyn Transport>`（`src-tauri/src/session.rs` 的 `Inner`），
  SSH 因此**不需要**改动会话层与前端 —— 这正是 `scope.md` §2 要求"用 capability flag
  而不是新 trait"的兑现。
- **实现状态**：已落地（plan 0502）。队列满时的唯一行为是显式错误
  `TransportError::Busy(&'static str)`。约束背景：`write_session` 是同步命令，而 async 命令
  走 `crate::async_runtime::spawn` —— 调用点是否位于 tokio 上下文取决于命令的写法，
  门面不能假设。因此队列满时不得阻塞调用线程：`blocking_send` 在 tokio 上下文中会 panic，
  在任何上下文中都可能阻塞界面线程；`Unsupported` / `Closed` 的语义也不合适。

### D15 —— 连接取值：超时 10 s、保活 30 s × 3、`nodelay = true`（plan 0502 定案）

- **决定**：`connect_timeout = 10s`；`keepalive_interval = Some(30s)`、`keepalive_max = 3`
  （上游默认为 `None` / 3）；`nodelay = true`（上游默认为 `false`，即 Nagle 开启）。
  默认值集中在 `akasha-ssh` 的 `SshConfig::default()` 一处。
- **理由**：
  - **保活必须取值**：上游默认 `None` 表示永不发送保活，于是半开连接（对端不响应而 TCP 未断开）
    会一直保持，症状是用户误认为连接仍然有效。30 s × 3 ≈ 90 s 检出；更短的间隔会在长连接设备上
    产生可观的空包。
  - **`nodelay = true`**：交互式终端的输入是"一次按键一个小包"，Nagle 会将其合并并等待确认，
    产生可感知的输入延迟，而 SSH 交互正是这一场景。
  - 超时 10 s 兼顾慢速网络与用户感知，与阶段 6 的重连退避（1 s / 2 s / 4 s）无关 ——
    本条只约束**单次**连接尝试的时长。
- **限制**：本 ADR 未构造真实的半开连接与高延迟链路，因此这三个数值是**有依据的默认值**，
  而非实测最优值；阶段 6 的真机重连用例负责回填。
- **实现状态**：已落地（plan 0502），其中 `nodelay` 一项在 plan 0505 修正 —— 上游只在
  `client::connect` 中读取 `Config::nodelay`，而我们两条路都走 `connect_stream`，
  因此改由自建 `TcpStream` 时显式 `set_nodelay(true)`（见 `docs/STATUS.md` 问题 #120）。

## 4. `Transport` 的 SSH 映射（D4）

### D4 —— capability：SSH 终端 `resize + exit_status`；隧道两者皆无；`session_leader() = None`

| 载体 | `resize` | `exit_status` | `session_leader()` |
|---|---|---|---|
| SSH 终端（shell channel） | 有（`window_change`） | 有（远端 `ChannelMsg::{ExitStatus, ExitSignal}`） | `None` |
| SSH 隧道（`direct-tcpip` channel） | 无 | 无 | `None` |
| 本地 PTY（现状） | 有 | 有 | 有（看门狗使用） |

- **决定**：capability 按**载体**取值，不按后端。隧道是纯字节管道，既没有尺寸也没有结局。
- **实现状态**：已落地（plan 0502）。`akasha-pty/src/transport.rs` 中 `Capabilities::exit_status`
  的文档现表述为"能否给出退出结局"，并说明"没有本地进程"是 `session_leader()` 的问题。
  修改前的表述为"能否给出退出结局。serial 与 SSH 没有本地进程语义。"—— **"没有本地进程"与
  "没有结局"是两件事**：SSH 没有本地 pid，但远端 shell 会报告 exit-status / exit-signal。
  `scope.md` §2 中"SSH 没有本地进程语义"一句仍然成立，该句指的是 pid。
- `session_leader() = None` 回应的是 ADR-0005 §6 要求每个新载体回答的问题：SSH 这条路上没有
  本地进程需要代收。app 被 SIGKILL 时 socket 由内核关闭，服务端随之撤销 `-R` 的监听，
  因此看门狗不介入 SSH 这条路径。
  plan 0502 验证的是这一档本身（`session_leader()` 返回 `None` 的断言包含在 capability 用例中）。
  **"真退出（含 `kill -9`）之后 `-R` 的远端监听消失"属于 plan 0604 的验收** —— `-R` 属于阶段 6，
  plan 0502 时它尚不存在；该条原先误挂在本节，修订见 §14。

## 5. 所有权与生命周期（D5–D6）

### D5 —— 连接归 `Session`；一条连接只属于一个 `Session`；不复用

- **决定**：终端 Session 拥有一条 `Transport`（= 一条 SSH 连接 + 一个 shell channel）；
  隧道 Session 拥有"每条规则一条连接"；关闭 Session 会**立即**关闭连接（无宽限期）；
  窗口关闭（默认收托盘）**不影响**连接；应用退出时全部关闭且零残留。
- **理由**：`scope.md` §2.2 已定案。复用会引入"一条断开全部断开"的耦合，而"每条隧道可独立重启"
  正是转发场景需要的。
- **否决的替代路**：连接池 / multiplexing —— 它把"这条连接何时该关闭"变成需要引用计数的问题，
  而引用计数的缺陷表现为"随机关闭其他隧道"。

### D6 —— 隧道是**独立**的 `Session`，一条规则对应一个 `Session`，共用同一 `SessionId` 空间与注册表

- **决定**：端口转发落在 `SessionKind::Tunnel`（`crates/akasha-core/src/session.rs` 已有）；
  隧道实体自持有条目（规则 id、状态、连接句柄），生命周期挂在它自己的 Session 上；
  状态变化事件按 `SessionId` 路由。
  **粒度取 1:1**：一条转发规则对应一个 `Tunnel` Session（`scope.md` §5.1 的表格写的是
  "一条或多条隧道"，本文把上界收敛到一条）。
- **理由**：遵循 `scope.md` §2.2 / §5.1 的总原则（转发不属于终端、各自持有连接）；1:1 使
  "关闭 Session = 终止**这一条**规则的重连循环与传输"（阶段 6 的 plan 0606）无需引入
  "对 Session 的部分操作"这种半开状态 —— 状态机的每个状态都只描述一条规则。多规则合并为
  一个 Session 会把"其中一条失败"变成"整个 Session 处于什么状态"的问题。
- **否决的替代路**：一个 Session 承载多条规则 —— 表达力更强，代价是上述半开状态。
  若将来确有需要（例如"一组隧道一起启停"），那是一个**新的容器概念**，按修订记录另立决策。

## 6. 认证与凭据（D7–D8、D16）

### D7 —— 认证顺序：agent → 密钥池 → keyboard-interactive → password；`partial_success` 时续接

- **决定**：按上述顺序尝试，每种仅在前一种失败后执行；`AuthResult::Failure` 且
  `partial_success = true` 表示"该方法已被接受、还需另一种"，此时按服务端给出的
  `remaining_methods` 继续，**不从头重来**。
- **理由**：agent 路径的副作用最少（不重复询问、私钥不进入本进程）；keyboard-interactive 是
  2FA / OTP 的唯一入口，必须排在 password 之前（与 OpenSSH 的默认顺序一致）。
- **否决的替代路**：只支持公钥（2FA 主机无法连接）；并行尝试多种方法（服务端会按自己的顺序
  累计失败次数，容易触发账号锁定）。
- **实现状态**：已落地（plan 0502），顺序经协议交互验证；`SshAuth` 额外携带 `agent_socket` 作为
  输入（Rust 2024 中 `std::env::set_var` 是 `unsafe`，而本仓库只允许 `akasha-store` 出现
  `unsafe`，故"agent 在哪"必须作为输入传入）。

### D8 —— 内存凭据缓存：键 = `(host, port, user, 认证方式)`，值为保护页中的口令；不落盘

- **决定**：缓存**不存解密后的私钥**，只存"打开密钥 / 登录所需的口令"
  （与 `Passphrase` 同族，存于 `memsafe::Secret` 的受保护页）；私钥本体继续只存在于密钥池那一页。
  命中时不重复询问 —— 这是"同一主机开三个 Session 只问一次凭据"（plan 0502 的判据）的落点。
- **失效条件**（三条均须实现）：① 库锁定 / 进程退出（清空）；② 该主机的认证被服务端拒绝
  （删除该条，否则会以错误口令反复重试）；③ 用户显式"忘记"（一个命令）。
  **没有 TTL** —— 超时后自动遗忘会让挂起的隧道在重连时突然弹出询问。
- **理由**：只缓存口令而非私钥，去掉了"普通堆内存中多一份长期私钥"这一最昂贵的副本
  （判据表见 ADR-0002 D13，新增用途须按该表重新验证）。
- **无法纳入保护页的副本（如实记录）**：`russh` 需要 `ssh_key::PrivateKey` 才能签名，其解析出的明文密钥
  位于**普通堆内存**，我们无法将其放入保护页。缓解措施 = 只在握手窗口内存在、签完即 drop、
  **不进入缓存**。ADR-0002 已定案（不可修改），因此该副本记录在本文 D8，不回填 ADR-0002 §7.5。
- **实现状态**：已落地（plan 0502 / 0503）。缓存键在实际实现中为私钥口令一项追加了**密钥标识**
  （同一主机的两个密钥各有各的口令，只按主机缓存会导致解密失败）。

### D16 —— 提问往返：事件 + 三条回答命令，超时 120 s；库锁定时不连接（plan 0504 定案）

- **决定**：`russh` 同步回调中需要询问用户的两件事（凭据 D8、未见过的 host key D11）共用
  **同一条**往返：后端 `emit` 事件 `ssh_prompt`（判别式区分 host key / 凭据），前端以三条命令
  之一作答（`ssh_prompt_credential` / `ssh_prompt_host_key` / `ssh_prompt_cancel`）；
  后端撤回提问时再 `emit` 一次 `ssh_prompt_dismissed`。**超时 120 s，超时即拒绝**
  （既不接受，也不留下悬置的连接）。**库锁定时不连接**（信任记录与密钥池都在库中），
  "读不到信任记录"一律拒绝，不得当作未知处理。
- **理由**：
  - **超时既是安全阀也是上界**：提问期间连接线程停在一次同步回调里（该回调占用 SSH runtime 的
    一个 worker），无人作答则永不返回。120 s 既够用户查看 2FA 验证码，也限定了"一个 worker
    最多被占用多久"。
  - **答案只走命令、不进事件**：事件是**朝向**前端的（任何订阅者都可见），而口令只许从前端
    往回走 —— 因此它的入口只有一个（`PassphraseInput`，无 `Debug` / `Clone`），进入后立即
    转为受保护页。
  - **"未作答" ≠ "拒绝"**：host key 那一问只接受"接受 / 拒绝"；超时 / 取消 / 答错类型一律
    视为**拒绝连接**（`HostKeyUnknown`），不得读成"用户已接受"。
  - **前端退出不得长期占用连接线程**：由超时与 `ssh_prompt_dismissed` 共同解决；
    用户也可在提示面板显式取消（使连接以"用户取消"结束）。
- **否决的替代路**：① 由 `akasha-ssh` 自行 `emit` —— 它不依赖 Tauri（`crates/*` 不得
  `use tauri::*`），且"问谁"是 app 的职责；② 把提问做成普通命令的返回值（前端轮询）——
  连接线程等待答案的位置在同步回调内，轮询要么丢失答案，要么把回调变成忙等。
- **代价（如实记录）**：提问是**同步阻塞**的（trait 本身同步），因此会占用 SSH runtime 的一个
  worker。缓解措施 = D2 的 worker 数 + 本条的超时，两者缺一都会让一条无人处理的连接长期
  占用 worker。
- **实现状态**：已落地（plan 0504）。跳板链上每一跳各产生一轮提问（host key + 凭据），
  E2E 实测四问按序发生。

## 7. 原语与转发（D9–D11）

### D9 —— `direct-tcpip` 原语只实现一次，形状为一条 `AsyncRead + AsyncWrite` 的流

- **决定**：`akasha-ssh` 暴露一条 `direct_tcpip` 原语（`channel_open_direct_tcpip` +
  `Channel::into_stream()`），对调用方表现为**一条 `AsyncRead + AsyncWrite` 的流**；
  该流由三处复用：**跳板**（`connect_stream` 直接以它为下一跳的底层流）、**`-L`**
  （在本地监听 socket 与它之间搬运字节）、**SFTP 的 B 档**（host ↔ host 的数据面）。
  连接句柄**不出 `akasha-ssh`**：原语挂在持有连接的实体上，而不是自由函数。
- **理由**：`connect_stream` 的签名只要求 `AsyncRead + AsyncWrite + Unpin + Send`
  （`client/mod.rs:1102`），因此"以通道作为下一跳的底层流"是上游现成的用法；形状定为流之后，
  另两处只是它的消费者，不需要各自再发明一条路径。
- **否决的替代路**：为跳板单独编写一条连接路径 —— 三处各写一遍，SFTP 与转发到来时仍需改动。
- **实现状态**：已落地（plan 0505），原语形状未变，补齐了两处原先未定死的细节：
  - **句柄归属**：选择"这条路自己持有连接"（另一条路是给 `SshTransport` 开受控借用口）。
    因此原语不接收 `&Handle` 之类的自由参数，而是由 `SshConnection`（一条已认证、没有通道的连接）
    持有句柄并暴露 `SshConnection::direct_tcpip(&self, host, port)`。句柄因此不出 `akasha-ssh`，
    也不需要回答"谁在何时可以访问该句柄"。
  - **流的类型**：对外类型是 `SshStream`（我们自己实现 `AsyncRead + AsyncWrite` 的 newtype），
    而非 `russh::ChannelStream` —— 后者是 0.x 的类型；这与 `HostKey` 把上游公钥留在私有字段中
    遵循同一条纪律：升级 `russh` 时改动止步于 crate 内。
  - 跳板链上每一跳都是一个 `SshConnection`，它们随 `Established::carriers` 一起 move 进
    那条 `pump` task —— "task 结束 = 整条链结束"，收尾只有一个出口（D5 的"关闭 Session
    立即关闭连接"因此对整条链成立）。
  - **三处消费者的进度**：跳板已落地（plan 0505）、`-L` 与 `-D` 已落地（plan 0602 / 0603，
    同在 `akasha-ssh::relay` —— 两者的差别只是目标从哪来，由一个 `Ingress` 表达）、
    SFTP 的 B 档仍未接（plan 0703）。原语本身自 0505 起**未改**。

### D10 —— `-R` 是另一套机制：`tcpip_forward` + Handler 回调

- **决定**：`-R` 不属于 D9 的原语。请求 `tcpip_forward(addr, port)`（`port = 0` 时使用返回值），
  入站连接由 `Handler::server_channel_open_forwarded_tcpip` 回调交给该隧道 Session 的 task；
  规则不活跃时**拒绝**（drop `reply`）；关闭时执行 `cancel_tcpip_forward` 并取消 task。
- **理由**：`scope.md` §2.2 明确规定"服务端发起"（`forwarded-tcpip`）与 `direct-tcpip` 不是
  同一条路径。Handler 是 russh 唯一能把入站 channel 交给我们的位置 —— 因此它必须持有回到
  Session 的通道，这也是它必须与 Session 一起构造的原因。
- **实现状态**：已落地（plan 0604）。落在 `akasha-ssh::remote`：`SshConnection::remote_listen`
  发请求，入站通道经 `Handler` 的回调进 `Inbound`（**按端口**查表，查不到就 drop `reply`），
  每条连接**先连本机目标再接受**那条通道，停止时 `cancel_tcpip_forward`。上面没写、由 0604
  补上的有三条：一是**目标不可达时的顺序** —— 我们这一侧唯一能对外说的话是那次通道拒绝
  （`ConnectFailed`），"先接受、连不上再关"会让对端的客户端看到一条连上就断的连接，
  而 OpenSSH 是前一种；二是那个连接**必须在任务里做**：Handler 的回调在连接的消息循环上被
  `await`，在回调里等一个不可达地址会让整条连接无响应（连保活都停）；三是**路由按端口认**，
  服务端回报的 `connected_address` 是"它认为在听的地址"（与服务端自己的 `GatewayPorts`
  一类配置有关），按地址字符串认会在真实服务端上静默失配。远端绑定地址**不做**回环限制 ——
  那个端口开在服务端，合规与否是它的策略（与 `-D` 相反，理由见 D9 的 0603 那一条）。

### D11 —— known_hosts：读取 `~/.ssh/known_hosts` + 库内缓存；不写用户文件；不匹配即拒绝

- **决定**：`check_server_key` **强制实现**（上游默认拒绝一切）；未知主机走"用户确认"
  （判断策略与提问往返见 D16，其接线在 plan 0504）；已确认的密钥写入**我们的库**；
  `~/.ssh/known_hosts` **只读**（`check_known_hosts_path` 支持它，含 `KeyChanged { line }`）。
  密钥变化时**拒绝连接并提示**，既不静默接受，也不静默改写。
- **理由**：不修改用户的文件 —— 它是 `ssh` 与其它 GUI 工具的共用资产，抢占写入会互相破坏；
  库内缓存跟随可搬迁的数据目录（`docs/portable.md`）。
- **否决的替代路**：直接调用 `learn_known_hosts`（会写用户文件，且便携模式下该文件不在数据目录内）。
- **实现状态**：已落地（plan 0503），判定顺序为库 → 用户文件（只读）→ 提问；本 ADR 的决定未变。
  落地时确认了库格式 `user_version` 1 → 2 的首次迁移（属 ADR-0002 辖区，记录见归档的 plan 0503
  与 `docs/STATUS.md`）。

## 8. 隧道状态机与重连（D12–D13）

### D12 —— 五态状态机 + 事件 + 手动重试

- **决定**：`连接中 / 已连接 / 重连中(n) / 失败 / 已停止`（`scope.md` §2.2 的原文五态）；
  状态变化发出事件（按 `SessionId` 路由，**界面未实现也先发**）；`失败` 有落点（托盘菜单）；
  提供手动重试（重试 = `失败 / 已停止 → 连接中`，并将尝试次数清零）。
- **理由**：`scope.md` §2.2 的"失败必须可见"与 §5.1 第 2 条（事件按 `SessionId` 路由）。
  状态机本身**不依赖 russh**：它是纯逻辑，放在 `akasha-core` / `akasha-ssh` 的可测部分。
- **实现状态**：已落地（plan 0601）。状态机落在 `akasha-core::TunnelState`（纯逻辑、零 Tauri
  —— 这正是上面那句"不依赖 russh"的落点），事件名是 `tunnel_state`，IPC 侧的状态短名与
  `TunnelState::as_str` 逐字相同。上面没写、由 plan 0601 补上的是**转移表**（§12 原先留的
  就是它）：`重连中(n)` 的 `n` 必须 ≥ 1，同态转移非法，且**重连成功也必须经过 `连接中`**
  —— 否则"状态没变"与"变了一次"在事件流里无法区分。
  到 plan 0605 才第一次有人产生 `重连中` 这条边（在那之前，状态机里有那一态而没有任何代码
  会走到它），同时补上「`失败` 有落点」这一半的**最后一环**：状态变化必须重推托盘菜单
  （`Sessions::set_tunnel_state` 此前只在登记 / 注销时通知订阅者，于是托盘会一直停在隧道
  刚登记时那一行 —— 菜单是一份快照，不重推就不更新）。手动重试那条边补了一处顺序：
  **只有终态能重试，且这条检查排在绑定端口之前**。

### D13 —— 重连参数与"哪些失败不重连"

- **决定**：默认 **3 次**，退避 **1s → 2s → 4s**（`initial = 1s`、`factor = 2`、
  `max_attempts = 3`，均可配置；默认值写在配置模型里，不散落在代码中）。
- **判据**：

| 失败类别 | 行为 |
|---|---|
| 传输层断开（`Error::Disconnect` / `HUP` / `IO` / 超时） | 进入 `重连中(n)`，按退避重试 |
| **认证失败**（`AuthResult::Failure` 且无可用剩余方法） | 直接 `失败`，不重试 |
| **主机密钥不匹配**（`Error::KeyChanged`） | 直接 `失败`，不重试，要求用户显式确认 |
| 配置 / 协议错误（`NoCommonAlgo`、地址不存在） | 直接 `失败`，不重试 |

- **理由**：无限重连会持续扰动防火墙（`scope.md` 已否决）；区分"网络断开"与"口令错误"是
  同时满足"自动恢复"与"不锁定账号"的唯一做法 —— 以错误口令连续尝试三次正是账号锁定的经典成因。
- 补充：退避**不加抖动**（一个 Session 一次只连一条，不存在惊群问题）；若将来需要，它是配置项，
  不需要改代码。
- **实现状态**：已落地（plan 0605）。三个数落在 `akasha_core::config::Reconnect`
  （`Config` 的一个字段），退避序列由 `Reconnect::delay` 一处算出；驱动它的是每条隧道一个
  **看护任务**，在**首次连上之后**才起。上面没写、由 plan 0605 定死的是三件事：
  **「断开」怎么认**（转发任务按 500 ms 看一眼 `Handle::is_closed()` —— 上游 0.x 没有可
  `await` 的关闭信号，而它对「半死」的连接不敏感：TCP 没断、对端不回话时要等保活耗尽，
  即 `keepalive_interval × keepalive_max`，默认约 90 秒）；**重连 = 重新走一遍
  「准备 + 连接 + 起转发」**（所以 `-R` 会重新发一次 `tcpip_forward` —— 远端监听是服务端
  那条连接的资源，连接一断它就被撤销了；规则里写 `port = 0` 时重连后可能换一个端口）；
  **看护任务只有一个停止入口**（一条 `oneshot`，停止 / 重试 / 退出都从它进去，且它在每个
  `await` 上回应 —— 这是 D5 的「关闭 Session 立刻关闭连接」在重连这条路上的落点）。
  本 plan 另定了一条**不在 D13 表里**的口径：**首次连接失败不自动重试** —— 表说的是
  「传输层**断开**」，而断开的前提是曾经连上过；一次都没成功的那次失败是同步报给用户的
  （命令返回里带原因，界面上就是那条失败文案），下一步动作在用户手上。

## 9. 配置导入（D14）

### D14 —— `~/.ssh/config` 只做受限子集；`Match` / `Include` / 未知关键字显式报错

- **决定**：按 `scope.md` §8 风险 4 的已定案结论执行，支持的指令**限于六条**：
  `Host` / `HostName` / `User` / `Port` / `IdentityFile` / `ProxyJump`。
  解析器**不猜测**：遇到 `Match` / `Include` 或任何无法识别的关键字时**显式报错**，
  不得静默跳过（静默误解析的后果是连接到错误主机）。`Host` 的多段匹配按 OpenSSH 的
  "每个参数**首次取到的值生效**"语义实现，不自造优先级。
- **理由**：`ssh -G` 那条捷径随 §2.1 的定案作废（它依赖系统 `ssh`）；完整解析器是本项目
  最容易被低估的复杂度，而"受限子集 + 显式报错"把它变成一条可被测试的边界。
- **实现状态**：已落地（plan 0506）；支持集在导入界面上列出（见下）。
  私钥**不随配置导入**：`IdentityFile` 只令条目落成 `publickey` 认证且 `key_id` 留空
  （即使用 ssh-agent），导入报告中逐条说明。

#### D14 的三档边界（plan 0506 展开）

"报错"与"警告"的分界线是**后果**，不是"是否认识"：

| 档 | 判据 | 行为 |
|---|---|---|
| **导入** | 六条 | 映射进 ssh 配置池 |
| **警告** | 忽略它既不改"连到哪台机器"，也不改"信任哪把主机密钥"，且"未生效"会逐条出现在报告中 | 逐条列出并继续 |
| **报错** | ① `Match` / `Include`（条件与内容都不在我们读到的文本里）② 改目的地或解析路径（`ProxyCommand` / `Canonicalize*` / `BindAddress` / `AddressFamily` / `Tunnel`…）③ 改信任来源（`HostKeyAlias` / `RevokedHostKeys`）④ `IgnoreUnknown` ⑤ **表中没有的** | 整份不导入，**一次列全**（带行号） |

兜底方向是最后一格：**未收录的一律报错**。因此"漏判一条会改目的地的关键字"的后果是它落入
报错档，而不是被静默放行。规则与名单只有一处实现（`akasha-store::sshconfig` 的 `classify`）。

同批定下的三条细节：

- **`ProxyJump` 只接受裸主机名**（`host`，逗号分隔表示一条链）与 `none`；`user@host` /
  `host:port` / ssh URI **报错**并给出改法。理由：池中一跳就是**一行**，而 `user@` / `:port`
  是对某一跳的局部改写，库里没有对应表示 —— 强行落入会变成一条用户从未写过的行。
- **`IdentityFile` 只部分兑现**：私钥不导入（导入配置与导入机密是两类操作，后者须按 ADR-0002 D13
  重新验证），条目按 `publickey` 落库、`key_id` 留空（即走 ssh-agent），报告中逐条说明。
- **求值语义以 `ssh -G` 的实测为准**（系统 OpenSSH 10.5）：`Host *` 写在前面时其取值**压住**
  后面的具体条目（"首次取到的值生效"）；文件开头到第一个 `Host` / `Match` 之间是**全局段**；
  **关键字**不区分大小写而 **`Host` 模式**区分大小写；未写 `HostName` 时目标名被小写化；
  **通配块不产生条目**，其取值靠匹配并入具体条目。

## 10. 不可逆点（改动代价为一整层）

1. **`Transport` 的 SSH 映射**（D3 / D4）：capability 一旦进入 IPC 类型与前端，反向改动要同时
   修改前端与生成物。
2. **连接所有权**（D5 / D6）：`Session` 与连接的 1:1 关系会同时出现在注册表、事件、托盘菜单三处。
3. **原语的形状**（D9）：跳板 / SFTP B 档 / `-L` 都按"流"消费它。
4. **隧道状态机**（D12）：状态名与事件名会进入 `src/ipc/bindings.ts` 与前端。

## 11. 代价与对策

| 代价 | 对策 / 现状 |
|---|---|
| 进程内多一个 tokio runtime（与 tauri 自身的并存） | 只在 `akasha` 创建一处、worker 数可配；退出路径先 `shutdown_all` 再 drop runtime（顺序不可颠倒）。plan 0504 落地的形态：runtime 由 `State` 持有，`RunEvent::Exit` 中先回收会话，runtime 在 tauri 拆除 `App` 时才 drop —— 顺序由生命周期保证，无需手写两次 |
| 每次连接都重新握手（不复用） | D7 的顺序 + D8 的缓存；有 agent 时天然不重复询问 |
| 明文私钥在握手期间短暂存在于普通堆 | D8 的两条缓解措施；不缓存、不落盘、签完即 drop |
| 上游把 `ssh-key` 钉在预发布版 | 升级 `russh` 时一并评估；`=0.63.3` 使"升级"成为一个有意的动作 |
| 0.x 的 API 会变动 | 用法集中在 `akasha-ssh` 一处（app 只认 `Transport`），升级面被限制在一个 crate 内 |

## 12. 未决问题（留给后续 plan）

- SFTP 的 B 档消费 D9 原语的具体接法 —— plan 0703。

（`~/.ssh/config` 子集的解析细节已由 plan 0506 落地，见 §9；指令清单本身在 D14 定死。）

## 13. 复审条件

- **完整 supervisor 立项时**（与 ADR-0005 §6 同一条）：SSH / 隧道的回收是否仍由
  "app 自持资源 + 进程外看门狗"承担，还是改为"资源归 supervisor，app 只是客户端"。
- **`russh` 发布 1.0（或 `ssh-key` 转为正式版）时**：重估 D1 的 `=` 钉法与 feature 组合。
- **出现"一条连接多路复用"的硬需求时**（例如同主机数十条隧道）：重估 D5 —— 届时引入的是
  引用计数 + 独立的重连 / 复用状态机，本 ADR 的"一实体一连接"被取代而非叠加。
- **两个 tokio runtime 成为可观测的负担时**：重估 D2（改为复用宿主 runtime 的 handle）。

## 14. 修订记录

| 日期 | 改动 | 理由 |
|---|---|---|
| 2026-09-12 | 初稿；状态置「实现中」 | plan 0501：在改动 `akasha-ssh` 之前先确定协议面与资源模型 |
| 2026-09-13 | D3 / D4 中"落地时要补"的两条改为**已落地**（`TransportError::Busy`、capability 注释）；新增 **D15**（超时 / 保活 / `nodelay`）；§12 删除已定的三条；把"`-R` 远端监听消失"这条验收的归属从 plan 0502 改到 **plan 0604** | plan 0502 落地了 D3 / D4 指派给它的两条；`-R` 属于阶段 6，原先将该验收写在 D4 的 0502 段属于**归属错误**（改动须在 ADR 状态之外留痕，见 `docs/adr/README.md` 的三态） |
| 2026-09-13 | 本文引用的阶段 5 plan 编号**全部重排**（known_hosts 0505 → **0503**、`direct-tcpip` 0503 → **0505**、config 导入 0504 → **0506**；新增 **0504** = SSH 接进 IPC / 前端）；§12 补入"未知 host key 的提问形态"（跨 plan 0503 / 0504） | 执行顺序改由依赖决定：D11 的信任策略（谁问、问什么、答案存哪）属于**接口形状**，必须在 plan 0504 把它送到前端之前确定，否则 0504 只能自造一套临时信任；`direct-tcpip`（D9）的三处消费者都需要先有一条从 app 可建立的会话才能验证 |
| 2026-09-13 | 新增 **D16**（提问往返的形状与超时、库锁定时不连接）；D2 标记为"已落地"；§12 删除已定的两条 | plan 0504 把 D8 / D11 的两个提问口接到前端，同时确定了"谁来问、超时多长、无人作答怎么办"。D2 / D3 的实现形状未变 |
| 2026-09-13 | **D11 按原文落地**（plan 0503）：三态判定、`~/.ssh/known_hosts` **只读**、确认过的密钥进**我们自己的库**；决定未变，仅把关联指针改到归档路径 | D11 当初已把策略定死，实现未推翻它。落地带出一个新事实（库格式 `user_version` 1 → 2，本仓库首次迁移），属 ADR-0002 辖区，记录在归档的 plan 0503 与 `docs/STATUS.md` |
| 2026-09-13 | **D9 按原文落地**（plan 0505）：原语形状 = 一条 `AsyncRead + AsyncWrite` 的流，实现为 `SshConnection::direct_tcpip`；对外类型是 `SshStream` 而非 `russh::ChannelStream`；新增 `SshError::Forward`；补上原先未写死的两处（句柄归属、流的类型） | D9 定死了"只实现一次"与"形状是流"，实现未推翻它。补的两处是它留下的空白：骨架阶段已写明原语获取句柄只有"受控借用口"与"自己持有连接"两条路，本 plan 选择后者；流的类型沿用 `HostKey` 的做法，不把上游 0.x 的类型暴露进公开签名 |
| 2026-09-13 | **D14 展开**（plan 0506）：补上三档边界（导入 / 警告 / 报错）及各自的**判据**、`ProxyJump` 只接受裸主机名、`IdentityFile` 只部分兑现、求值语义的四条实测依据；`Match` / `Include` / 未知关键字仍然报错，决定未变 | D14 只写了"无法识别即报错"，而真实配置中包含大量与连接无关的指令（`ServerAliveInterval`、`IdentitiesOnly`、`LocalForward`…）—— 照字面执行会让该功能在任何一份真实配置上失败，"受限子集"退化为"无人可用"。分界线因此从"是否认识"改为"忽略它是否会连到别的机器、或改变我们信任哪把主机密钥"，并保留"表中没有的一律报错"作为方向 |
| 2026-09-13 | 文档整理：补决策索引；D9 的签名表述统一为"流 + 句柄不出 crate"（原表述 `direct_tcpip(&Handle, …) -> ChannelStream` 与已落地形态矛盾）；§2 的 sshd 一条注明阶段 5 未使用；§12 移除已由 0506 落地的条目；引用代码位置改用符号名而非会漂移的行号 | 阶段 5 收尾核查：消除 ADR 内部与跨文档的不一致 |
| 2026-09-13 | **D12 落地**（plan 0601）：补上**转移表**与事件名（`tunnel_state`）、`重连中(n)` 的次数下界（≥ 1）与两条非法边（同态、重连直达 `已连接`）；`SshConnection` 增加同步门面 `connect_via` 并持有它自己的跳板链，建链只留 `hops_chain` 一份实现 | 五态集合与"状态变化发事件"由 D12 定死，实现未推翻它；它留给 plan 0601 的正是"每条边的触发条件"。补写时定死那两条非法边，是因为它们决定**事件流可不可读**（"状态没变"与"变了一次"必须分得开）。`SshConnection` 的两处形状补充同 D9 当年补"句柄归属"与"流的具体类型"：D9 只说"只实现一次"，没说隧道那条路怎么持有一条没有通道的连接 |
| 2026-09-13 | **D9 的原语多一处消费者**（plan 0603）：`-D` 的 SOCKS5 服务端在 `akasha-ssh::socks5`（无认证的 `CONNECT`），与 `-L` 共用 `akasha-ssh::relay`（两条路的差别是 `Ingress::Fixed` / `Ingress::Socks5`）；`SshError::Forward` 多带一档 `class`（`ForwardFailure`，取自上游结构化的 `ChannelOpenFailure`），新增 `SshError::NotLoopback`；原语与 `SshConnection` 的形状**未变** | D9 把原语定成一条流，`-D` 只是"目标由客户端说"的那一个消费者，未推翻它。补的两条是它没写的：一是**哪个字节承担错误消息** —— SOCKS5 客户端只收到一个 `REP`，因此通道开不出来时失败必须分类（"服务端不允许转发"与"目标服务没起来"要分得开），而分类只能取自结构化的 `ChannelOpenFailure`，不能从错误字符串里猜；二是**无认证的监听不许绑非回环地址** —— 本版本没有取得明确同意的那一轮询问（D16 的往返只覆盖凭据），因此 `127.0.0.1` / `[::1]` / `localhost` 之外的绑定地址一律拒绝，且拒绝发生在**绑定之前** |
| 2026-09-13 | **D9 的第三个消费者落地**（plan 0602）：`-L` 在 `akasha-ssh::relay` 里 —— `LocalListener`（**先绑**）+ `LocalForward`（接受循环 + 每条入站连接一条 `direct_tcpip` 通道 + `copy_bidirectional`）；新增 `SshError::Listen` 把"本机端口没拿到"与 `Connect`（对端连不上）分开；原语与 `SshConnection` 的形状**未变** | D9 说这条流有三处消费者、`-L` 是其中之一，实现未推翻它。补的一条是它没写的：**绑定先于握手** —— 端口被占用是本类功能最常见的一类失败，而它必须在"用户答凭据"之前就失败（否则那两轮提问全是白费的）。`SshError::Listen` 单独一档同 `Forward` 当年的理由：用户的下一步动作不同（腾端口 vs 查网络） |
| 2026-09-14 | **D10 落地**（plan 0604）：`-R` 在 `akasha-ssh::remote` —— `SshConnection::remote_listen` / `cancel_remote_listen` + `RemoteForward`，入站路由挂在连接上（`Inbound`，`Handler::server_channel_open_forwarded_tcpip` 按**端口**查表）；`handshake` 的返回值由 `Handle<Handler>` 变成 `Authenticated`（多带回一份入站入口）；新增 `SshError::RemoteListen` 与 `TunnelError::RemoteBind`，**删除 `TunnelError::Unsupported`**（三个方向都支持之后它再也出不来）；`Rule::ingress` → `Rule::prepare`（多一档 `Prepared`） | D10 把 `-R` 定成与 D9 无关的另一套机制，实现未推翻它：请求、Handler 回调、拒绝、撤销四件事与它写的一字不差。补的三条是它没写的：**先连本机目标再接受通道**（拒绝才是对端能收到的唯一解释）、那个连接**必须在任务里**（Handler 回调在连接的消息循环上被 `await`）、**按端口而不是按地址字符串**认入站通道。删除 `Unsupported` 是三个方向都支持的必然结果 —— 留着一个永远出不来的错误档就是在文档里留一句假话 |
| 2026-09-15 | **D12 / D13 落地**（plan 0605）：补上「断开」怎么认（转发任务按固定间隔看 `Handle::is_closed()`，`ForwardEnd` 区分「被停止」与「连接没了」）、重连 = 重新走一遍准备 + 连接 + 起转发（`-R` 因此要重新发一次 `tcpip_forward`）、按层分档的 `TunnelError::retryable`、看护任务的单一停止入口（`TunnelRun`）、`set_tunnel_state` 重推托盘菜单（「失败可见」的最后一环）；新增 `Config::reconnect` | D13 原文只说「传输层断开 → 重连」，没写**断开由谁在什么时候认出来** —— 上游 0.x 只给了同步的 `is_closed()`，这是一个必须写下来的实现约束（它同时决定了半死连接的发现延迟是保活量级）。「重连 = 重来一遍」是同一条决定的另一半：连接是那条转发的命根子，所以三个方向里 `-R` 的准备那一步（`tcpip_forward`）也必须重做，否则会出现「重连成功、端口却不在听」。托盘的刷新是 D12 那句「`失败` 有落点」在实现上的最后一环：管线从 plan 0301 起就在，但状态变化从没通知过它 |
