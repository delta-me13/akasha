# ADR-0003：SSH 栈与资源模型

- **状态**：**实现中**（Implementing，2026-09-12）
- **日期**：2026-09-12
- **决策者**：cyrene
- **影响范围**：新 crate `src-tauri/crates/akasha-ssh`；`Transport` 在 SSH 上的映射与
  capability 取值；`Session` ↔ 连接的**所有权**；隧道状态机与事件；`akasha` 侧的 tokio runtime；
  阶段 6 / 7 的接口形状
- **关联**：[plan 0501](../plans/archive/0501-adr-0003-ssh-stack.md)（本文的落地）、
  [0502](../plans/archive/0502-ssh-connect-auth.md) / [0503](../plans/archive/0503-known-hosts.md) /
  [0504](../plans/0504-ssh-into-ipc-frontend.md) / [0505](../plans/0505-direct-tcpip-primitive.md) /
  [0506](../plans/0506-ssh-config-subset-import.md)、阶段 6 全部
- **取代**：无

---

## 1. 背景

`docs/scope.md` §2.1 / §2.2 / §5.1 已经定案了 SSH 这一层的**产品级**结论。本文做的是把它们
转成**可实现、可核对**的架构决策，并把"改它要重写一整层"的那些点写死：连接模型、
`Transport` 映射、`direct-tcpip` 原语的形状。

写它的时机见 [`README.md`](./README.md)：动 `akasha-ssh` 的代码**之前**。因此本文只定形状与
约束，不含实现步骤（那些在 plan 0502 起）。

| `scope.md` 已定案的条目 | 本文对应的决策 |
|---|---|
| §2.1 纯 Rust（`russh`），不调系统 `ssh` | D1 / D2 |
| §2 能力差异用 capability flag 表达，不用新 trait | D3 / D4 |
| §2.2 三条转发路径 + "一个原语服务三处" | D9 |
| §2.2 远程转发是**另一套**机制 | D10 |
| §2.2 转发是独立 Session | D6 |
| §2.2 连接生命周期 = 拥有它的 Session 的生命周期 | D5 |
| §2.2 不复用连接 + 三条副作用对策 | D5 / D7 / D8 / D11 |
| §2.2 有限次重连、状态机、**失败必须可见** | D12 / D13 |
| §5.1 资源挂 Session、事件按 `SessionId` 路由、类型互不依赖 | D5 / D6 / D12 |
| §8 风险 4 `~/.ssh/config` 受限子集 | D14 |

## 2. 事实依据（调研，2026-09-12）

以下每一条都是**可核对**的：来源 = crates.io API 与从 `static.crates.io` 下载的
`russh-0.63.3` 源码（行号为解包后的路径），本机侧 = 本仓库的 `Cargo.lock` 与 `rustc --version`。

| 事实 | 值 | 出处 |
|---|---|---|
| `russh` 最新稳定版 | **0.63.3**（2026-09-09 发布，共 130 个版本） | crates.io `/api/v1/crates/russh` |
| 许可证 | Apache-2.0（`deny.toml` 的 allow 已含） | 同上 |
| edition / MSRV | edition 2024，`rust-version = "1.89"` | 解包后的 `Cargo.toml` |
| 本机工具链 | rustc **1.98.1** | `rustc --version` |
| crypto 后端 | `ring` 与 `aws-lc-rs` **二选一必开**，都不开直接 `compile_error!` | `src/lib.rs:90-94` |
| `ring` 依赖 | `0.17.14` | `Cargo.toml` |
| `aws-lc-rs` 依赖 | `1.16.2` | `Cargo.toml` |
| `ssh-key` 依赖 | **`=0.7.0-rc.11`（预发布，上游钉死）** | `Cargo.toml` |
| 上游特性集 | `default = ["flate2", "aws-lc-rs", "rsa"]` | `Cargo.toml` `[features]` |
| 本仓库 lock 里已有的 `ring` | **0.17.14**（由 `rustls` / `quinn-proto` / `reqwest` 带入） | `src-tauri/Cargo.lock` |
| 本仓库 lock 里已有的辅助 crate | `bytes` 1.12.1 / `tokio-util` 0.7.19 / `zeroize` 1.9.0 / `memsafe` 1.0.2 | 同上 |
| 重连/退避设施 | 上游**没有**（全源码搜不到 `reconnect`） | 全源码 grep |
| 本机有真 `sshd` | `/usr/bin/sshd`（plan 0502 起可真机验收） | `which sshd` |

**API 形态**（全部取自 0.63.3 源码）：

| 需要的能力 | API | 位置 |
|---|---|---|
| 建连接 | `client::connect(Arc<Config>, addrs, handler) -> Handle<H>`；`connect_stream(...)` 可接**任意** `AsyncRead + AsyncWrite` | `client/mod.rs:1083` / `:1102` |
| 服务器密钥校验 | `Handler::check_server_key(&PublicKeyOrCertificate)` —— **默认实现拒绝一切**，必须覆写 | `client/mod.rs:2368` |
| 密码 / 公钥 / 键盘交互 | `authenticate_password` / `authenticate_publickey` / `authenticate_publickey_with(user, key, alg, &mut S)` / `authenticate_keyboard_interactive_start` | `client/mod.rs:335` / `:456` / `:493` / `:356` |
| 认证结果 | `AuthResult::{Success, Failure { remaining_methods, partial_success }}` —— `partial_success` 就是 2FA 的续接点 | `src/auth.rs` |
| agent | `keys::agent::client::AgentClient::{connect_env, connect_pageant, connect_named_pipe}`，且 **`impl Signer for AgentClient<R>`**（可直接喂给 `authenticate_publickey_with`） | `keys/agent/client.rs:75` / `:96` / `:104`；`auth.rs:258` |
| 终端会话 | `channel_open_session()` → `request_pty(want_reply, term, cols, rows, px, px, &[(Pty, u32)])` → `request_shell()`；`window_change`；`data` / `data_bytes`；`ChannelMsg::{Data, ExtendedData, Eof, Close, ExitStatus, ExitSignal, …}` | `client/mod.rs:795`；`channels/mod.rs:205` / `:299` / `:326` |
| 本地转发 / 跳板 / SOCKS5 | `channel_open_direct_tcpip(host, port, originator_addr, originator_port) -> Channel<Msg>`；**`Channel::into_stream() -> ChannelStream`（`impl AsyncRead + AsyncWrite`）** | `client/mod.rs:838`；`channels/mod.rs:661`；`channels/channel_stream.rs:28` / `:41` |
| 远程转发 | `tcpip_forward(address, port) -> u32`（`port = 0` 时由服务端选）/ `cancel_tcpip_forward`；入站入口 = `Handler::server_channel_open_forwarded_tcpip(…, reply: ChannelOpenHandle)`（`reply.accept()` 确认，**drop 即拒绝**） | `client/mod.rs:885` / `:911` / `:2477` |
| known_hosts | `keys::known_hosts::{check_known_hosts_path, known_host_keys_path, learn_known_hosts_path}`；密钥变化报 `Error::KeyChanged { line }` | `keys/known_hosts.rs:24` / `:63` / `:137` |
| 保活 | `Config::{keepalive_interval: Option<Duration>, keepalive_max: usize}`，默认 **`None` / 3**；`Handle::send_keepalive` | `client/mod.rs:2284` 起的 `Config` 与紧随其后的 `Default` |

**读到的上游默认值**（会影响我们的取值）：`window_size = 2 MiB`、`maximum_packet_size = 32768`、
`channel_buffer_size = 100`、`keepalive_interval = None`、`inactivity_timeout = None`、`nodelay = false`。

## 3. 版本、feature 与运行时（D1–D3）

### D1 —— `russh = "=0.63.3"`，`default-features = false`，features = `["ring", "rsa"]`

- **决定**：精确钉版本；crypto 后端用 `ring`；开 `rsa`（老密钥仍常见）；**不开** `aws-lc-rs`、
  **不开** `flate2`；`dsa` / `des` 明确不要。
- **理由**：
  - `ring` 0.17.14 **就是** lock 里已有的那个版本（`rustls` / `quinn-proto` / `reqwest` 带入），
    不新增第二个 crypto 后端；`aws-lc-rs` 要 `aws-lc-sys`（cmake + bindgen，Windows 上还要 NASM），
    把四平台与将来 Android 的交叉编译成本加上去，换来的性能与本产品的瓶颈（网络 RTT）无关。
  - 上游 130 个版本、仍是 0.x，API 换得快；`=` 与 `tauri-specta` 同一条口径（`AGENTS.md` §5）。
  - 不开 `flate2`：压缩是**可协商**的，客户端不提供就退化成不压缩；而压缩本身是已知的
    侧信道面。真遇到只给压缩的服务器，再开一个 feature 即可。
- **否决的替代路**：`ssh2` / libssh2（绑 OpenSSL —— `scope.md` §2.1 已否）、调系统 `ssh`（同上）、
  用上游默认 features（会把 `aws-lc-rs` 拉进来，与本条的交叉编译理由冲突）。
- ⚠️ 记下：上游把 `ssh-key` 钉在**预发布** `0.7.0-rc.11`，我们的 lock 因此会含一个 pre-release。
  这不是我们选的、也不由我们控制；升级 `russh` 时一并看它有没有换成正式版。

### D2 —— `akasha-ssh` 不自建 runtime：只收 `tokio::runtime::Handle`

- **决定**：库不开线程池、不建 runtime；`akasha`（app 侧）在启动时建**一个**专用 tokio runtime
  并把 `Handle` 注入。单测用 `#[tokio::test]`（或自建 runtime）提供 handle。
- **理由**：库自己建 runtime 是公认的反模式（谁 drop、几个 worker、什么时候停都变成库的隐式行为）；
  `Handle` 注入让"进程里 runtime 只在一处配置"成立，也让 `akasha-ssh` 在没有 app 时可测。
- **否决的替代路**：① 库内懒建单例 runtime（drop 语义与测试隔离都变脏）；
  ② 用 tauri 的 `async_runtime`（`crates/*` 不得 `use tauri::*`，且要碰它没承诺的接口）。
- **代价**：`akasha` 的 `tokio` 从 dev-dependency 升为**真依赖**
  （features：`rt-multi-thread` / `time` / `sync` / `io-util` / `net`）。

### D3 —— 同步 `Transport` 是门面，异步在门后面

- **决定**：`Transport`（在 `akasha-pty`）保持**同步**不改；`akasha-ssh` 把连接作为 task 跑在注入的
  `Handle` 上，两个方向各一条 `tokio::sync::mpsc`：
  - **写**：`Transport::write(&[u8])` → 入队 → task `recv().await` → `Channel::data_bytes()`
    （背压由 russh 自己的窗口机制提供）；`resize` 同理 → `window_change`。
  - **读**：task 收 `ChannelMsg::Data` → 入队 → 同步侧用 `blocking_recv()` 实现
    `std::io::Read`（`output_stream()`），再交给现有的合批器 `akasha_pty::spawn_batcher`。
- **理由**：`src-tauri/src/session.rs:102` 已经持有 `Box<dyn Transport>`，SSH 因此**不需要**改动
  会话层与前端 —— 这正是 `scope.md` §2 要求"用 capability flag 而不是新 trait"的兑现。
- ✅ **这条契约已落地**（plan 0502）：`TransportError::Busy(&'static str)` 是队列满时的唯一说法。
  背景照记：`write_session` 是**同步** command
  （`session.rs:643`），而 async command 走 `crate::async_runtime::spawn`
  （`tauri-2.11.5/src/ipc/mod.rs:329`）—— 也就是说调用点**是否在 tokio 上下文里**取决于
  command 怎么写，门面不能假设。所以队列满时的行为必须是一个**显式错误**（`TransportError`
  需要一个新的背压变体，`Unsupported` / `Closed` 都不合适），而不是阻塞调用线程的
  `blocking_send`（它在 tokio 上下文里会 panic，在任何上下文里都可能把 UI 拖住）。

### D15 —— 连接取值：超时 10 s、保活 30 s × 3、`nodelay = true`（plan 0502 定案）

- **决定**：`connect_timeout = 10s`；`keepalive_interval = Some(30s)`、`keepalive_max = 3`
  （上游默认是 `None` / 3）；`nodelay = true`（上游默认 `false`，即 Nagle 开着）。
  默认值写在 `akasha-ssh` 的 `SshConfig::default()` 一处。
- **理由**：
  - **保活必须给一个值**：上游默认 `None` = 永不发保活，于是一条"半死"的连接
    （对端不响应、TCP 也没断）会一直挂着，而症状是**用户以为还连着**。
    30 s × 3 ≈ 90 s 发现它；更短会在长连接设备上多出可观的空包。
  - **`nodelay = true`**：交互式终端的输入是"一次按键一个小包"，Nagle 会把它们攒起来
    等确认 —— 那是可感知的输入延迟，而 SSH 交互**正是**这个场景。
  - 超时 10 s 是"够慢的网络也来得及、又不至于让用户以为卡死"，与阶段 6 的重连退避
    （1 s / 2 s / 4 s）是两件事：这一条管**一次**尝试活多久。
- ⚠️ 实测回填：本 plan 没有造出真实的"半死连接"与高延迟链路，所以这三个数值是
  **有理由的默认值**，不是实测出来的最优值。真机验收（阶段 6 的重连用例）时再回填。

## 4. `Transport` 的 SSH 映射（D4）

### D4 —— capability：SSH 终端 `resize + exit_status`；隧道两者皆无；`session_leader() = None`

| 载体 | `resize` | `exit_status` | `session_leader()` |
|---|---|---|---|
| SSH 终端（shell channel） | ✅ `window_change` | ✅ 远端 `ChannelMsg::{ExitStatus, ExitSignal}` | `None` |
| SSH 隧道（`direct-tcpip` channel） | ❌ | ❌ | `None` |
| 本地 PTY（现状） | ✅ | ✅ | 有（看门狗用） |

- **理由**："能力"按**载体**取值，不按后端。隧道是一条纯字节管道，它没有尺寸也没有结局。
- ✅ **那条冲突的注释已改**（plan 0502）：`akasha-pty/src/transport.rs` 里 `Capabilities::exit_status`
  现在的文档写的是"有没有结局"，并点明"没有本地进程"是 `session_leader()` 的事。
  原先写的是
  "能否给出退出结局。serial 与 SSH 没有本地进程语义。" —— **"没有本地进程"与"没有结局"是两件事**：
  SSH 没有本地 pid，但远端 shell 会报 exit-status / exit-signal。`scope.md` §2 的那句话
  （"SSH 没有本地进程语义"）因此仍然成立，它说的是 pid。
- `session_leader() = None` 是 ADR-0005 §6 要求**每个新载体**回答的那一问：SSH 这条路上没有
  本地进程要代收。app 被 SIGKILL 时 socket 由内核关闭，服务端随之拆掉 `-R` 的监听 ——
  所以看门狗对 SSH 无事可做。
  ✅ plan 0502 验的是**这一档本身**（`session_leader()` 返回 `None` 的断言在能力位那条用例里）。
  ⚠️ **"真退出（含 `kill -9`）之后 `-R` 的远端监听消失"这句原先挂错了地方** —— 它是
  **plan 0604** 的验收（`-R` 属于阶段 6，plan 0502 里连 `-R` 都还不存在）。改它的那次修订见 §14。

## 5. 所有权与生命周期（D5–D6）

### D5 —— 连接归 `Session`；一条连接只属于一个 `Session`；**不复用**

- **决定**：终端 Session 拥有一条 `Transport`（= 一条 SSH 连接 + 一个 shell channel）；
  隧道 Session 拥有"每条规则一条连接"；关 Session = **立刻**关连接（无宽限期）；
  窗口关闭（默认收托盘）**不影响**；退出 = 全部关闭 + 零残留。
- **理由**：`scope.md` §2.2 已定案。复用会带回"一条断全断"的耦合，而"每条隧道可独立重启"
  正是转发场景要的。
- **否决的替代路**：连接池 / multiplexing —— 它把"这条连接什么时候该关"变成一个需要引用计数的
  问题，而引用计数的 bug 表现为"随机关掉别人的隧道"。

### D6 —— 隧道是**独立**的 `Session`，**一条规则 = 一个 `Session`**，共用同一个 `SessionId` 空间与注册表

- **决定**：端口转发落在 `SessionKind::Tunnel`（`crates/akasha-core/src/session.rs:46` 已有）；
  隧道实体自己有条目（规则 id、状态、连接句柄），生命周期挂在它自己的 Session 上；
  状态变化事件按 `SessionId` 路由。
  **粒度取 1:1**：一条转发规则对应一个 `Tunnel` Session（`scope.md` §5.1 的表格写的是
  "一条或多条隧道"，本文把上界收到一条）。
- **理由**：`scope.md` §2.2 / §5.1 的总原则（转发不属于终端、各自持有连接）；
  1:1 让"关闭 Session = 终止**这一条**规则的重连循环与传输"（阶段 6 的 0606）不需要引入
  "对 Session 的部分操作"这种半开状态 —— 状态机的每个状态都只描述一条规则。多规则合一
  Session 会把"其中一条失败"变成"整个 Session 是什么状态"的问题。
- **否决的替代路**：一个 Session 收多条规则 —— 表达力更强，但代价是上面那个半开状态。
  若将来真的需要（例如"一组隧道一起启停"），那是一个**新的容器概念**，按修订记录走，不偷改本条。

## 6. 认证与凭据（D7–D8）

### D7 —— 认证顺序：agent → 密钥池 → keyboard-interactive → password；`partial_success` 时续接

- **决定**：按上述顺序尝试，每种只在**前一种失败**后执行；`AuthResult::Failure` 且
  `partial_success = true` 表示"这一种已被接受、还要再来一种"，此时按服务端给的
  `remaining_methods` 继续，**不从头重来**。
- **理由**：agent 是最干净的路径（不重复问、私钥不进我们进程）；keyboard-interactive 是
  2FA / OTP 的唯一入口，必须排在 password 之前（与 OpenSSH 的默认顺序一致）。
- **否决的替代路**：只支持公钥（2FA 主机连不上）；并行尝试多种方法（服务端会按自己的顺序
  计失败次数，容易触发锁定）。

### D8 —— 内存凭据缓存：键 = `(host, port, user, 认证方式)`，值是**保护页**里的口令；绝不落盘

- **决定**：缓存**不存解密后的私钥**，只存"打开密钥 / 登录需要的那句口令"
  （与 `Passphrase` 同族：`memsafe::Secret` 的受保护页）；私钥本体继续只住在密钥池那一页里。
  命中时**不重复问** —— 这是"同主机开三个 Session 只问一次凭据"（plan 0502 的判据）的落点。
- **失效条件**（三条都要实现）：① 库锁定 / 进程退出（清空）；② 该主机的认证被服务端拒绝
  （删掉那一条，否则会拿错口令反复重试）；③ 用户显式"忘记"（一个命令）。
  **没有 TTL** —— 超时后自动忘会让挂着的隧道在重连时突然弹问。
- **理由**：只缓存口令而不是私钥，把"普通堆内存里多一份长期私钥"这个最贵的副本去掉
  （判据表见 ADR-0002 D13，新增用途要按它重验）。
- ⚠️ **够不着的副本（照实记）**：`russh` 需要 `ssh_key::PrivateKey` 才能签名，它解析出来的
  明文密钥在**普通堆内存**里，我们无法把它放进保护页。缓解 = 只在握手窗口内存在、签完即 drop、
  **不进缓存**。ADR-0002 已定案（不可改），它的 §7.5 副本清单因此要在实现时以本 ADR 补一条。

## 7. 原语与转发（D9–D11）

### D9 —— `direct-tcpip` 原语只实现一次，形状 = "一条 `AsyncRead + AsyncWrite` 的流"

- **决定**：`akasha-ssh` 暴露 `direct_tcpip(&Handle, host, port) -> ChannelStream`
  （即 `channel_open_direct_tcpip` + `Channel::into_stream()`），三处复用：
  **跳板**（`connect_stream` 直接吃它当下一跳的底层流）、**`-L`**（在本地监听 socket 与它之间
  搬字节）、**SFTP 的 B 档**（host↔host 的数据面）。
- **理由**：`connect_stream` 的签名只要求 `AsyncRead + AsyncWrite + Unpin + Send`
  （`client/mod.rs:1102`），所以"通道当下一跳的底层流"是上游现成的用法；形状定成流之后，
  另两处只是它的两个消费者，不需要各自再发明一条路径。
- **否决的替代路**：为跳板单独写一条连接路径 —— 三处各写一遍，SFTP 与转发到来时还要再改。

### D10 —— `-R` 是**另一套**机制：`tcpip_forward` + Handler 回调

- **决定**：`-R` 不在 D9 的原语里。请求 `tcpip_forward(addr, port)`（`port = 0` 时用返回值），
  入站连接由 `Handler::server_channel_open_forwarded_tcpip` 回调交给该隧道 Session 的 task；
  规则不活跃时**拒绝**（drop `reply`）；关闭时 `cancel_tcpip_forward` + 取消 task。
- **理由**：`scope.md` §2.2 明确"服务端发起"（`forwarded-tcpip`）与 `direct-tcpip` 不是一条路。
  Handler 是 russh 唯一能把入站 channel 交给我们的位置 —— 因此它必须持有**回到 Session** 的一条
  通道（这也是它必须与 Session 一起被构造的原因）。

### D11 —— known_hosts：读 `~/.ssh/known_hosts` + 库内缓存；**不写**用户的文件；不匹配即拒绝

- **决定**：`check_server_key` **强制实现**（上游默认拒绝一切）；未知主机 → 走"用户确认"
  （UI 未做之前由命令返回一个明确的错误）；已确认的密钥写进**我们的库**；
  `~/.ssh/known_hosts` **只读**（`check_known_hosts_path` 支持它，含 `KeyChanged { line }`）。
  密钥变化 → **拒绝连接并提示**，既不静默接受、也不静默改写。
- **理由**：不动用户的文件 —— 那是 `ssh` 与其它 GUI 工具的共用资产，抢写会互相踩；
  库内缓存跟着可搬迁的数据目录走（`docs/portable.md`）。
- **否决的替代路**：直接调 `learn_known_hosts`（写用户的文件，且便携模式下它不在数据目录里）。

## 8. 隧道状态机与重连（D12–D13）

### D12 —— 五态状态机 + 事件 + 手动重试

- **决定**：`连接中 / 已连接 / 重连中(n) / 失败 / 已停止`（`scope.md` §2.2 的原文五态）；
  状态变化发事件（按 `SessionId` 路由，**UI 未做也先发**）；`失败` 有落点（托盘菜单）；
  提供手动重试（重试 = `失败 / 已停止 → 连接中`，并把尝试次数清零）。
- **理由**：`scope.md` §2.2 的"失败必须可见"与 §5.1 第 2 条（事件按 `SessionId` 路由）。
  状态机本身**不依赖 russh**：它是纯逻辑，放 `akasha-core`/`akasha-ssh` 的可测部分。

### D13 —— 重连参数与"哪些失败不重连"

- **决定**：默认 **3 次**，退避 **1s → 2s → 4s**（`initial = 1s`、`factor = 2`、
  `max_attempts = 3`，全部可配置，默认值写在配置模型里而不是散在代码里）。
- **判据**：

| 失败类别 | 行为 |
|---|---|
| 传输层断开（`Error::Disconnect` / `HUP` / `IO` / 超时） | 进 `重连中(n)`，按退避重试 |
| **认证失败**（`AuthResult::Failure` 且没有可用的剩余方法） | **直接 `失败`，不重试** |
| **主机密钥不匹配**（`Error::KeyChanged`） | **直接 `失败`，不重试**，要求用户显式确认 |
| 配置 / 协议错误（`NoCommonAlgo`、地址不存在） | **直接 `失败`，不重试** |

- **理由**：无限重连会惊扰防火墙（`scope.md` 已否）；把"网络断了"与"口令不对"分开，是唯一能
  同时满足"自动恢复"与"不锁账号"的做法 —— 拿错口令连打三次正是账号锁定的经典成因。
- 记下：退避**不加抖动**（一个 Session 一次只连一条，没有惊群问题）；要加时它是配置项，不是改代码。

## 9. 配置导入（D14）

### D14 —— `~/.ssh/config` 只做受限子集；`Match` / `Include` / 未知关键字**显式报错**

- **决定**：按 `scope.md` §8 风险 4 的已定案结论执行，支持的指令**就是那六条**：
  `Host` / `HostName` / `User` / `Port` / `IdentityFile` / `ProxyJump`。
  解析器**不猜**：遇到 `Match` / `Include` 或任何它看不懂的关键字，宁可**显式报错**，
  也不静默跳过（静默误解析的后果是"连错主机"）。`Host` 的多段匹配按 OpenSSH 的
  "每个参数**首次取到的值生效**"语义实现，不自己发明优先级。
- **理由**：`ssh -G` 那条捷径随 §2.1 的定案作废（它要系统 `ssh`）；完整解析器是**本项目最容易被
  低估的复杂度**，而"受限子集 + 显式报错"把它变成一条能被测的边界。
- 配套：支持哪些指令要能在界面上列出来（UI 阶段的落点，不是后端契约）。
- 展开在 plan 0506；本文只钉"不许静默误解析"与这六条指令的边界。

## 10. 不可逆点（改它们要重写一层）

1. **`Transport` 的 SSH 映射**（D3 / D4）：capability 一旦进了 IPC 类型与前端，
   反向要动前端与生成物。
2. **连接所有权**（D5 / D6）：`Session` ↔ 连接的 1:1 会同时出现在注册表、事件、托盘菜单三处。
3. **原语的形状**（D9）：跳板 / SFTP B 档 / `-L` 都按"流"消费它。
4. **隧道状态机**（D12）：状态名与事件名会进 `src/ipc/bindings.ts` 与前端。

## 11. 代价与对策（照实记下）

| 代价 | 对策 / 现状 |
|---|---|
| 进程里多一个 tokio runtime（与 tauri 自己那个并存） | 只在 `akasha` 建一处、worker 数可配；退出路径先 `shutdown_all` 再 drop runtime（顺序不能反） |
| 每次连接都握手（不复用） | D7 的顺序 + D8 的缓存；有 agent 时天然不重复问 |
| 明文私钥在握手期间短暂存在于普通堆 | D8 的两条缓解；不缓存、不落盘、签完即 drop |
| 上游把 `ssh-key` 钉在预发布 | 升级 `russh` 时一并看；`=0.63.3` 让"升级"成为一个有意的动作 |
| 0.x 的 API 会变 | 用法集中在 `akasha-ssh` 一处（app 只认 `Transport`），升级面被限制在一个 crate 内 |

## 12. 尚未定 —— 留给后续 plan 的问题

- 隧道状态机的**转移表**（每条边的触发条件）与事件名 —— plan 0601。
- `~/.ssh/config` 子集的**解析细节**（`Host` 多段匹配的取值顺序、`ProxyJump` 的语法、
  报错文案）—— plan 0506。指令清单本身已在 D14 定死（六条）。
- 库内 known_hosts 的**表结构**：阶段 4 的四套池里还没有它，加表意味着 `user_version` 的
  一次迁移 —— plan 0503 要动 `akasha-store` 的 schema。
- 未知 host key 的**提问形态**（谁来问、超时多长、答案记在哪）—— plan 0503 定策略接口，
  plan 0504 把它接到前端。
- SFTP 的 B 档消费 D9 原语的具体接法 —— plan 0703。

## 13. 复审条件

- **完整 supervisor 立项时**（与 ADR-0005 §6 同一条）：SSH / 隧道的回收是否仍由
  "app 自持资源 + 进程外看门狗"承担，还是改为"资源归 supervisor，app 只是客户端"。
- **`russh` 出现 1.0（或 `ssh-key` 出正式版）时**：重估 D1 的 `=` 钉法与 feature 组合。
- **出现"一条连接多路复用"的硬需求时**（例如同主机几十条隧道）：重估 D5 ——
  届时引入的是引用计数 + 独立的重连/复用状态机，本 ADR 的"一实体一连接"被取代而非叠加。
- **两个 tokio runtime 变成可观测的负担时**：重估 D2（改为复用宿主 runtime 的 handle）。

## 14. 修订记录

| 日期 | 改了什么 | 为什么 |
|---|---|---|
| 2026-09-12 | 初稿；状态置「实现中」 | plan 0501：动 `akasha-ssh` 之前先把线协议与资源模型定下来 |
| 2026-09-13 | D3 / D4 里"落地时要补"的两条改成**已落地**（`TransportError::Busy`、能力位注释）；新增 **D15**（超时 / 保活 / `nodelay`）；§12 删掉已定的三条；把"`-R` 远端监听消失"这条验收的归属从 0502 改到 **0604** | plan 0502 落地了 D3 / D4 挂给它的两条；`-R` 属于阶段 6，原先把这条验收写在 D4 的 0502 段里是**归属写错**（改它要在 ADR 状态之外留痕，见 `docs/adr/README.md` 的三态） |
| 2026-09-13 | 本 ADR 引用的阶段 5 plan 编号**全部重排**（known_hosts 0505 → **0503**、`direct-tcpip` 0503 → **0505**、config 导入 0504 → **0506**；新增 **0504** = SSH 接进 IPC / 前端）；§12 补上"未知 host key 的提问形态"这条跨 0503 / 0504 的问题 | 执行顺序改由依赖决定：D11 的信任策略（谁问、问什么、答案存哪）是**接口形状**，必须在 0504 把它送到前端之前定下来，否则 0504 只能自造一套临时信任；`direct-tcpip`（D9）的三处消费者都要先有一条从 app 打得开的会话才验得了 |
| 2026-09-13 | **D11 按原文落地**（plan 0503）：三态判定（库里有且一致 → 连 / 有但对不上 → 拒 / 未知 → 问）、`~/.ssh/known_hosts` **只读**、确认过的进**我们自己的库**；本 ADR 的**决定一字未改**，只把关联指针改到归档路径（§12 里"提问形态"那条仍然成立：策略接口已定，接前端在 plan 0504） | D11 当初就把策略写死了，实现没有推翻它，所以本文没有必要改内容。落地时带出一个新事实（库格式 `user_version` 1 → 2，本仓库第一次迁移），它属于 ADR-0002 的辖区 —— 已记在归档的 plan 0503 与 `docs/STATUS.md` |
