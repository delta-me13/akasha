# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-15

## 摘要

**阶段 2「端到端最小终端」5/5 完成**；**阶段 3「托盘与应用生命周期」6/6 完成**；
**阶段 4「存储与凭据池」9/9 完成**；**阶段 5「SSH 栈」6/6 完成**；
**阶段 6「SSH 端口转发」6/6 完成**：隧道实体 + 状态机（0601）、三种转发（0602 / 0603 / 0604
—— `-L` 与 `-D` 共用"本机监听 + 每条入站连接一条 `direct_tcpip` 通道"，`-R` 是**另一套机制**：
端口开在服务端、通道由服务端发起）、断线重连（0605）、关闭 `Session`（0606）。
**阶段 7「SFTP」4/4 完成**：双栏骨架 + 两侧独立选主机（0701）、local ↔ host 双向传输
（0702）、host ↔ host 两档（0703）、并发 in-flight（0704）。

阶段 5、6、7 的形状分别固化在 **ADR-0003 / ADR-0006** 里，两份**都已定案**
（落地它们的 plan 全部归档；改动只能由新的 ADR 取代）：
- **ADR-0003（SSH 栈与资源模型）**：`russh` 的版本与 feature、运行时归属、`Transport` 在 SSH 上的
  映射、`direct-tcpip` 原语的形状、隧道状态机与重连判据全部**带出处**确定。
- **ADR-0006（SFTP 栈与传输引擎）**：SFTP 会话承载在**一条流**上（与 D9 的原语同形）、
  一个 `Session` 拥有**两侧各自一条独立连接**、传输引擎只认"两个端点"而**落盘不变量归目标端点**
  （本机 `std::fs` 的 `rename`，远端 SFTP 的 `rename`）；两栏之一可以是**本机文件系统**
  （`SftpOrigin`），两栏都是主机时**档位在连接那一刻定**（`through` / `throughFailure` / `via`
  三个字段是它的读数口）。⚠️ ADR-0003 D9 与 §12 里那句"SFTP 的 B 档仍未接"**已过期** ——
  那份 ADR 不可改，留痕在 ADR-0006 §6；查 B 档状态以本文件为准。
- **0704 的四样**：上限落成引擎的一个类型（`akasha_ssh::InFlight` —— 一个 SFTP 会话持一个、
  有 `limit` / `live` / `peak` 三个读数，**等空位可取消**）、临时名的占用从"先查存在、再创建"
  改成**一次**原子占用（本机 `create_new`、远端 `CREATE|EXCLUDE`）。

**阶段 8 的两条都完成（plan 0801 / 0802）**：`akasha-serial` 落地 —— `Transport` 的串口实现 +
`ports()` 枚举 + 参数校验，零 Tauri 依赖、`libudev` 只在 Linux；判据与读数口见下文
「阶段 8 的形状」。串口接入 app **不在阶段 8 内**，已立为 **阶段 11** ——
它的**三条全部已完成**（plan 1101 / 1102 / 1103）：一条串口 `Session` 能从界面打开、字节双向流动、
关标签页即回收；界面上三条输入并列（枚举到的端口 / 手输路径 / 池行取值），取值越界时
当场看到字段与取值；设备被拔掉时那条会话自己结束、标签页随之关闭，界面上留下那句
「串口设备已断开：<设备路径>（Broken pipe）」。形状见下文「阶段 11 的形状」。

阶段 4 的八项（每项一句）：**SQLCipher 加密库可打开**（0401）、**口令只从一条路径进入且可真正验证**
（0402 —— 拆分"打开"与"新建"；此前在文件不存在的路径上**任何口令都能打开**）、
**口令在内存中同样受保护**（0406）、**四类池可增删改查**（0403）、**库中数据可导出也可还原**
（0404）、**解锁与锁定构成完整生命周期**（0407 —— 真实 app 上 `VmLck` **0 → 176～192 → 0 kB**）、
**目录迁移后数据仍在且可用**（0405）、**便携目录不可写时拒绝启动**（退出码 2）。
**阶段 5 的四块**依次是**连接 + 认证**（0502）、**主机密钥的信任策略 + 本仓库首次库格式迁移**
（0503）、**接入 IPC 与前端**（0504）、**`direct-tcpip` 原语 + 跳板（ProxyJump）**（0505），
最后是 `~/.ssh/config` 导入（0506）—— 逐条落点见下表。

### 阶段 5 完成情况（按依赖顺序）

| plan | 完成内容 | 判据落点 |
|---|---|---|
| 0501 | ADR-0003（D1–D16，带出处的调研 + 资源模型） | `docs/adr/0003`，状态「已定案」（2026-09-15，随阶段 6 收尾） |
| 0502 | `akasha-ssh` 连接 + 认证（D7 的顺序、D8 的内存凭据缓存） | **crate 层**：进程内 SSH 服务端，三个连接共享一次凭据询问 |
| 0503 | known_hosts 三态判定（D11）+ 库格式 v2 与首次迁移 | **crate 层**：策略 + 库契约；迁移用真实 v1 库 |
| 0504 | SSH 接入 IPC / 前端：带目标的会话命令、提问往返、提示界面 | **真实 app**（`ssh_session` E2E + 测试进程内的服务端） |
| **0505** | **`direct-tcpip` 原语**（D9 的形状 = 一条流）+ **跳板链**（池中的 `jump_id` 生效） | **crate 层**（两个真实服务端 + 负控）+ **真实 app**（`ssh_jump` E2E） |
| **0506** | **`~/.ssh/config` 受限子集导入**（三档边界：六条导入 / 局部指令警告 / 其余整份报错）+ 池的**第一条写路径** | **crate 层**（29 条解析 + 5 条落库，纯函数）+ **真实 app**（`ssh_config_import` E2E：导入的条目经跳板真实连通） |

### 阶段 6 的形状（0601 的实体 + 0602 的 `-L` + 0603 的 `-D` + 0604 的 `-R` + 0605 的重连 + 0606 的关闭）

**隧道是一个独立的 `Session`**（ADR-0003 D5 / D6，`scope.md` §2.2）：一条转发规则一个
`Session`，它自持有一条连接。三种转发机制**全部落地** —— `-L` / `-D`（plan 0602 / 0603，
共用"本机监听 + 每条入站连接一条 `direct_tcpip` 通道"）与 `-R`（plan 0604，ADR-0003 **D10**：
请服务端监听 + 服务端发起的 `forwarded-tcpip` 通道，**不复用那个原语**）。

| 事 | 落在哪 |
|---|---|
| 五态与转移表 | `akasha-core::TunnelState`（纯逻辑、零 Tauri；同态与"重连直达 `已连接`"都是非法边） |
| 实体表与注册表 | `src-tauri/src/session.rs` 的 `Sessions` —— **同一张注册表、同一把锁**（D6），不另立第二份 |
| 连接 | `SshConnection`（D9 的类型：已认证、**没有通道**）+ 同步门面 `connect_via`（它会持有自己的跳板链） |
| **转发（`-L` / `-D`）** | `akasha-ssh::relay`：`LocalListener`（**先绑**）→ `LocalForward`（接受循环 + 每条入站连接一条 `direct_tcpip` 通道 + `copy_bidirectional`）。两条路的差别是 `Ingress`：`Fixed`（规则里的目标）/ `Socks5`（目标由客户端说，协议在 `akasha-ssh::socks5`） |
| **转发（`-R`）** | `akasha-ssh::remote`：`SshConnection::remote_listen`（发 `tcpip_forward`，`port = 0` 时用服务端回报的端口）→ `RemoteForward`（路由表 `Inbound` + 每条入站通道**先连本机目标再接受** + `copy_bidirectional`）。停止 = 撤路由 → 收在途连接 → `cancel_tcpip_forward` → 断开连接 |
| **重连（三个方向共用）** | app 侧 `src-tauri/src/tunnel.rs` 的**看护任务**：首次连上之后每条隧道起一条任务 —— 等那次转发结束（`akasha-ssh::ending` 的 `ForwardEnd`：被停止 / 连接没了）→ **只有实体仍在 `已连接`** 才重连 → 按 `akasha_core::Reconnect::delay` 退避 → `重连中(n)` → `连接中` → 再走一遍"准备 + 连接 + 起转发"。停止 / 重试 / 退出经**实体自己持有的一对 `watch`**（`TunnelStop` / `TunnelStopSignal`，plan 0606 换掉了 0605 的 `oneshot`）让它退出 |
| **`-D` 的协议** | `akasha-ssh::socks5`（RFC 1928）：**只做无认证的 `CONNECT`** —— `BIND` 回 `0x07`、不认的 `ATYP` 回 `0x08`、没有 `0x00` 方法回 `05 FF`、版本不对什么都不回；成功 `REP` 在通道开出来**之后**才回，`BND.ADDR` 是占位 `0.0.0.0:0`（SSH 的通道确认里没有对端的绑定地址） |
| 命令 | `tunnel_open(forwardId)` · `tunnel_retry(handle)` · `tunnel_stop(handle)` · 只读 `vault_forwards()` |
| 事件 | `tunnel_state`（载荷 `{handle, state, attempt}`）—— 按 `SessionId` 路由 |
| **关闭（0606）** | `tunnel_stop`：`已停止`（发事件）→ 停下手上的动作（`TunnelStop` 那对 `watch`：在途的尝试与看护循环各订一份）→ `Tunnel::reclaim()`（停转发 + 断连接）→ 注销注册。`shutdown_all` 走**同一份**回收 |
| probe | `tunnels` → `[{handle, ruleId, name, state, attempt, bind}]`（与托盘菜单同源；`bind` = 实际监听地址）· `residue` → `{sshConnections, watchTasks}`（判据"两个计数都归零"的读数口） |
| 失败分档 | `TunnelError`：`locked` / `noSuchForward` / `noSuchHost` / `notATunnel` / **`bind`（本机端口没拿到）** / **`remoteBind`（服务端那个端口没拿到：被它占着 / 它不允许远端转发）** / **`notLoopback`（SOCKS5 绑了非回环地址）** / `failed {kind,message}` / `transition` / `internal`。⚠️ 从 plan 0604 起**没有 `unsupported` 这一档**：三个方向都支持了，留着一个永远出不来的错误档就是在文档里留一句假话 |

⚠️ **`重连中(n)` 的前提是"曾经连上过"**：`3 次 + 1s/2s/4s` 的预算只花在**已连接之后掉线**
这条路上；**首次连接失败不自动重试** —— 那次失败是同步报给用户的（命令返回里带原因，
下一步动作在用户手上），而 D13 的表说的是"传输层**断开**"。
⚠️ **"断开"怎么认**：转发任务按 **500 ms** 看一眼 `Handle::is_closed()`（上游 0.x 只给了这个
同步问法，见问题 #141）—— 连接真的结束时**秒级**发现，而"半死"（TCP 没断、对端不回话）要等
保活耗尽（30 s × 3 ≈ 90 s = `keepalive_interval × keepalive_max`）。
⚠️ **重连 = 重新走一遍"准备 + 连接 + 起转发"**：连接是那条转发的命根子，所以 `-R` 会**重新发
一次 `tcpip_forward`**（远端监听是服务端那条连接的资源，断连即撤销）—— 不重新请求就是"重连成功
了、端口却不在听"。规则里写 `port = 0` 的 `-R` 重连后**可能换一个端口**（服务端重新挑）。
⚠️ **失败分档决定要不要重试**（`TunnelError::retryable`，D13 的表）：传输层（`connect` / `jump`）
与**端口没拿到**（本机 / 服务端）会重试；认证、主机密钥、配置与内部状态**不重试** —— 以错误的
口令连续尝试三次正是账号锁定的经典成因。
⚠️ **状态变化会重推托盘菜单**（`set_tunnel_state` → `Sessions::notify_changed`，问题 #135）：
菜单是一份快照，不重推它就永远停在隧道刚登记时那一行 —— 而 `scope.md` §5.2 把"失败必须可见"
的落点放在托盘上。
⚠️ **看护任务是"停止 / 重试 / 退出"三处共用的一个中止入口**：`tunnel_stop` / `tunnel_retry` /
`shutdown_all` 都让它退出，而它在**每一个 `await`** 上回应它（停止因此不必等一次握手的 10 秒）。
⚠️ **`tunnel_retry` 只认终态，且这条检查排在绑定之前**（D12）：`重连中` 也在等下一次尝试，但那是
看护任务的事。先问状态机（纯判断）再动资源，与问题 #131 同一条纪律。
⚠️ **停止 = 停止 + 注销**：`tunnel_stop` 先把状态推到 `已停止`（发事件），再摘掉那个 `Session`
并让转发收尾 —— 收尾（停监听 → 收在途连接 → 礼貌断开连接）**在 runtime 上做**，命令发完信号即返回。
⚠️ **"关闭转发 `Session`"就是 `tunnel_stop`**（plan 0606）：`close_session` 是终端那条
（kill + wait），对隧道句柄答 `NotFound` —— 转发是**仅渲染**的视图，它的停止必须是它自己的显式动作
（`scope.md` §5.6）。这条边界看起来像缺口（隧道在 `Sessions` 里同样是 `Session`），所以写下来。
⚠️ **判据里的两个计数**（plan 0606）：`residue` 报的是 `SshConnection` 对象与看护任务本身，
**不是**注册表里的实体 —— 关闭命令自己就会把实体摘掉，"表里没了"只是那条命令的效果。
⚠️ **"立刻断连"要靠取消信号送到握手那一层**（plan 0606）：建立连接发生在**阻塞线程**上
（app 侧的 `spawn_sync`），而扔掉 `await` 那一侧取消不了它 —— 那条 socket 会一直开到
`connect_timeout`（D15 的 10 s）。所以 `SshConnection::connect_via_until(…, cancel)` 让
**那次调用自己**结束；实测停止到对端读到 EOF **7.5 ms**。
⚠️ **停止信号是"实体自己持有的一对 `watch`"**（plan 0606，替掉 plan 0605 的 `oneshot`）：
`oneshot` 的发送端被替换（看护任务挂上自己那一份）时接收端会立刻醒，于是 `select!` 的两个分支
同时就绪，而 `tokio::select!` 这时**随机挑一个** —— 一次**成功**的连接因此有约一半的机会被报成
"已停止"。实体持有发送端之后，只有"真的被要求停止"或"实体没了"才会唤醒等待方。
⚠️ **先绑定、后连接**：端口被占用是本类功能最常见的一类失败，它必须在"用户答凭据"**之前**失败 ——
绑定失败 = **没登记成**（`Err`），与"登记了但连不上"（`Ok(TunnelAttempt{failure})`，仍在册可重试）分开。
⚠️ **`target_host` / `target_port` 从 plan 0602 起参与**：`-L` 里它们被原样送进
`direct_tcpip`，由**对端**解析（在本地解析等于绕开跳板机）；`-R` 里它们说的是**本机**服务，
由**本机**解析 —— 两侧的角色正好相反。`-D` 没有目标（库的 `CHECK` 拦着），
客户端在握手报文里给的域名同样**不做本机解析**。
⚠️ **`-R` 的绑定地址不做回环限制**（与 `-D` 相反）：那个端口开在**服务端**，
能不能开在非回环地址上是**它的**策略（`sshd` 的 `GatewayPorts` 默认只允许回环）。规则里写什么
就请求什么，不替对端做决定。
⚠️ **`-R` 的"绑定"发生在连接之后**：那个端口在服务端，要先有连接才能发请求 ——
所以"先绑定、后连接"那条只对 `-L` / `-D` 成立。端口拿不到时它**仍然登记着**（`失败` 态、可重试），
与"本机端口没拿到"（`Err`，没登记成）不同。
⚠️ **`-R` 的入站通道按端口认**：服务端回报的 `connected_address` 是"它认为在听的地址"
（与它的配置有关），按地址字符串认会在真实服务端上静默失配；没登记过的端口一律**拒绝**
（drop `reply`）。
⚠️ **`-D` 只允许绑回环地址**（plan 0603 的安全项）：SOCKS5 这一侧无认证（`0x00` 是唯一接受的
方法），绑 `0.0.0.0` 等于把"经这台跳板机访问远端网络"的能力交给同网段的所有人。检查在**绑定
之前**（`SshError::NotLoopback`），因此那样一条监听根本建不出来 —— **本版本没有"我确定要开放"
的选择项**（取得明确同意的那一轮询问是 D16 的凭据往返，它不覆盖这件事）。
⚠️ **SOCKS5 客户端只收到一个 `REP` 字节**：通道开不出来时它必须**分类**（`0x02` 对端不允许转发 /
`0x05` 目标拒绝连接 / `0x07` 命令不支持），类别取自上游结构化的 `ChannelOpenFailure`
（`SshError::Forward` 的 `class`），不从错误字符串里猜。

### 阶段 7 的形状（plan 0701 的实体 + 0702 的引擎 + 0703 的两档 + 0704 的并发上限）

**形状先固化在 [ADR-0006](./adr/0006-sftp-stack-and-transfer-engine.md)（状态「实现中」）**：
SFTP 协议实现取 `russh-sftp = "=3.0.0"`（会话定义在**一条 `AsyncRead + AsyncWrite` 流**上，
与 D9 的 `SshStream` 对齐，且它**不依赖 `russh`**）；传输引擎只认"两个端点"，
**"临时名 + 原子重命名"是目标端点的职责**（0702 已落地）。

| 事 | 落在哪 |
|---|---|
| 实体与两侧 | `src-tauri/src/sftp.rs`：`Sftp { id, sides: [Side; 2], transfers, next_transfer }`；每侧 = `{来源, 名字, 状态, 失败原因, 当前目录, 连接 + 会话句柄}` |
| 实体表与注册表 | `src-tauri/src/session.rs` 的 `Sessions` —— **第三张表 `sftps`**，与 `live` / `tunnels` 共用同一张注册表与同一把锁（ADR-0003 D6 的纪律） |
| 连接 | `crate::ssh::connect_connection` —— 与终端、隧道**同一条**建链路径（含跳板链与主机密钥校验）；差别只在连上之后开的是 `sftp` 子系统。**本机那一档不建连接**（没有握手、没有认证，也就没有会失败的地方） |
| 会话 | `akasha-ssh/src/sftp.rs` 的 `SftpClient`：`SshConnection::sftp()` 开子系统会话；`list(path)` 先 `canonicalize` 再 `read_dir`，条目**按名字排序**；plan 0702 起它同时是 [`Endpoint`]（`open_read` / `begin_write`） |
| 传输引擎 | `akasha-ssh/src/transfer.rs`：`Endpoint` + `PendingWrite` + `Cancel` + `Progress` + `transfer()`。**两个端点**是唯一的输入，临时名与原子重命名在**目标端点**里 —— 于是本机 ↔ 主机、以及 host ↔ host 的两档共用同一个引擎 |
| 并发上限 | `akasha-ssh/src/in_flight.rs`：`InFlight { limit, permits, live, peak }` + 有界入口 `transfer`（`select!` 带 `biased` 地等空位，取消那一支先看）。默认值在 `akasha_core::Transfer::in_flight`（8） |
| 两档 | `src-tauri/src/ssh.rs` 的 `connect_connection_via`：`hops` = A 那一行自己的跳板链 ++ `[A]` ++ B 那一行自己的跳板链，终点是 B —— 交给**已有的** `connect_via_until`（plan 0505 的"只实现一次"）。`src-tauri/src/sftp.rs` 的 `sftp_connect` 在连 B 之前先看另一栏：它已连上一台**不同的**主机就试这条链（B 档），失败则本机直连（A 档） |
| 本机端点 | `akasha-ssh/src/local.rs`：`LocalEndpoint`（**无字段**：没有连接要持有，路径每条命令带进来）。`.name.part` 同目录写完再 `rename`（POSIX 与 Windows 都原子） |
| 传输的记账 | 实体的 `transfers`（新的在前）+ 每条一个 `Arc<Tracked>`：进度是 `AtomicU64`、状态与失败原因是短锁 —— **搬字节的任务不去抢会话表那把锁** |
| 命令 | `sftp_open` · `sftp_connect(handle, side, origin)` · `sftp_list` · **`sftp_transfer` / `sftp_transfer_cancel` / `sftp_transfers`** · `sftp_sides` · `sftp_sessions` · `sftp_close` |
| probe | `sftp` → `[{handle, sides:[{side, origin, name, state, failure, path, through, throughFailure}], transfers:[{id, from, fromPath, to, toPath, state, done, total, failure, via}], inFlight:{limit, live, peak}}]`（两侧永远两项、`left` 在前；传输新的在前） |
| 失败分档 | `akasha-ssh` 新增 `SshError::Sftp`（"会话建不起来 / 用不了"，用户要看的是**对端有没有开 SFTP**）与 **`SshError::File { path, reason }`**（plan 0702：**端点上的一个文件操作**失败 —— 用户要去看的是**那个路径**，用会话那一档报一次重命名失败会把排查方向整个带偏）；app 侧 `SftpError`：`locked` / `noSuchHost` / `notAnSftp` / **`notConnected`（这一侧还没连）** / **`noSuchTransfer`** / `failed {kind,message}` / `internal`（plan 0703 删掉了 `unsupported`：host ↔ host 不再是"还没做"） |

⚠️ **"两侧独立"有可断言的形式**：两侧各持各的连接，失败只落在**那一侧**
（`state = failed` + `failure` 里那句话），另一侧照样可用；换主机时**上一次那条连接会被交出来**
在锁外回收（两条连接同时挂着就是漏掉一条没有主人的连接）。
⚠️ **`side` 是唯一进入契约的呈现概念**（ADR-0006 D7）：后端不给它别的含义 ——
资源归那个 `Session`，不归某一侧。
⚠️ **关面板不等于结束会话**（`scope.md` §5.6 的"仅渲染的视图"）：所以有一条 `sftp_sessions`
让面板重新打开时**接回**已有会话 —— 否则"关前端不影响后端执行"就变成了"关前端等于失控"。
⚠️ **上游的 `request_subsystem` 只发送、不等回复**（`russh` 的 `channels/mod.rs` 走 `send_msg`）：
对端没开 SFTP 时它不回 `VERSION`，于是"它拒绝了"表现为**初始化会话等满期限**。
所以错误消息里说明"多半是对端没开 SFTP"，且期限可调（`sftp_with_timeout`）——
否则一条"这里会失败"的负例只能靠等满默认的 10 秒来证明。
⚠️ **落盘不变量是引擎的出口条件**（plan 0702，`scope.md` §4.2）：成功的定义是**目标端点已经把
临时名换成了最终名**，其余每一条退出路径（取消 / 读失败 / 写失败 / 重命名失败）都先把临时名
删掉再返回 —— "用户看到最终名就等于成功"因此成立。
⚠️ **取消只有一条路径**（ADR-0006 D4）：用户点「取消」与关闭 `Session` 推的是**同一个**
`Cancel`，清理由目标端点的 `abort` 完成。于是 `sftp_close` 的顺序不能换 ——
**先中止、等清理落地，再断连接**（临时文件是用那条连接删的）。
⚠️ **临时名的取名规则在引擎那一层，抢占由端点做**（`.name.part`，被占时退到 `.name.N.part`，
最多试 16 个）。plan 0704 起抢占是**一次**调用（本机 `create_new` = `O_EXCL`，远端
`WRITE|CREATE|EXCLUDE`）—— "先查存在、再创建"那道缝正是两个同名文件并发传输会踩进去、
双双写同一个临时文件的地方。
⚠️ **远端分不清"名字被占"与别的通用失败**：SFTP v3 的状态码只有 8 个取值，没有"已存在"这一档
（那是 v4 才补的）。所以只有通用的 `Failure` 换下一个候选，权限 / 超时 / 断连当场报出去 ——
后者换 16 个候选只是把同一次失败问 16 遍。
⚠️ **上限约束的是整个会话**（ADR-0006 D6）：一个 SFTP 会话持一个 `InFlight`，"每一条传输一条
命令"的形状因此不会各自为政。排队中的传输在传输记录里与"正在搬"长得一样（状态只有"还没结束"
这一档），分辨它们靠 `live` 比"还没结束的条数"少。
⚠️ **上限的默认值不按"饱和点"取**：分档实测里 16 仍在明显更快，取 8 是占用与吞吐的折中
（上限同时是对端打开句柄数与本机缓冲数的上界）。分档曲线里**上限 2 与串行一样慢**那一段
还没有定位 —— 见待验证与 plan 0704 的实施记录。
⚠️ **档位在"连接那一刻"定，定的是目标那一栏的端点**（ADR-0006 D5）：那条链建链尝试的
**任何**失败都算"B 不可用" —— 不分类（对"要不要回退"这个问题它们没有区别），但**必须留下证据**：
`through`（经哪台直通）/ `throughFailure`（回退原因）/ 传输的 `via`。于是"这次传输走的是哪一档"
是**端点怎么来的**的后果，`sftp_transfer` 与传输引擎都不做档位判断（引擎一行都没改）。
⚠️ **"优先 B"的收益是可直达性，不是字节数**：两档下文件都要过一次本机（读源一遍、写目标一遍），
`scope.md` §4.1 原来的"带宽减半"按字面不成立，已按这个口径更正。
⚠️ **隧道那条链是独立的一条**（本机 → A → B），**不复用另一栏的连接对象**：另一栏换主机 /
断开都不会带走这一栏 —— ADR-0006 D3 的"不复用同一条连接"因此仍然成立。

### 阶段 8 的形状（plan 0801 的 crate 与 feature + plan 0802 的枚举与参数）

| 事 | 落在哪 |
|---|---|
| crate | `src-tauri/crates/akasha-serial`：零 Tauri 依赖，对外只认 `akasha_pty::Transport`（与 `akasha-ssh` 同一条边界） |
| 载体 | `transport.rs` 的 `SerialTransport::open(&SerialSettings)` + `Transport` 实现。**能力位是 `Capabilities::NONE`**（没有窗口尺寸、没有结局、没有本地进程）：`resize` 答 `Unsupported`、`exited()` 恒为 `Ok(None)`、`session_leader()` 是 `None`；`shutdown` 幂等（立停止标志 + 尽力 flush） |
| 参数 | `settings.rs` 的 `SerialSettings`：六个字段（路径 + 波特率 / 数据位 / 停止位 / 校验 / 流控），取值域与 serial 池的 `CHECK` 对齐（数据位 5..=8、停止位 1..=2），`serialport` 的类型只出现在 crate 内部；`DataBits::try_from` / `StopBits::try_from` 越界报出字段与取值，`validate()` 查空路径与 `baud == 0` |
| 枚举（plan 0802） | `enumerate.rs` 的 `ports()`：上游四种类别映射成 `PortKind`（USB 带 vid / pid / 序列号 / 厂商 / 产品名五项），输出**按路径排序、同一路径只留信息量最大的一档**。**空表是正常结果**；⚠️ **列出来的端口不保证能打开**（问题 #150） |
| 错误分域（plan 0802） | `error.rs` 把三组失败分开：`Enumerate`（列不出来）/ `Settings` · `NoPath`（参数不合法，报**字段与取值**）/ `Open` · `Handle`（这一个设备打不开，报**路径**） |
| feature | `libudev = ["serialport/libudev"]`、`default = ["libudev"]`，且 `serialport` 以 `default-features = false` 引入 —— 链不链 libudev 是 manifest 上看得见的事。**平台差异由上游的 target 段承担**：那个 optional 依赖只在 `cfg(all(target_os = "linux", not(target_env = "musl")))` 下存在，Windows / macOS 上打开这个 feature 什么都不编译 |
| 判据的读数口 | `just libudev-check`（按目标核对依赖图，两条负例验过）与 `just serial-check`（两条枚举实现各执行一遍，负例验过）—— 数字见「已验证为通过」与「各轮」 |
| libudev 缺失时的降级 | Linux 上关闭该 feature（`default-features = false`）**仍能枚举** —— 上游另有一支 sysfs 实现；运行期拿不到 `libudev::Context` 时它返回**空表**而不是错误。发行版缺 libudev 的开发包**不是硬失败** |

⚠️ **手动指定路径不依赖枚举**：`SerialTransport::open` 收的就是一个设备路径
（`/dev/ttyUSB0` / `COM3`），"端口列表为空"或"libudev 整个不在"都不影响打开一个已知端口。

⚠️ **两种枚举实现给出不同的结果**：默认配置（libudev）列出 32 条 `/dev/ttyS*`，
`--no-default-features`（sysfs）列出 0 条（它要求 `/dev/<名字>` 存在）—— 读数与教训见问题 #150。

⚠️ **Windows / macOS 的原生编译证据还差一层**（问题 #149）：`akasha-pty` 里的
`rustix::process` 挡住那个目标，本地证据是依赖图与一次一次性探针。

⚠️ **阶段 8 到 crate 层为止**：串口接入 app（命令 / 界面 / 设备消失时的表现）是**阶段 11**，
不是阶段 8 的第五条。

### 阶段 11 的形状（plan 1101 的接线 + plan 1102 的三条输入 + plan 1103 的可读原因）

**串口与 local / SSH 是同一种东西**：一条 `Transport` 装进 `Sessions` 的 `live` 表 ——
没有第四张表、没有新的会话模型、没有新的收尾路径。变的只有"载体从哪来"。

| 事 | 落在哪 |
|---|---|
| app 侧接线 | `src-tauri/src/serial.rs`：`SerialEntry` + `vault_serials`（列池里的行）、`SerialPort` / `SerialPortKind` + `serial_ports`（列本机端口，plan 1102）、`SerialParams`（六个字段的 IPC 形状）、`open_serial_session`（**同步**）、`SerialIpcError`（`settings{field,value}` / `open{path,message}` / `enumerate{message}` / `internal`，**没有 `locked`**） |
| 取值域两侧的映射 | `SerialParity` / `SerialFlow` 两个 IPC 枚举 → `akasha_store::pools::serial` 与 `akasha_serial` **各一个穷尽 `match`**；`data_bits` / `stop_bits` 保持库里的原始数值，越界由 crate 的 `TryFrom` 报字段与取值 |
| 命令 | `vault_serials` · `serial_ports`（plan 1102，**同步**）· `open_serial_session(channel, params)` |
| 载体从哪来 | **参数显式，不是池行 id**：打开一个设备不碰库（没有秘密），所以这条路径**不需要解锁**；`locked` 只挡"列出池里有哪些"。于是 plan 1102 的"手输路径 + 参数可编辑"复用**同一条**命令 |
| 三条并列的输入 | 枚举到的端口 / 手输的设备路径 / 池里的一行 —— **谁都不挡谁**：枚举那一块读不出来（`enumerate`）时只有它自己显示那句，手输照常；库锁着时只有池那一块说"先解锁"。点一条枚举结果**只是把路径填进表单**（不试开、不标可用） |
| 表单与报错 | `src/serial/SerialPicker.tsx`：六个字段（数值三栏是**文本框** —— 越界取值要能从界面产生）、`data-serial-field` 与 `[data-serial-open]`。⚠️ **范围那一档仍归后端**（`settings{field,value}`）：前端只拦"寄不出去"（空串 / 非数字 / 超出 `u32`·`u8` 的宽度），那一条是 JSON 边界的必答项，不是第二份取值域 |
| 前端 | `SessionTarget` 的 `serial` 变体、`TabKind` 加 `"serial"` 并进 `CLOSABLE`、`src/ipc/serials.ts`（两个只读入口：池 + 端口） |
| 判据的读数口 | `app_state { probe: "sessions" }`（串口**没有本地进程**，"关闭零残留"只能看注册表 —— 同 ADR-0003 D4）；E2E `tests/serial_session.rs` / `serial_ports_ui.rs` / `serial_device_gone.rs` 各自造一对 PTY（脚手架 `tests/support/` 的 `FakeSerialDevice`），把**从端的路径**当设备 |
| 结束原因从哪来（plan 1103） | `akasha-pty` 的 `Transport::stream_error`（默认 `None`）：`shutdown()` 报"结局是什么"（退出码 / 信号），这一条报"输出流为什么不是正常结束"。串口读端遇到**真实**读错误时把它渲染成 `SerialError::DeviceGone`（带设备路径）记在同读端共享的那一格里；会话层在收尾**之后**读它，算"结局优先、原因次之"进 `SessionEnded.status`（`PTY` / `SSH` 照旧报结局） |
| 那句话怎么到界面 | `session_ended` 的 `status` 逐层交到 `App.tsx`（此前整个被丢掉），由它先写进应用级通知行 `[data-session-notice]` 再关闭标签页 —— 说出这句话的那个面随即被卸载，所以那句话只能留在壳层。顺带补上一条：本地终端的"退出码 N"从此也看得见 |

⚠️ **`resize` 的 `Unsupported` 不是失败**（plan 1101）：`Sessions::resize` 现在**先看能力位**
（`Transport::resize` 的契约本来就要求调用方这么做）。不改的话串口标签页一打开就是红的 ——
前端每次 `fit()` 都发一次 `resize_session`，而串口的能力位是 `Capabilities::NONE`。

⚠️ **`serialport` 默认独占打开**（`TIOCEXCL` + 独占 `flock`，上游 `posix/tty.rs`）：
同一台设备被第二个会话打开会当场得到 `EBUSY`。这对真实设备是对的（两个进程抢一个串口本来
就不该成功），**不要**为了绕开它去关独占。⚠️ 它与 **React StrictMode 的两次挂载**撞上过一次：
被丢弃的那次挂载真的把设备打开了，于是活下来的那次收到 `Device or resource busy` ——
处置是把"推迟一个微任务再开"从 SSH 扩到串口（`src/terminal/attach.ts`），
**只有本地终端保持同步发出去**（它是这条路上唯一可以重复且无痕的东西）。

⚠️ **设备消失是 `Err`，不是 `Ok(0)`**（plan 1103 的前置检查）：从端那一路的 `read` 立刻返回
`BrokenPipe`（上游 `serialport` 的 `posix/poll.rs::wait_fd` 见到 `POLLHUP` / `POLLNVAL` 报
`BrokenPipe`，否则报 `Other(EIO)`）。这一条是整条路的前提 —— `SerialReader` 把 `Ok(0)` 当
"设备安静"继续等，若上游哪天改用 `Ok(0)` 表示消失，这条会话会**永远不结束**（而且忙等）。
流结束之后与 local / SSH 走同一条：`retire` → `session_ended` → 标签页随之关闭。
⚠️ **"我们自己收尾"不留原因**：收尾先立起停止标志，读端此后给的是 `Ok(0)`（单测两面都钉住）。

⚠️ **打开失败的那句话由标签页说，不由面板说**（plan 1102）：会话是那个面自己发起的
（`App.tsx` 只负责开一个面，见 `attachTerminal`），所以参数不合法（数据位 9）时**面已经开出来
了**、它当场是出错态，报错行里是 `data_bits = 9` —— 与 SSH 连不上同一条路。面板自己显示的
只有"读列表失败"与"参数填不出来"这两档（后者**不开面**）。
⚠️ **枚举结果不代表可用**（问题 #150，plan 1102 的呈现纪律）：udev 报 devnode 时不检查它在
`/dev` 下是否存在（本机列出 32 条 `/dev/ttyS*`，`/dev` 下一条都没有），所以那一块不标
"可用 / 不可用"、也不禁用别的输入 —— 能不能开只有点「打开」之后才知道。

**关闭窗口的语义由配置与托盘可用性共同决定**（0302 + 0303）：

### 阶段 5 之前那些跨阶段的结论（还在生效）
| `close_behavior` | 托盘是否可用 | 点击关闭按钮之后 |
|---|---|---|
| `tray`（默认） | 是 | **窗口隐藏、进程保留** —— 会话与终端缓冲原样存活 |
| `tray` | **否** | 退出（**降级**：窗口一旦隐藏便无法唤回，问题 #60） |
| `exit` | 任意 | 退出（走 0204 的收尾路径，零残留） |

配置文件 = **数据目录**里的 `config.json`（`docs/portable.md` §3.1）：bin 同目录存在
`akasha-data/` 时使用它（便携模式），否则退回 OS 数据目录（Linux 上 =
`~/.local/share/fans.cyrene.akasha-terminal/`）。**只读、不自动创建**；读不到或值不认识 →
取默认值 + 一条日志。⚠️ **仅在启动时读取** —— 修改后需重启 app。

**单实例（0304）**：第二个实例会唤回已有窗口（还原 → 显示 → 置前）后自身退出；
窗口处于隐藏状态时同样如此（仅 `set_focus()` 无法唤回隐藏窗口）。

因此回收时机有**六类触发**（粒度从一个会话到整个进程）：

| 触发 | 谁被回收 |
|---|---|
| 关一个终端标签页 / 会话自己结束 | **只有那一个会话**（SSH 那条路 = **整条跳板链一起断**） |
| **关窗口（默认 = 收托盘）** | **不回收** —— 进程、会话、终端缓冲全留着（0302） |
| 关窗口（`close_behavior = exit`） | 全部（`RunEvent::Exit` → `Sessions::shutdown_all()`） |
| **从托盘菜单退出** | 全部（`shutdown_all` → `app.exit` → `RunEvent::Exit` 再收一次，幂等） |
| panic | 全部（panic hook：打崩溃现场 → 回收 → `abort()`） |
| `tauri dev` 重载 / `kill -9` / `kill -TERM` | 全部 —— **另一个进程**：看门狗读到管道 EOF（ADR-0005） |

⚠️ **关闭标签页 ≠ 关闭窗口 ≠ 退出应用**：关闭**最后一个**标签页只是进入**空状态**（界面为空、进程保留）；
关闭窗口（默认）只是**隐藏**。只有**三大终端**（local / ssh / serial）的标签页带关闭按钮；
转发 / 密码库 / 文件传输是**仅渲染**的视图标签页（**无关闭按钮**）—— 见 `docs/scope.md` §5.6。

**可搬迁性（0405）**：一条数据目录规则 + 一条配方。判定"可写"的方式是**实际写入一个探针文件再删除**
（`.akasha-writable`）—— mode 位无法反映 ACL / 只读挂载 / squashfs。配方 `just portable` 自动执行完
`portable.md` §5 的五步（复制 bin → A 启动 → 完全退出 → 迁移为 B → B 启动 → 断言四类池 **1/1/1/1**
+ 库侧逐项比对内容）。

## ⚠️ UI 现状：**当前界面是功能验证壳层，不是设计稿**

**正式 UI 的布局 / 视觉 / 交互尚未有设计稿。** `src/**` 现有的界面（标签栏、状态栏、主机选择器、
提示面板、配色、空状态文案）只有一个用途：让后端行为可被观察、可被验证。规则写在 `AGENTS.md` §4.0，
展开在 `docs/scope.md` §1.3。三点：

- **不得**把当前界面视为产品约束或"既有风格"，也不得在其上进行视觉打磨；
  前端改动的判据是"**这条后端行为能否被验证**"，而不是视觉质量。
- 后端**不得**依赖前端的呈现方式：界面整体重做时，命令 / 事件 / `Session` 状态机与收尾路径
  应当**原样可用**。
- 验证用的探针与选择器（`window.__akashaTerminal`、`.tab-pane.is-active …`、
  `.ssh-prompt[data-prompt-kind]`、`.host-picker-jump`）是**测试接口**，不是 UI 规范。
  **重做界面属于尚未规划的工作**。

## 已验证为通过（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + gen-types-check + docs-check） | 退出码 **0**，**6/6 全部通过** |
| `just test` | **386 tests run: 386 passed**（`akasha` **80** + `akasha-core` 30 + `akasha-pty` 39 + `akasha-serial` **24** + `akasha-ssh` **86** + `akasha-store` 127）。⚠️ `akasha` 的 80 条包含 `tests/` 下的集成目标（未设置 `VICTAURI_E2E` 时它们只输出原因并返回 —— 其中 `portable` 3 条、`ssh_session` / `ssh_jump` / `ssh_config_import` / `tunnel_state` / `tunnel_local_forward` / **`tunnel_dynamic_forward` / `tunnel_remote_forward` / `tunnel_reconnect` / `tunnel_teardown` / `sftp_dual_pane` / `sftp_transfer_atomic` / `sftp_host_to_host` / `sftp_pipelining` 各 1 条**） |
| `just serial-check`（plan 0802） | 退出码 **0**：`akasha-serial` 的 **24 条**在两种 feature 配置下**各执行一遍**（默认走 libudev 的枚举实现，`--no-default-features` 走 sysfs 的），两次都是 **24 passed / 0 skipped** |
| ↑ **判据：枚举在本机列出端口**（plan 0802） | ✅ `ports()` 在本机（libudev）返回 **32 条** `/dev/ttyS0`…`/dev/ttyS31`，**按路径排序、无重复、路径非空**，且每条在 `/sys/class/tty/<名字>` 里都有对应项（库内 `enumeration` 2 条）；关闭该 feature 后**同一套用例**返回 **0 条**且照常通过 —— "没有端口"与"枚举失败"因此是两种结果 |
| ↑ **判据：参数错误给出可读报错**（plan 0802） | ✅ 越界取值报**字段与取值**（`data_bits = 9` / `stop_bits = 3` / `baud = 0`，库内 3 条）；打不开报**路径与 OS 原因**（不存在的设备与一个目录路径都实测过） |
| ↑ **参数在真 tty 上回读**（plan 0802） | ✅ 请求 `115200 / 7 数据位 / 2 停止位 / 偶校验 / 软件流控` → 回读 `115200` / `Two` / `Software` **等于请求值**；⚠️ 数据位与校验位被 PTY 归一化（回读 `Eight` / `None`），那两项只到"映射是全的" —— 见「待验证」 |
| ↑ **判据：转发端口可访问远端服务**（plan 0602） | ✅ `tunnel_local_forward` E2E（真实 app + **测试进程内**一台 SSH 服务端与一个回声服务端，**1.05 s**）：界面打开池里那条规则 → 主机密钥与口令各答一轮 → probe 报 `bind = 127.0.0.1:<规则端口>` → 从测试进程连该端口**写一行、读回同一行**（回声服务答的） |
| ↑ **走的是 `direct-tcpip`，且每条入站连接各开一条通道**（plan 0602） | ✅ **对端记到恰好 1 条 `direct-tcpip` 请求**（`host` = `akasha-local-forward.invalid`、`port` = 回声服务端口，**本机解析不出这个名字** —— 用例自行解析一次并断言失败）、中继字节数 `> 0`；同一端口再连一次 → 请求数变 **2** |
| ↑ **端口被占用的报错可读**（plan 0602） | ✅ 规则指向一个被本进程占着的端口：界面显示「本地监听 127.0.0.1:38725 绑定失败：地址已在使用 (os error 98)」，且 probe 里**没有**它（**没登记成** —— 重试也不会好，用户要动的是端口） |
| ↑ **停止后端口释放、连接断开**（plan 0602） | ✅ 点"停止" → 该端口**不再接受连接**（连接被拒，不是超时）、`sessions` 的 `live`/`registered` = **1/1**、**服务端看到 1 条连接断开** |
| ↑ **判据：远端监听端口可回连到本机服务**（plan 0604） | ✅ `tunnel_remote_forward` E2E（真实 app + **测试进程内**一台 SSH 服务端与一个 HTTP 服务端，**7.01 s**）：界面打开池里那条 `remote` 规则 → 主机密钥与口令各答一轮 → probe 报 `bind = "127.0.0.1:<规则端口>"` → **`curl http://127.0.0.1:<那个端口>/probe`**（**现成**客户端）**退出码 0**，且取回的响应体就是**本机** HTTP 服务写的那一串 |
| ↑ **服务端真的在听，且通道由它发起**（plan 0604） | ✅ 服务端记下的 `tcpip_forward` 请求：地址 `127.0.0.1`、端口与规则一致、被认下，且**实际绑的端口**就是回报给我们的那个；`forwarded_tcpip_accepted` **1 → 2**（每条入站连接各一条通道）、中继字节数 `> 0`、本机 HTTP 服务的请求计数 `≥ 1`（它只听 `127.0.0.1`，所以字节只可能经那条通道到达） |
| ↑ **本机目标不可达 → 通道被拒**（plan 0604） | ✅ 库里那条目标指向**没人听的端口**的规则：连一次服务端那个端口 → 服务端看到 `ConnectFailed`，而 `forwarded_tcpip_accepted` **没有增加**（"先接受再关"的实现会在这里什么都不留下 —— 那正是这条断言要分得开的） |
| ↑ **远端端口拿不到 → `失败` 且看得见**（plan 0604） | ✅ 库里那条绑定端口**在服务端那一侧被占着**的规则（用例自己握着那个端口）：请求失败 → probe 里那条 `state = failed`、**事件里有 `failed`**（托盘与界面据此可见）、面板上显示「远端监听 127.0.0.1:36779 没拿到：服务端拒绝了这条转发请求：那个端口在它那一侧被占着，或它不允许远端转发」、服务端记下那条请求 `accepted = false`；⚠️ 它**仍然登记着**（可重试）—— 与"本机端口没拿到"（没登记成）不是同一种失败 |
| ↑ **停止即撤销**（plan 0604） | ✅ 点"停止" → 那个端口**不再接受连接**、服务端收到 `cancel-tcpip-forward` 用的正是**它回报的那个端口**、`sessions` 的 `live`/`registered` = **1/1**、服务端看到 **3 条**连接断开（三条规则各一条） |
| ↑ **判据：掉线进「重连中」、耗尽次数变「失败」且三处可见**（plan 0605） | ✅ `tunnel_reconnect` E2E（真实 app + **测试进程内**一台 SSH 服务端与一个 HTTP 服务端，**19.19 s**）：两条隧道（一条 `-L`、一条 `-R`）都连上且两个端口都能 `curl` 通 → 服务端把会话断开 → 事件里各出现 `reconnecting`（`attempt = 1`）、面板上写着「重连中（第 1 次）」→ **两条都自己回到 `connected`、两个端口又都能 `curl` 通**；随后服务端**整个消失** → 事件序列 `reconnecting(1) → reconnecting(2) → reconnecting(3) → failed`，从消失到 `failed` 实测 **7.55 s**（预算 ≥ **7 s** = 1+2+4），probe 里 `state = failed` 且不再报监听地址、面板上写着「失败」 |
| ↑ **重连是"重新连一次"，`-R` 还要重新请求监听**（plan 0605） | ✅ 同一个 E2E：服务端的 `tcpip_forward` 请求数 **1 → 2**（两次都被认下、第二次的端口**仍是规则里那个** —— 重连之后端口不许悄悄变）、`direct_tcpip` 请求数增加（`-L` 的重连是**另起一条连接**）、被切断的两条连接都在服务端记到断开 |
| ↑ **重连途中停止要能中止循环**（plan 0605） | ✅ 再切一次 → 在退还避里点"停止" → 那条从 probe 里消失，且 **3 秒内没有新的 `connected` 事件**（退避是 1 s；负控等的是"出现"这件事，超时才是通过） |
| ↑ **失败之后仍可手动重试**（plan 0605） | ✅ `tunnel_retry` 回非空 `failure`，事件里再走一遍 `connecting → failed`（服务端仍未回来） |
| ↑ **重连机制是库内可测的**（plan 0605，crate 层） | ✅ `akasha-ssh` 新增 **5 条**（`ending` 3 单测 + `local_forward` / `remote_forward` 各 1 条）：两个结束原因的短名互不相同 / 原因经通道送达 / **发送端直接消失按"停止"处理**（不许自作主张重连）/ 连接被切断 → 转发以 `ConnectionLost` 结束**且端口随之释放** / 同一条在 `-R` 上成立**且服务端那一侧的监听随连接消失** |
| ↑ **退避与预算是一处纯逻辑**（plan 0605，crate 层） | ✅ `akasha-core` 新增 **3 条**：默认 3 次 + `1s → 2s → 4s`、第 4 次没有预算、`budget() = 7s` / 把重连预算设成 0（`max_attempts = 0`）时第 0 次也没有预算 / 荒唐的 `factor` **饱和**而不溢出（配置里的数字是用户给的） |
| ↑ **哪些失败不重试是纯逻辑**（plan 0605，app 层 2 条） | ✅ `TunnelError::retryable`：传输层（`connect` / `jump`）与端口没拿到 → 重试；认证 / 主机密钥 / 配置 / 内部状态 → **不重试** |
| ↑ **判据：关闭转发 `Session` 后两个计数都归零**（plan 0606） | ✅ `tunnel_teardown` E2E（真实 app + **测试进程内**一台 SSH 服务端与一个 HTTP 服务端，**5.48 s**）：`-L` 与 `-R` 两条隧道都连上、两个端口都能 `curl` 通、`residue = (2, 2)`、服务端也看到 2 条连接 → 面板「停止」关闭 `-R` → `residue = (1, 1)`、服务端 1 条、**服务端那个远端端口连不上**（撤销了监听）、本机那条仍 `curl` 得通 → 关闭 `-L` → `residue = (0, 0)`、服务端 **0 条**、两个端口都还回去了 |
| ↑ **关闭是幂等的，且不会被重新拉起**（plan 0606） | ✅ 同一个 E2E：对同一个句柄再 `tunnel_stop` 一次返回 `Ok`（不是错误）、它没有回到册里；**1.5 秒后再读** `residue` 仍是 `(0, 0)` —— 看护循环若还活着会在退避之后把它重新连起来 |
| ↑ **在途的尝试也能被中止**（plan 0606） | ✅ 同一个 E2E（第三条规则指向一台"接了 TCP 就不再说话"的进程）：先让它落到 `失败` → 对端开始接听但不回话 → 点「重试」（卡在握手中）→ 点「停止」→ **对端 7.5 ms 内读到 EOF**。⚠️ 修之前这条断言是**红的**：那次握手发生在阻塞线程上，丢掉 `await` 取消不了它，socket 要等 `connect_timeout`（10 s）才关 |
| ↑ **取消是库内可测的**（plan 0606，crate 层 2 条） | ✅ `akasha-ssh` 新增 `connection_count` 1 条（一条连接一被持有就上账、丢掉就下账，两条各算一条）+ `cancellable_connect` 1 条（`cancel` 就绪即返回 `SshError::Cancelled`，且**对端读到 EOF** —— 不是"函数返回了"就算完）；app 层 1 条钉住停止信号的两条性质（订在停止之后的接收端不响、第二次停止照样响） |
| ↑ **判据：配置 SOCKS5 代理后能访问远端网络**（plan 0603） | ✅ `tunnel_dynamic_forward` E2E（真实 app + **测试进程内**一台 SSH 服务端与一个 HTTP 服务端，**0.85 s**）：界面打开池里那条 `dynamic` 规则 → 主机密钥与口令各答一轮 → probe 报 `bind = 127.0.0.1:<规则端口>` → **`curl --socks5-hostname 127.0.0.1:<端口> http://akasha-dynamic-forward.invalid:<端口>/probe`**（**第三方**客户端）**退出码 0**，且取回的响应体就是远端服务写的那一串 |
| ↑ **目标由客户端说，且每条入站连接各开一条通道**（plan 0603） | ✅ **对端记到恰好 1 条 `direct-tcpip`**（`host` = curl 在握手里给的那个名字、`port` = HTTP 服务端口，**本机解析不出这个名字** —— 用例自行解析一次并断言失败）、中继字节数 `> 0`；再 curl 一次 → 请求数变 **2**；中继表里**没有**的名字 → 客户端收到 `REP 0x02`（**分类真的到了客户端**，而不是通用的 `0x01`） |
| ↑ **非回环绑定被拒**（plan 0603 的安全项） | ✅ 库里那条 `bind_host = 0.0.0.0` 的 `dynamic` 规则：界面显示「SOCKS5 监听不能绑到 0.0.0.0：这一侧无认证，只允许绑回环地址（127.0.0.1 / [::1] / localhost）」，且 probe 里**没有**它（**没登记成** —— 端口一次都没绑过） |
| ↑ **停止后端口释放、连接断开**（plan 0603） | ✅ 点"停止" → 该端口**不再接受连接**、`sessions` 的 `live`/`registered` = **1/1**、**服务端看到 1 条连接断开** |
| ↑ **`-R` 是库内可测的**（plan 0604，crate 层） | ✅ `akasha-ssh` 新增 **6 条**（`remote` 2 单测 + `remote_forward` 4 集成）：按**端口**查表（没登记过的端口没有路由、超出 `u16` 的端口号匹配不上、凭据 drop 即撤销、陈旧凭据不清别人的登记）/ `port = 0` 时用服务端回报的端口 / **请求具体端口时端口就是请求的那个**（回复里没有端口字段，上游把它表示成 `0` —— 见问题 #134）/ 正例（服务端监听端口 → `forwarded-tcpip` → 本机回声服务，通道数 1→2）/ 反例（本机目标不可达 → 服务端看到 `ConnectFailed` 且 `accepted` 为 0）/ 远端端口拿不到 → `RemoteListen` 且错误里带地址 / 停止后端口释放 + `cancel-tcpip-forward` 用的是服务端回报的那个端口 |
| ↑ **SOCKS5 服务端是库内可测的**（plan 0603，crate 层） | ✅ `akasha-ssh` 新增 **15 条**（`socks5` 11 + `relay` 3 + `socks5_forward` 1）：协商选中无认证（多给两个方法也会选中 `0x00`）/ 三种 `ATYP` / 只提供口令认证回 `05 FF` / `BIND` 回 `0x07` / 不认的 `ATYP` 回 `0x08` / 版本不对**什么都不回** / 只写了一半的报文在有限时间内结束 / `REP` 的字节值就是协议值 / 失败类别翻成 `REP` 的表 / 只放行回环地址 / **非回环在绑定之前就被拒** / 空绑定地址按入站类型给出不同的提示 / 正例（SOCKS5 端口 → 对端中继 → 回声服务，请求数 1→2、`REP 0x00` 之后才通话）/ 反例（表里没有的名字 → `REP 0x02` 且连接被关闭，`BIND` 与未知 `ATYP` **不去开通道**） |
| ↑ **转发本身是库内可测的**（plan 0602，crate 层） | ✅ `akasha-ssh` 新增 **7 条**（`relay` 4 + `local_forward` 3）：端口 0 由内核分配并报回实际地址 / 端口被占用报 `Listen` 且带地址与原话 / 空绑定地址被拒 / 正例（本地端口 → 对端中继 → 回声服务，请求数 1→2、中继字节数 `> 0`）/ 停止后端口释放 + 连接断开 / **负控**（没人绑的端口连不上、被占端口是 `Listen` 而不是 `Connect`） |
| ↑ **判据：五态可观测 + 状态变化发事件**（plan 0601） | ✅ `tunnel_state` E2E（真实 app + **测试进程内**一台服务端，**0.90 s**）：界面点开隧道面板 → 打开池里那条能连通的 → 主机密钥与口令各答一轮 → probe `tunnels` 的 `state = connected`、界面上 `data-tunnel-state="connected"`、事件序列 `["connecting","connected"]` 且 `handle` 与 probe 一致 |
| ↑ **`→ 已停止`，以及"连接真的断了"**（plan 0601） | ✅ 点"停止" → probe 里那条消失、`sessions` 的 `live`/`registered` = **1/1**、**服务端看到 1 条连接断开**（plan 0601 新增的连接级计数 `connections_closed` —— 隧道没有通道，`sessions_closed` 在这条路上恒为 0）、事件里有 `stopped` |
| ↑ **`连接中 → 失败` 可见，且能手动重试**（plan 0601） | ✅ 连不上的那条（规则指向一台**不可达主机**）：`tunnel_open` 回 `{handle, failure:{kind:"failed",…}}`、probe `state = failed`、事件里有 `failed`；`tunnel_retry` 再走一遍 `connecting → failed`；**全程没有 `reconnecting`** —— 首次连接失败不自动重试（plan 0605 的口径，见「阶段 6 的形状」） |
| ↑ **状态机是纯逻辑**（plan 0601，crate 层） | ✅ `akasha-core` 新增 **12 条**：正例表 / 反例表（含同态全部被拒、重连直达 `已连接`）/ `attempt ≥ 1` / 重试面（只有 `失败`·`已停止` 可重试）/ **五态从 `连接中` 都走得到**；注册表侧新增 1 条按 `SessionId` 路由 |
| ↑ **建链只留一份实现**（plan 0601，crate 层） | ✅ `hops_chain` 被 `SshTransport::connect_via` 与 `SshConnection::connect_via` 共用；`akasha-ssh` 既有用例（跳板正例 + 负控 + known_hosts 7 条）**行为未变** |
| ↑ **判据：含 `Match` 的配置产生明确报错**（plan 0506） | ✅ `ssh_config_import` E2E（真实 app + 测试进程内两台服务端）：在界面导入一份含 `Match` 的配置 → **逐条**列出"第 4 行 `match`：条件块无法求值…"且**不产生导入报告** → `vault_hosts` 行数与导入前相同（一行未写） |
| ↑ **导入的跳板可用**（plan 0506 —— 这也是它排在 0505 之后的原因） | ✅ 同一 E2E 的另外两半：① 界面列出**支持集**（`Host,HostName,User,Port,IdentityFile,ProxyJump`）→ 填路径 → 导入报告 `新增 2 · 更新 0 · 跳过 0`，两条"未生效"逐条带行号（`serveraliveinterval` / `identityfile`）；② 使用**导入得到的条目**建立经跳板的会话 —— 四条提示按序答完 → 跳板记到 1 条 `direct-tcpip → akasha-e2e-import.invalid:22` → 字节到达目标 |
| ↑ **导入只迁移配置、不迁移私钥**（P2） | ✅ E2E 中该行的 `auth = publicKey` + `keyId = null`（密钥在 agent 中）；`no_absolute_paths` 新增一条：配置中写明 `IdentityFile /home/nobody/.ssh/id_ed25519`，导入完成后**库中没有任何值提及该路径**（解析器识别到它，仅写入报告） |
| ↑ **解析语义与系统 `ssh` 逐字一致**（开发期对照，不进任何门禁） | ✅ 同一 fixture 交给 `ssh -G`：`hostname akasha-e2e-import.invalid` / `user e2e` / `port 22` / `proxyjump e2e-config-jump` —— 与导入池中的条目**逐字一致**。⚠️ 系统 `ssh` 不是本产品的依赖（`scope.md` §2.1），仅在开发期作为**差分对照物** |
| ↑ **三档边界在 crate 层固化**（plan 0506，纯函数） | ✅ `sshconfig_parse` **29 passed**（0.11 s）：首次取值优先 / 全局段 / 关键字不区分大小写而 `Host` 模式区分 / 通配块不产生条目 / `ProxyJump` 的 `none`·逗号链·补建 / `Match`·`Include`·改目的地（`ProxyCommand` / `Canonicalize*` / 源地址）·改信任来源 一律报错且**一次列全** / 局部指令逐条警告 |
| ↑ **落库判据**（plan 0506） | ✅ `hosts_import` **5 passed**：链挂上且 `jump_chain` 可读出 / 同名默认不动、`overwrite` 才替换 / **补建的跳板条目永不覆盖**用户写入的行 / **手工写入的环被存储层拒绝且一行不写**（事务回滚） |
| ↑ **判据：ProxyJump 可连通仅对跳板机可见的目标**（plan 0505） | ✅ `ssh_jump` E2E（真实 app + **测试进程内两台**服务端）：池中该行的 `host` 是 `akasha-e2e-inner.invalid`（用例自行解析一次并**断言失败**）→ 界面选中它 → **四条提示按序答完**（跳板主机密钥 / 跳板口令 / 目标主机密钥 / 目标口令）→ 连通 → **跳板服务端记录恰好 1 条 `direct-tcpip → akasha-e2e-inner.invalid:22`** → 输入字节到达**目标**服务端 |
| ↑ **该名字在本机不可解析**（构造前提） | ✅ `(INNER_NAME, 22).to_socket_addrs()` **返回 Err** —— 由用例自行断言。因此"字节到达目标"**只可能**经过跳板（无特权环境无法构造真实网络隔离，这是替代口径，见「待验证」） |
| ↑ **每一跳各自询问凭据**（D8 的缓存键含 host） | ✅ 跳板服务端收到 `jump-host-password`、目标收到 `inner-host-password`（**两者不同**，给错即认证失败）；两台的**指纹也不同**（各自被询问过一次） |
| ↑ **跳板上的中继确实转发**（服务端侧证据） | ✅ 跳板的 `relayed_bytes` **> 0**（实测 5240 字节，会话仍开启时即可读出）；关闭标签页后中继结束、目标服务端观察到连接断开 |
| ↑ **库内的原语验收**（plan 0505，crate 级 4 条） | ✅ `jump_host` **4 passed / 0.25 s**：正例（跳板恰好 1 条 `direct-tcpip`，host/port 与配置一致）+ **负控**（不经跳板直连该名字**必须失败**）+ 跳板拒绝转发时返回 `SshError::Forward` 而非 `Connect` + 同步门面在 tokio 上下文中被拒绝 |
| ↑ **跳板链**（plan 0505，store 级 3 条） | ✅ `pools_roundtrip` **11 passed**（该目标的用例总数；其中 3 条为 plan 0505 的跳板链判据）：链**目标在前**可读出；**手工以 SQL 写入的环**在读路径上被拦截（写入路径无法阻止直接改库，而环会使连接**永久阻塞**）；超过 `MAX_JUMP_DEPTH` 的链报错 |
| ↑ **判据：真实 app 上建立 SSH 会话**（plan 0504） | ✅ `ssh_session` E2E（真实 app + **测试进程内**服务端，**2.32 s**）：界面点击 SSH → 选中池中该行 → **主机密钥提示中的指纹等于服务端的指纹** → 接受 → 口令提示 → 作答 → 连通（标签页标题 = 池中的名称） |
| ↑ **字节双向流动 / 已确认密钥写入库 / 凭据仅询问一次 / 关闭标签页零残留**（plan 0504） | ✅ 四项各有断言：回显出现在屏幕上**且服务端收到同一串**；直连库文件读到 `known_hosts` **1 行**；第二个会话**未发起任何询问**即连通（服务端第 2 次收到**同一口令**）；关闭两个标签页 → `sessions` probe 返回 **`{"live":1,"registered":1}`** + 服务端观察到 **2 条**连接断开 |
| ↑ **提问往返本身**（plan 0504，crate 级 6 条） | ✅ 答案送达发起提问的一方 / 超时会**撤回**该提问、其后作答报 `Gone` / **取消与超时可区分** / **无人作答 = `HostKeyUnknown`（拒绝），而非 `Ok`** / 用户接受后**确实写入缓存** |
| ↑ **判据：直接打开 SFTP 即可用，无终端依赖**（plan 0701） | ✅ `sftp_dual_pane` E2E（真实 app + 测试进程内**两台**服务端，**7.34 s**）：**不新建任何终端标签页**，只打开 SFTP 面板 → 新建会话 → 两栏各选一台主机（指向两台不同的服务端）→ 两侧各答一轮（主机密钥 + 口令，提示里的地址分别是两台服务端）→ **左栏列出 `left-alpha.txt` / `left-dir`、右栏列出 `right-beta.txt`**，且左栏**没有**右栏的条目 |
| ↑ **两侧独立、且真的各连了一条**（plan 0701） | ✅ 同一个 E2E：只连左侧时右侧 `state = disconnected`；probe `sftp = {"handle":27,"sides":[{"side":"left","name":"e2e-sftp-left","state":"connected","path":"/",…},{"side":"right",…}]}`（路径是服务端 `realpath` 的结果）；两台服务端**各**记到 **1 次** `sftp` 子系统请求；打开前后**终端标签页数不变**；注册表 `live`/`registered` 从 **1 → 2 → 1**（关闭会话之后），两台服务端各看到那条连接断开 |
| ↑ **对端没开 SFTP 会明确失败**（plan 0701，crate 层负例） | ✅ `akasha-ssh` 新增 2 条（`sftp_session`）：正例（条目与对端给的一致且按名字排序、路径是 `realpath` 的结果、对端记到一次子系统请求）+ **负例**（服务端不提供 sftp → `sftp_with_timeout(1)` 在期限附近失败、错误里带"sftp 子系统"、且**没被记成认下**） |
| ↑ **判据：中断传输后目标目录里没有看似完整的文件**（plan 0702） | ✅ `sftp_transfer_atomic` E2E（真实 app + 测试进程内一台服务端，其 SFTP 根目录是**真盘**上的一棵临时目录，**6.19 s**）：左栏选**本机**并前往测试自己的临时目录 → 右栏连主机 → 传 4 MiB 的 `big.bin`（服务端每次读等 20 ms）→ **取消之前搬了 2883441 / 4194304 字节，目标目录实测 `[".big.bin.part"]`**（临时名在、最终名不在）→ 点「取消」→ 状态转 `cancelled` → **取消之后目标目录 0 个条目**（既没有 `big.bin`，也没有临时名）→ 探针里那条传输也是 `cancelled` |
| ↑ **成功 = 原子重命名落地，且字节相同**（plan 0702） | ✅ 同一个 E2E：本机写一个 300 KiB 的 `small.bin` → 左栏「传到对侧」→ 状态转 `done` → **对端真盘上 `small.bin` 的字节与源逐字节相同**，且对端目录只有 `[big.bin, small.bin, sub]`（没有临时名） |
| ↑ **关闭 `Session` 也要清干净**（plan 0702，`scope.md` §4.2 的第三格） | ✅ 同一个 E2E：再传一次 `big.bin` → 有进度时目标目录是 `[".big.bin.part", "small.bin"]` → 点「结束会话」→ `sftp_close` **先中止传输并等清理落地、再断连接** → 两栏消失之后目标目录只剩 `["small.bin"]`；探针里会话为空、服务端看到那条连接断开 |
| ↑ **传输的落盘不变量是库内可测的**（plan 0702，crate 层 6 条） | ✅ `akasha-ssh` 新增 `transfer_atomic` 6 条：两个**可控的假端点**（假目标在临时文件建好时报一次信号、假源交出第一块后停住）把"传到哪一步"变成**可以等待的事件** —— 成功那条实测"此刻目标目录只有 `.payload.bin.part`、长度正好是 `CHUNK_BYTES`、最终名不存在"，放行之后字节逐一同、临时名消失；取消那条（源永久停住）取消后目录**空**；写失败那条（第 2 次写入报错）同样**空**；已存在的 `.b.txt.part` **不被覆盖**而最终名照样正确；本机端点单独列目录（排序 + 类型）；与本机端点之间的**上传 / 下载**拿服务端真目录对账（字节相同、没有临时名） |
| ↑ **判据：A 无法直连 B 时自动走 A 档；两档均不落盘**（plan 0703） | ✅ `sftp_host_to_host` E2E（真实 app + **测试进程内两台**服务端，**5.29 s** —— 同一条用例在不同轮次实测过 5.3～11.5 s，耗时取决于几次握手与面板刷新，不是断言）：左栏连跳板、右栏连那台 `host` 是 `akasha-e2e-sftp.invalid:22` 的主机（**本机解析不出这个名字** —— 用例自己解析一次并断言失败）→ 右栏显示"经 … 直通"、探针 `through` = 跳板那一行 → **跳板收到恰好 1 条 `direct-tcpip → akasha-e2e-sftp.invalid:22`，中继搬了 55776 字节** → 目标盘上 `alpha.bin` 的字节与源逐字节相同、目录里没有临时名、**源那一台一个条目都没多**（`不落盘` 的两档口径见「待验证」） |
| ↑ **回退可观测，且回退之后传输仍然走通**（plan 0703） | ✅ 同一个 E2E：右栏换成本机能直达的地址 → 跳板**拒转发**（探针里的原因原文是 `AdministrativelyProhibited`）→ `through` 为空、`throughFailure` 非空且界面上也写着那句 → **第二个文件照样落到目标盘上**，而传输记录里的 `via` 为空（本机内存中转）；服务端侧证据：跳板收到 **2 条** `direct-tcpip`（第二次那条没被认下） |
| ↑ **两档共用同一个引擎，且 B 档只消费 0505 那条流**（plan 0703，crate 层 3 条） | ✅ `akasha-ssh` 新增 `host_to_host` 3 条：B 档（`connect_via([跳板], 目标)` → 两条 SFTP 会话 → 一个 `transfer()`；跳板恰好 1 条 `direct-tcpip`、中继搬过字节、目标真盘上字节相同且没有临时名）+ **负控两半**（不经跳板直连那个名字报 `Connect` / 跳板没有那条映射时报 `SshError::Forward`，且 `relayed_bytes = 0`）+ A 档（本机分别连两台，`direct_tcpip` 为空） |
| ↑ **判据：大量小文件的吞吐显著优于串行请求**（plan 0704） | ✅ `sftp_pipelining` E2E（真实 app + 测试进程内一台服务端，主机行指向一条**每方向延后 10 ms** 的链路，**5.39 s**）：左栏本机、右栏那台主机 → **串行**（12 个文件逐个发、逐个等结束）**1.553 s**、探针 `peak = 1` → **并发**（一口气发完）**561 ms**、`limit = 8`、`live = 0`、`peak = 5` —— **2.8×**；两批之后对端**真盘**上 12 个文件的字节逐一相同、目录里没有临时名，链路共搬了 208 段。⚠️ 耗时与 `peak` 每次不同（另一轮实测串行 1.509 s / 并发 389 ms / `peak 8`），断言只有"并发明显更快"这一条 |
| ↑ **上限真的在，且排队与取消都在并发下正确**（plan 0704，crate 层 6 条） | ✅ `akasha-ssh` 新增 `pipelining` 6 条：7 个文件 / 上限 3 时目标端点**同时**只见到 3 个 `begin_write`（第 4 个连闸门都进不去）、放行后 7 个都落地而 `peak` 停在 3 / 排队中被取消的那条**一个端点都没碰过**（它的路径从未被 `open`）、进去的那条走收尾、目标目录空 / 5 个文件中间那个写失败 → 另外 4 个字节正确落地 / 两条并发传输写**同一个最终名**时目标目录里是**两个**不同的临时名（放行后最终名的字节等于两条源之一）/ 上传一个文件：服务端记到的 `open` 恰好一次（`/.probe.bin.part`）且临时名没出现在 `stat` 里（"由对端保证的唯一性"在协议层就是少一次往返） |
| ↑ **分档数字**（plan 0704，crate 层） | 12 个 1 KiB 文件在带时延链路上：上限 1 = **1.168 s**、2 = 1.159 s、4 = 563 ms、8 = 377 ms、16 = 211 ms（每个文件 97.3 → 17.6 ms）。⚠️ **上限 2 与串行一样慢**，那一段没有定位（见「进行中 / 下一步」）；默认值因此不按饱和点取 |
| ↑ **`just test-e2e` 全部通过** | 退出码 **0**：第一段（默认收托盘）**25 个目标 / 30 个用例** + 第二段 `exit_residue` **1 个用例** = **26 个目标 / 31 个用例**、0 失败；第三段（可搬迁性）`portable` **3 passed**。⚠️ `E2E_NO_APP` 的 `session_watchdog` 不在第一段的目标清单里（另有入口），`E2E_SELF_APP` 的 `portable` 由第三段执行、不计在上面那个数里 |
| ↑ **判据：串口会话能打开并双向传字节、关标签页即回收**（plan 1101） | ✅ `serial_session` E2E（真实 app + **测试进程自己造的一对 PTY**（从端的路径当设备），**1.00 s**）：界面点"串口" → 选池里那一行 → 标签页出现且"已连接" → **设备 → 界面**（往主端写 `from-device-1101`，屏幕文本里出现它）→ **界面 → 设备**（在标签页里敲 `to-device-1101`，主端读到同一串）→ 关标签页后 `sessions` probe 回到打开前的 `{"live":1,"registered":1}`（**两数相等**） |
| ↑ **池行 → 参数这条搬运**（plan 1101，crate 层 5 条） | ✅ `akasha --lib serial` **5 passed**：两个 IPC 枚举的全量映射 / 六个字段的搬运 / **越界取值报字段与取值**（`data_bits` 与 `stop_bits` 各三个取值）/ **空路径报出用户给的那一串** / 不存在的设备报出它试过的路径 |
| ↑ **判据：界面上看到本机枚举结果并据此（或手输路径）打开；取值越界时显示字段与取值**（plan 1102） | ✅ `serial_ports_ui` E2E（真实 app + 测试进程自己造的一对 PTY，**1.46 s**）：后端枚举 **32 条** → 面板上那个计数与它**相等**（库**故意不解锁**：池那一块显示"先解锁"、端口照常列出 → 三条输入并列）→ 点第一条枚举结果 `/dev/ttyS0` → **"设备路径"那一栏变成它** → 手输 PTY 从端的路径 → 「打开」→ 标签页"已连接"、`from-device-1102` 到了界面、`to-device-1102` 回到设备 → 关标签页后 `sessions` 回到 `{"live":1,"registered":1}`；数据位填 **9** → 「打开」→ 那个面的报错行里是 **`data_bits = 9`**、关闭之后 probe 同样回到打开前（**没有登记**）；数据位填 `abc` → **不开面**，`[data-picker-problem]` 说清是哪一栏 |
| ↑ **端口映射与"契约加一档"的编译期代价**（plan 1102，crate 层 2 条） | ✅ `akasha --lib serial` **7 passed**：新增"四档 `PortKind` 各有去处（`Usb` 五项一个不丢）"与"五项都缺时保持 `None`（不填假值）"。⚠️ `SerialIpcError` 加 `enumerate` 之后，前端两处 `switch`（`SerialInvokeError` / `SerialPortsUnavailable`）当场**编译不过**（TS2366）——"契约加了一档、前端少一个分支"在编译期就现形 |
| ↑ **判据：设备消失时会话结束、没有残留注册（且原因可读）**（plan 1103） | ✅ `serial_device_gone` E2E（真实 app + 测试进程自己造的一对 PTY，**0.36 s**）：手输从端路径打开 → 标签页"已连接" →（**关闭唯一一份主端 = 拔掉设备**）→ 标签页自己关闭、`[data-session-notice]` 里是 **`「/dev/pts/0」已结束：串口设备已断开：/dev/pts/0（Broken pipe）`**、`sessions` probe 回到打开前的 `{"live":1,"registered":1}`。⚠️ 这条用例**先断言那句话此刻不在界面上**（正反例成对：否则"拔掉之后它出现了"会被上一次留下的通知顶过去） |
| ↑ **读端失败的形态与"自己收尾不留原因"**（plan 1103，crate 层） | ✅ `akasha-serial` **25 passed**：新增"关闭 PTY 主端 → 读端报错（**不是** `Ok(0)`），且 `stream_error` 说得清是哪台设备（含路径与`已断开`，问几次都是同一句）"；配套断言"设备安静（超时 / 零字节）与**我们自己收尾**都不留原因"。`akasha-pty` **40 passed**（新增"`stream_error` 的默认值是 `None`"） |
| ↑ **原因出得了载体**（plan 1103，app 层） | ✅ `akasha --lib session` **18 passed**：假载体报一句原因 → `retire` 把它**原样**交出（会话层不拼那句话）；**反例**：不报原因的载体拿到 `None`（只断言前者分不清"字段在工作"与"它永远是同一个值"） |
| ↑ **判据：主机密钥变化即拒绝，未见过的询问一次**（plan 0503） | ✅ `akasha-ssh` 的 7 条：未知且无人可问 → `HostKeyUnknown`（携带用于核对的指纹，且**认证尚未开始**）；确认 → 写入缓存，**第二个连接 0 次询问**；记录不匹配 → `HostKeyChanged`（**两个指纹都在**）且**不发起询问**；用户拒绝 → 拒绝连接且**不记录**；用户文件中已认可 → 连通且文件**逐字节未变** |
| ↑ **本仓库首次格式迁移**（plan 0503） | ✅ `akasha-store` 的 6 条：`DDL_V1` 构造出**真实 v1 库** → `open` 之后 `user_version = 2`、五张表存在、**该 host 行仍在**；再次打开当前格式的库**不写入任何字节**；缺表的 v1 **不迁移**；加密导出与明文导出两条还原路径均**升级副本、来源逐字节不变** |
| ↑ **判据：同主机三个连接仅询问一次凭据**（plan 0502） | ✅ `three_sessions_ask_for_one_credential`：`provider.calls() == 1`、缓存 `len() == 1`、服务端三次均收到**同一口令** |
| ↑ **认证顺序由协议交互验证**（plan 0502） | ✅ 服务端记录的序列：`publickey → password`、`publickey → keyboard-interactive`；agent 不可用时序列中**没有** `publickey` |
| ↑ **`nodelay` 实际生效**（plan 0505 修正） | ✅ `tcp_stream` 自建 TCP 时显式 `set_nodelay(true)`（问题 #120：上游仅在 `client::connect` 中读取 `Config::nodelay`，而两条路都使用 `connect_stream`） |
| ↑ **`Cargo.lock` 增量仅一行**（plan 0504 / 0505） | ✅ 新增 `akasha → akasha-ssh` 这条边**只增加一行**；0505 **未增加任何行**（无新依赖，`rand` 早已是 `akasha-ssh` 的真依赖） |
| `pnpm build`（tsc + vite build） | 退出码 0；产物 **878.75 kB / gzip 241.58 kB**（**+0.28 kB / +0.14 kB**：plan 1103 的应用级通知行；CSS 14.96 kB） |
| `just docs-check` | 全部通过（ROADMAP 62 个条目 ≤3 行且无代码块 / 53 份 plan ≤200 行且索引一致） |
| `ast-grep scan` + `ast-grep test` | 均退出 **0**（plan 0704 未新增 / 修改规则） |
| **三条 unsafe 注释 lint**（clippy，位于 `just lint`） | 退出码 **0**；三条各以一个探针验证其**确实会失败**（探针用后即撤） |
| ↑ **解锁 / 锁定 / 内存回收**（0407） | ✅ 真实 app 上 `VmLck` **0 → 176～192 → 0 kB**；进程内存扫描（带正对照）：口令 `1 → 2 → 2 → 1` 处、派生密钥 `3 → 1` 处 |
| ↑ **导出与还原**（0404） · **目录迁移**（0405） · **退出零残留**（0204/0205） | ✅ 三项判据仍然全部通过（`just portable` **3 passed**；`app 已退出` + `零残留`） |
| ↑ **终端 / 会话判据（未退化）** | `renderer = webgl`；写入 8 MB 数据后仍可交互；raw 通道 10.73 MB / 172 批；收尾帧 1 个、console 零异常 |
| ↑ **§7 的 registry 一条：当前不可满足**（问题 #82） | `get_registry` 返回 **`[]`**（命令均未标 `#[inspectable]`）。**替代证据**为真实路径上的 `invoke_command` 成功 —— plan 0704 的新命令均以此验证 |
| `cargo tree -p akasha-core \| grep -c tauri` | **0**（分层成立） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数据，0704 未重新运行**） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉取 RustSec 数据库。

## 待验证（本地或沙箱环境无法执行）

- **串口的真实硬件一次都没有被碰过**（plan 0801 / 0802）：本机 `/dev` 下没有任何串口设备节点
  （libudev 那套枚举仍列出 32 条 `/dev/ttyS*`，它们**全部打不开** —— 见问题 #150），
  所以"枚举出**可用**的端口"与"与真实设备互操作"这两条都没有本地证据。同一处还有一半：
  **数据位与校验位在 PTY 上被内核归一化**（实测请求 7 数据位回读 8 位、请求偶校验回读不校验），
  所以那两项的证据只到"映射是全的"（纯函数用例），"在真设备上生效"未实测。
  ⚠️ 这条同时说明反向的一件事：**PTY 不是串口参数的忠实回读装置**，把它当成全部五个参数的
  证据会得到一份看起来完整、实际只覆盖三项的读数。
  ⚠️ **plan 1101 的端到端判据也建立在这同一条替代口径上**：`serial_session` 的两串字节
  确实经过内核的 PTY 走了一个来回（两个方向各是独立的证据），但"与一台**真实串口设备**
  互操作"仍然没有证据 —— 真设备上有 USB 串口芯片、模数转换、流控引脚，PTY 一样都没有。
  ⚠️ **plan 1102 有一半判据只在有端口的机器上才执行**：`serial_ports_ui` 里"界面上那个计数 ==
  后端枚举的条数"在本机（32 条）与 CI 上都成立，而"点一条枚举结果 → 路径栏被填上"在枚举为空的
  机器上会**显式跳过并写明原因**（CI 的 runner 就是这一类）。那一半的"据此打开"因此没有
  跨机器的证据 —— 它在每一台机器上要么被验过、要么被说过。
  ⚠️ **plan 1103 也建立在这同一条替代口径上**：本机实测的"设备消失"是**关闭 PTY 主端**
  （`POLLHUP` → `BrokenPipe`）。真 USB 转串口设备被拔掉时内核报的是 EIO（上游 `wait_fd` 的
  另一支，走 `Other(EIO)`）—— 两条都进 `Err`，所以那条路不会静默，但 **EIO 那一支没有实测**。
  ⚠️ 还有一条只在本机成立的前提：`portable-pty` 造出来的从端**不会**被别的进程按独占打开，
  而真实设备上 `serialport` 的 `TIOCEXCL` 会让**第二个**会话拿到 `EBUSY`。
  ⚠️ **`AGENTS.md` §7 里 `introspect { action: "processes" }` 那一条对串口没有对象**：
  串口没有本地进程（`session_leader()` 是 `None`，crate 也不 spawn 进程）——
  所以"真正退出之后零残留"在这条路上只剩注册表（`live` / `registered` 回到打开前的读数，
  实测 `1/1`）。这两件事**不是同一件**：注册表归零说的是"没有留下可管理的会话"，
  而"没有任何进程/线程留下"这一句在这里由**结构**保证（设备句柄 + 合批线程随流结束而退出），
  没有单独的读数口 —— `residue` 探针报的是 SSH 连接与隧道看护任务。
- **SFTP 只与自建的测试服务端对接过**（plan 0701 / 0702）：客户端这两条链（子系统请求、
  `realpath`、`readdir`；以及 `open` / `read` / `write` / `rename` / `remove`）**未与真实 `sshd`
  的 sftp 子系统互操作**；`limits@openssh.com` / `fsync@openssh.com` 这类扩展缺失时的降级路径
  因此也没有对照物（测试服务端**一个扩展都不声明**，所以降级那一侧每次都被走到 ——
  但"与真实 `sshd` 对上"仍是空白）。
- **吞吐数字只在一条人为带时延的链路上量过**（plan 0704）：本机回环的一次往返在微秒级，
  所以判据的口径是"给会话接一条每方向延后 10 ms 的链路"（`akasha_ssh::testing::slow_link`）。
  真实网络上的往返是**分布**的（抖动、丢包、拥塞窗口），而这里是一个定值 —— 它证明的是
  "并发把往返折叠起来了"，不是"在真实链路上快多少"。同一批的分档曲线里**上限 2 与串行
  一样慢**那一段也还没有定位（见「进行中 / 下一步」）。
- **临时名的原子占用只在自建的测试服务端上验过**（plan 0704）：本机那一侧是内核的 `O_EXCL`
  （`create_new`），远端那一侧靠对端实现 `CREATE|EXCLUDE` —— 与真实 `sshd` 的
  `sftp-server` 未对照。⚠️ 更值得记的是：**测试脚手架原先把 `EXCLUDE` 当成了 `CREATE`**，
  于是"抢占临时名"这条判据在库内看起来成立、实际没有（见问题 #147）。
- **传输的失败路径只覆盖到"写失败"**（plan 0702，crate 层有 1 条）：读源失败、重命名失败
  这两条退出路径没有**专门的**用例（它们与写失败共用同一段收尾代码，但"共用"是阅读结论，
  不是实测结论）。
- **host ↔ host 的两档都只与自建的测试服务端对接过**（plan 0703）：B 档走的是
  `direct_tcpip` + 隧道里的一个 SFTP 会话，A 档是本机分别连两台 —— 两档都**未与真实 `sshd`
  对照**（`AllowTcpForwarding no`、`PermitOpen` 白名单、转发通道的缓冲与吞吐特性都没有对照物）。
  它们改变的是**失败长什么样**；回退的触发点（建链失败即回退）不依赖具体是哪一种
  （ADR-0006 §4 已记这一条）。
- **"两档均不落盘"的判据是形状 + 目录事实，不是对本机磁盘的直接观察**（plan 0703）：
  支撑它的是三件事 —— 两档的端点都是**远端**端点（引擎拿不到本机路径，这是形状上的事实）、
  源那一台的目录**一个条目都没多**、目标那一台只有"临时名 → 最终名"这一次改名。
  "把文件先下到本机再上传"那种实现形态在判据上无法被直接证伪（没有本机路径可读）。
- **连接之后对端断开（会话变坏）这条路没有覆盖**：两侧的 `state` 会一直停在 `connected`，
  本阶段既没有健康检查也没有事件 —— 用户发现它只能靠下一次列目录失败。会话生命周期
  **尚未规划**。
- **`just dev-web` 的模拟后端没有 SFTP**：九条命令各自**明确报错**（与 SSH 那两条同一条口径），
  所以浏览器里的 SFTP 面板只会显示那句提示 —— 真路径要用 `just dev`。
- **`-R` 从未与真实 `sshd` 互操作**：判据全在**本仓库自建的测试服务端**上（plan 0604）——
  它实现 `tcpip-forward` / `cancel-tcpip-forward` 与 `forwarded-tcpip`，但**永远只绑回环**，
  `GatewayPorts` 那一层语义（非回环请求会被拒还是被改写）**未实测**。因此"请求 `0.0.0.0`
  会发生什么"在真实服务端上的答案**未知** —— 我们只请求，不做判断。
- **`-R` 的入站通道按端口认，而服务端回报的地址字符串未在真实服务端上验过**：
  测试服务端把请求里的地址原样回报，真实 `sshd` 的取值（是否规范化、是否受 `GatewayPorts`
  影响）未实测。按端口认正是为了不受它影响，但"真实服务端回报的端口与我们请求的一致"
  这件事只有 RFC 4254 §7.2 的文本作依据。
- **`-R` 的 `port = 0`（由服务端挑端口）只有 crate 层覆盖**：库里 `forwards.bind_port` 的
  `CHECK` 不接受 0，因此 app 这条路径**产生不出**这个请求；crate 用例覆盖了它（回报的端口
  可连、撤销用的是回报的那个）。
- **"本机服务不可达 → 拒绝通道"只在测试服务端上验过**：测试服务端能区分
  `ConnectFailed` 与 `AdministrativelyProhibited`（这正是判据），而真实 `sshd` 的 ssh 客户端
  侧如何呈现（是否在它自己的日志里记下原因）未实测。
- **`curl` 也是 `tunnel_remote_forward` 的前置**：判据的客户端必须是现成的客户端。
  与动态转发那条同一条口径 —— 没有 `curl` 的机器上会**失败**（不是跳过）。
- **SOCKS5 的正确性是"能互通"，不是"协议完全实现"**：只做了无认证的 `CONNECT`，且只与
  **`curl`** 这一种客户端实测过（浏览器、其它库未测）。认证协商 / `BIND` / UDP associate
  一律明确拒绝（`0x07` / `0x08` / `05 FF`）—— 这几档有用例，但**没有真实客户端**来确认它对拒绝的
  处理是否符合预期。
- **`REP 0x05`（对端拒绝连接目标）在本地构造不出来**：测试服务端是"先接受通道、再去连目标"
  （`testing.rs` 的简化），因此"目标连不上"表现为**成功 `REP` 之后连接被关**；真实 `sshd` 在
  连不上目标时回 `ChannelOpenFailure(ConnectFailed)`，那一档才变成 `0x05`。该映射只有单元测试
  覆盖（`reply_for` 的表），**未在真实 sshd 上验证**。
- **`0.0.0.0` 被拒这条判据验的是"拒绝"，不是"开放之后会怎样"**：本版本**没有**开放到同网段的
  路径（见「阶段 6 的形状」的安全项），所以"同网段的人经它访问远端网络"这一风险**从未被构造过**
  —— 用例只证明了那条监听起不来。
- **`curl` 是 `tunnel_dynamic_forward` 的前置**：判据的客户端必须是**现成的** SOCKS5 客户端
  （手写客户端证不了"现成客户端认这个服务端"，库内用例已覆盖那一半）。因此该用例在
  没有 `curl` 的机器上会**失败**（不是跳过）—— 本机与 CI 三平台都自带 `curl`。
- **`-R` 的"远端"与"本机"是同一个进程里的两个监听地址**（同 `-L` / `-D` 的构造口径）：
  无特权环境做不出网络隔离，区分两侧的是**谁在听**（测试服务端 / 本机服务）。
  ⚠️ 不得把这条读成"两台机器上验过"。
- **"断线"由测试服务端主动断开造出来，不是真的拔网线**：`Running::cut_connections()` 让服务端
  发一条 `SSH_MSG_DISCONNECT` 并关闭连接（客户端**立刻**发现），`Running::shutdown()` 再连监听
  一起停。真实的拔网线 / 网络分区下 TCP 不会立刻断，"半死"要靠保活耗尽才被发现
  （30 s × 3 ≈ 90 s）—— 那条路径**未实测**，`just test` 里也构造不出来。
- **关闭一条"半死"的隧道未实测**（plan 0606）：收尾里那一步是**礼貌断开**（发
  `SSH_MSG_DISCONNECT` 再关 socket），而"半死"（TCP 没断、对端不回话）时它写得出去、对面却收不到
  —— 那条路径要靠 TCP 自己的超时收场。本地造不出真实的分区（同下一条的口径），
  `tunnel_teardown` 的刺激是服务端**主动**断开。
- **`residue` 的连接数只数"已认证的连接"**（plan 0606 的口径）：一次**还在握手**的尝试没有
  `SshConnection` 可言，所以它不在这个数里 —— 判据"归零"说的是"没有连接留下"，而"那次尝试的
  socket 关没关"由 E2E 单独盯（对端读到 EOF 的时刻）。⚠️ 不得把"计数为 0"读成"没有任何 socket"。
- **真实 `sshd` 上"连接断 → 远端监听释放"的时机未实测**：重连要重新发 `tcpip_forward`，而那个
  端口必须已经**在服务端那一侧被释放**。测试服务端按连接持有转发（连接一断监听随之消失，与真实
  `sshd` 同形）。⚠️ 若真实服务端释放得更慢（例如要等保活超时），重连的第一次请求会拿到
  "端口被占" —— 用户看到的是重试三次后 `失败`。这一档没有实测。
- **托盘那一处断言在无托盘宿主的机器上显式跳过**：本沙箱里 `tray` probe 报 `ready = false`
  （Linux 上托盘图标要写 `$XDG_RUNTIME_DIR/tray-icon`），所以"托盘上写着失败"这条**从未真正
  读到过菜单**。probe 报的是**菜单项文案的来源**（`tray::tunnel_labels`，与菜单**共用同一个
  函数**），不是把 muda 菜单读回来 —— 后者要按 item 类型解构，多证明的只是"文案没有变换"。
  ⚠️ 还有一条**从未被走过的**路径：`set_tunnel_state` 现在会重推菜单，而它可能从 **runtime
  线程**被调用（看护任务里），tauri 的菜单构造走 `run_on_main_thread` 并**阻塞等待**结果
  （`run_main_thread!` 里是 `rx.recv()`）。本沙箱里托盘建不起来，所以"从 runtime 线程重推整份
  菜单"在真实桌面上是否顺畅**未实测**（同类调用点还有 `open_tunnel` / `remove_tunnel`，它们从
  IPC 那一侧来）。
- **"端口被占用"的判据依赖操作系统的错误码文案**：界面显示的是「绑定失败：地址已在使用 (os error 98)」
  这一句的**原文**，跨平台措辞不同（Windows 是 `os error 10048`）。用例断言的是"有地址、有原因"，
  不是某一句话。
- **隧道的停止是"同步命令 + 异步收尾"**：`tunnel_stop` 发完信号即返回，停监听、收在途连接、
  礼貌断开都在 runtime 上做 —— 观察收尾只能看**对端**（服务端的连接计数），命令的返回不代表已经收干净。
- **连接的 originator 仍是"最外层那条 TCP 的本地地址"**（plan 0505 的取舍）：`-L` 转发出去的
  `direct-tcpip` 请求带的就是它，而不是**那条入站连接**的对端地址 —— 真实服务器的审计日志因此
  看到的是我们，不是转发进来的客户端。是否改（以及怎么改）**尚未规划**。
- **"仅对跳板机可见"是构造出来的，不是真实的网络隔离**：无特权环境中切换 netns 或增加防火墙规则
  都需要 root，因此该性质依靠**名字**（`.invalid` + 跳板侧的中继表）实现 —— 用例自行解析一次并断言
  失败。它与"跳板机可见、本机不可见"在行为上等价，但不是同一件事。
- **多跳链（`jump_id` 指向的跳板自身还有跳板）未做端到端验证**：crate 级用例是**一跳**，E2E 也是
  **一跳**；链的顺序（目标在前 → 连接时反转）只有 store 的用例与代码在读取。真机上配置两跳跳板
  **未验证**。
- **SSH 测试服务端仍由本仓库自行搭建**（位于 `akasha-ssh::testing`）：它验证的是**客户端这条链**与
  app 的接线，不是与 OpenSSH 的互操作 —— 未连接过真实 `sshd`，也未连接过任何真实服务器。
  跳板这条路上还多一层：真实跳板机对 `direct-tcpip` 的策略（`AllowTcpForwarding`、`PermitOpen`、
  `originator` 相关的审计）**一条都未实测**。
- **解锁 / 锁定仍无界面**：命令、状态、生命周期均已具备（plan 0407），而界面上**没有可输入口令的
  位置** —— 因此在 SSH 的真实路径上，**解锁这一步由 E2E 以 `invoke_command` 完成**（界面只到
  "选择主机"为止）。⚠️ 不得将"已能建立 SSH 会话"理解为"用户从冷启动即可自行走通"：用户需要先有
  一个可输入库口令的位置。
- **主机池的增删改查仍无界面**：plan 0504 只增加了**只读**的 `vault_hosts`，plan 0505 使其额外携带
  `jumpId`。**配置一条跳板链目前只能直接写入库**（E2E 即如此构造数据）—— 界面上可见"经跳板 X"，
  但无法修改。
- **真实 agent 路径只覆盖了"不可用"**：agent 中确有密钥且服务端认可该密钥的路径从未运行。
- **并发提问未实测**：两条连接同时提问时，两条提示会**同时**显示在面板上（前端按 id 列表渲染），
  但只运行过"一条连接一次一问"。跳板链上的提问是**串行**的（四问按序），链越长该量级乘以跳数。
- **保活三个数值是默认值而非实测值**（ADR D15）：未构造"半开连接"与高延迟链路。
- **私钥与口令各有一份无法触及的明文副本**（D8 如实记录）：`russh` 需要 `ssh_key::PrivateKey`
  才能签名、需要 `String` 才能传递口令，两者都在**普通堆**上，我们无法擦除。
- **私钥候选的稳定标识只有行 id**：池的 `update` 会保留 id，因此"更换材料而未更换 id"会在**同一个
  解锁窗口内**留下一条过期口令（自愈机制已存在：解不开即 `forget` 并重新询问；缓存本身只存在于内存）。
- **提问占用一个 runtime worker**：这是 D16 如实记录的代价，由 4 个 worker + 120 s 超时兜底。
- **只读介质上的 v1 库未实测**：迁移需要写文件，该路径会以 `UpgradeFailed` 失败 —— 代码有此分支，
  但未构造真实只读文件系统验证。**降级同样未实测**（以旧二进制打开 v2 库）。
- **与 OpenSSH 的 `known_hosts` 互操作未实测**：`@cert-authority` / `@revoked` 这类标记行的行为
  未验证；用户文件中无法解析的行的处置（`warn` + 视为未知）没有用例守护。
- **解锁 / 导出 / 口令经 IPC 的边界**（同既往）：tauri 自身的两份口令副本无法触及（ADR-0002 §7.5）；
  `mlock` 失败路径只有单测；内存扫描仅在 Linux、仅扫描匿名段。
- **权限位、单实例、托盘在非 Linux 平台未验证**：CI 的类型检查无法覆盖运行期差异；CI 三个 job
  至今未运行（仓库没有 remote）。
- **前端类型检查不在任何门禁内**：`just ready` 只覆盖 Rust 与文档，`pnpm build`（tsc）需手动运行。
- **`just dev-web` 的模拟后端未在真实浏览器中操作过**：SSH 两条命令在其中**显式报错**
  （"没有 SSH 客户端"），因此主机选择器在浏览器中只会显示该提示。

## 当前基线（2026-09-15 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` / `akasha-store` / `akasha-ssh` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，问题 #29） |
| 后端模块 | `bindings` / `session` / `tray` / `config` / `lifecycle` / `single_instance` / `vault` / `watchdog` / `ssh`（长住状态 + 那条命令 + 跳板链 + 库内 known_hosts 适配器） / `prompt`（提问往返） / `pools`（池的读取 + **导入** + **转发规则**） / **`tunnel`（隧道实体 + 三条命令 + `tunnel_state` 事件 + `tunnels` probe）** / **`sftp`（SFTP 实体 + 两侧 + 九条命令 + `sftp` 探针）** |
| **命令清单** | `greet` · `vault_status` / `vault_unlock` / `vault_lock` · `vault_hosts` · **`vault_forwards`** · `import_ssh_config` · `open_session` · `open_ssh_session` · `write_session` / `resize_session` / `close_session` · `ssh_prompt_credential` / `ssh_prompt_host_key` / `ssh_prompt_cancel` · **`tunnel_open` / `tunnel_retry` / `tunnel_stop`** · **`sftp_open` / `sftp_connect` / `sftp_list` / `sftp_transfer` / `sftp_transfer_cancel` / `sftp_transfers` / `sftp_sides` / `sftp_sessions` / `sftp_close`**（事件：`session_ended` · `ssh_prompt` / `ssh_prompt_dismissed` · **`tunnel_state`**）。**plan 0601 新增四条命令**（三条驱动隧道 + 一条只读规则池）；`tunnel_open` / `tunnel_retry` 是 **async**（命令体里有一次会阻塞几秒的握手）。⚠️ **plan 0602 / 0603 / 0604 / 0605 都没有新增命令与事件**（三条转发走的是同样那三条命令 + 同一份事件 + 同一份 probe），只改了它们的内部与失败分档；plan 0605 新增的是一份配置（`Config::reconnect`）与一条 `tray` probe；**plan 0606 同样没有新增命令与事件**，只加了一条只读探针 `residue`（见下）。⚠️ **plan 0701 新增六条命令、零个事件**（`sftp_*` 四条驱动 + 两条只读）：SFTP 的连接结果由**命令返回**（失败落在那一侧），诊断走 `sftp` 探针 —— 不给它加事件是因为"两侧各自的状态"本来就只在用户按下连接之后才变。⚠️ **plan 0702 又加三条**（一条起传输 + 一条取消 + 一条只读），**仍然零个事件**：传输的进度与结局由 `sftp_transfers` 与探针读（`sftp_transfer` 同步：它排完任务就返回）。`sftp_close` 因此变成 **async**（它要等清理落地）；`sftp_connect` 的第二个参数由 `hostId` 变成 `origin`。⚠️ **plan 0703 一条命令、一个事件都没加**：两档是"同一对端点命令、目标那个端点怎么来的"，所以变的只有三个字段（`through` / `throughFailure` / `via`）与删掉一档错误。⚠️ **plan 0704 同样一条命令、一个事件都没加**：并发上限是引擎内部的策略，契约只多了 `SftpSummary.inFlight.{limit, live, peak}` 这一组读数（`sftp_open` 多收一个 `AppHandle`，签名不变） |
| **probe** | `lifecycle` → `{close_behavior, tray_ready, close_action}`（没登记时 `{"initialized":false}`，问题 #93）；`single_instance` → `{registered, activations}`；`sessions` → `{live, registered}`（SSH 与隧道都没有本地进程，"零残留"只能看注册表）；**`tunnels` → `[{handle, ruleId, name, state, attempt, bind}]`**（与托盘菜单同一份数据；`bind` = 实际监听地址 —— plan 0604 起对 `remote` 规则它说的是**服务端**那一侧的地址，`port = 0` 时是服务端挑的那个）；**`tray` → `{ready, tunnels:[…]}`**（plan 0605：`tunnels` 是**托盘菜单上那几行文字**，与菜单共用 `tunnel_labels`；`ready` = 这台机器上托盘建成没有）；**`residue` → `{sshConnections, watchTasks}`**（plan 0606：`SshConnection` 的存活计数与看护任务的存活计数，判据"两个计数都归零"的读数口 —— 数的是**资源本身**，不是注册表里的实体）；**`sftp` → `[{handle, sides:[{side, origin, name, state, failure, path, through, throughFailure}], transfers:[{id, from, fromPath, to, toPath, state, done, total, failure, via}]}]`**（plan 0701：一个 SFTP 会话一条记录，两侧的状态、失败原因与当前目录都在里面；plan 0702 起每条还带上**这个会话发起过的传输**与它们的进度 / 结局；plan 0703 起两侧还带上**到达方式**（`through` = 经哪台直通、`throughFailure` = 回退原因），传输带上 `via`；plan 0704 起每个会话还带一组并发读数 `inFlight:{limit, live, peak}` —— 判据"两侧各自列目录成功"、"中断之后没有半成品"、"这次走的是哪一档"与"上限真的在起作用"的读数口）。⚠️ **排队中的传输在传输记录里与"正在搬"长得一样**（状态只有"还没结束"这一档），分辨它们靠 `live` 比"还没结束的条数"少。**库没有 probe**：状态本身就是命令（`vault_status`） |
| 出字节路径 | PTY / SSH read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write`。**两条载体共用同一段输出路径的后半段**（`session::open_terminal`） |
| **隧道实体**（plan 0601 / 0602 / 0603 / 0604 / 0606） | `src-tauri/src/tunnel.rs`：`Tunnel { id, rule_id, rule_name, host_id, state, attempts, forward: Option<ActiveForward>, stop: TunnelStop }`，登记进 `Sessions` 的**同一张注册表**（`Inner.tunnels`，与 `live` 同一把锁；`len()` = 两者之和，与 `registered()` 相等）。`forward` 是**那条转发**（它持有连接；`None` = 还没连上 / 已经断开），`stop` 是**这条隧道的停止信号**（plan 0606：实体一登记就有，在途的尝试与看护循环各订一份接收端 —— 0605 那个可替换的 `oneshot` 会在替换时误唤醒 `select!`，见问题 #143）—— 两者分开正是因为"转发结束了"这件事归看护任务等，而转发本体归实体表（停止与 probe 要它）。`Rule::prepare()` 把方向翻成 `Prepared::Local`（`-L` 的 `Ingress::Fixed` / `-D` 的 `Ingress::Socks5`，**本机端口已经绑好**）/ `Prepared::Remote`（`-R`：服务端的绑定地址 + 本机目标）。`tunnel_open` 失败分两种：**没登记成**（`Err`：库锁着 / 规则不在池里 / 规则那一行坏 / **本机端口没拿到** / **地址不许绑**）与**登记了但连不上**（`Ok(TunnelAttempt { handle, failure })` —— 那条仍在册、可重试；`-R` 的**远端**端口没拿到属于这一种，因为那时连接已经建起来了）。`tunnel_stop` 先发 `已停止` 再注销注册，**幂等**；收尾只有一条路 —— `Tunnel::reclaim()`（停信号 → 丢转发），`tunnel_stop` 与 `shutdown_all` 都走它（plan 0606 之前 `shutdown_all` 靠丢掉实体、让字段各自在 drop 时收尾） |
| **`direct-tcpip` 原语**（plan 0505，ADR-0003 **D9**） | `akasha-ssh/src/forward.rs`：`SshStream`（自己实现 `AsyncRead + AsyncWrite`，**不把 `russh::ChannelStream` 漏进公开签名**）+ `SshConnection`（已认证、**没有通道**的连接，持有 `Handle` **与它自己的跳板链** `under`）+ `SshConnection::direct_tcpip(host, port)`。三处消费者（跳板 / 转发 / SFTP B 档）使用的都是**这条流**，三处**都接上了**（B 档见 plan 0703：`src-tauri/src/ssh.rs` 的 `connect_connection_via` 把"[A 的跳板链] ++ [A] ++ [B 的跳板链]"交给 `connect_via_until`，原语本身一行未改）。plan 0601 给它加了同步门面 `connect_via`，并把"逐跳搭链"抽成 `hops_chain`（**建链只有一份实现**，`SshTransport` 与它共用）。⚠️ `SshConnection::over` **不持有**承载它的那条连接（它只造下一跳）—— 链的存活归调用方，`chain` / `connect_via` 会把整条链放进 `under` |
| **跳板链**（plan 0505） | 库侧：`hosts::jump_chain`（**目标在前**、有界、成环报 `StoreError::JumpChain`）。app 侧：`ssh.rs::plan_chain` 按 id 解出各跳，`open_ssh_session` 再将其反转为"最外层在前"后调用 `SshTransport::connect_via(runtime, hops, target)`（`connect` 即空链的那一次）。**每一跳各一份 `SshConnect`**（各自询问凭据、各自校验主机密钥）；链上每一跳是一个 `SshConnection`，随 `Established::carriers` **move 进最终那条连接的 `pump` task** —— "task 结束 = 整条链结束"，收尾按**最内层先断** |
| **连接的 originator** | `direct-tcpip` 要求带发起方地址（RFC 4254 §7.2）：用**最外层那条 TCP 的本地地址**（我们唯一真知道的），往下每一跳复用；拿不到就空串 + 0（不得伪造一个看似真实的地址写入对端日志） |
| **SSH 的 IPC 层**（plan 0504） | `src-tauri/src/ssh.rs`：app 启动时建**一个**专用 tokio runtime（**4 个 worker**，D2）；`open_ssh_session` 是 **async 命令**（不阻塞 IPC），内部起一条**普通 `std::thread`** 运行同步门面（`spawn_blocking` 的线程**也算** tokio 上下文，会触发 `BlockingInsideRuntime`），结果经 `tokio::sync::oneshot` 回来。`SshConnect` 的材料按池行组：`password` → 不用 agent、不带钥匙；`agent` → 只用 agent；`publickey` + `key_id` → 那一把钥匙（PEM → 受保护页 → `KeyCandidate`，标识 `key#<id>`） |
| **提问往返**（plan 0504，ADR-0003 **D16**） | `src-tauri/src/prompt.rs`：`Prompts`（`Arc` + 待答表 + 可注入的发布口）；事件 `ssh_prompt`（判别式：`hostKey` / `credential`）+ `ssh_prompt_dismissed`；三条回答命令；编号从 1 起、只增不减；**超时 120 s → 拒绝 + 撤回**；答过 / 超时的 id → `PromptError::Gone`；**主机密钥那一问只认"接受 / 拒绝"**，超时 / 取消 / 答错类型一律 `Err(HostKeyUnknown)`（= 拒绝连接）。⚠️ 跳板链上**每一跳各产生一轮**（密钥 + 口令），E2E 实测四问按序 |
| **库内主机密钥缓存**（plan 0504 接线） | `VaultHostKeys`：`Vault` 可 `Arc` 克隆，`with_conn` **短借**连接；库处于锁定状态 → `SshError::HostKeyCache` → **拒绝连接**（不视为未知）。⚠️ **不得在持锁期间连接**：`remember` 会在连接中途回锁库（跳板链因此先**一次读完整条链**再开始连接） |
| **库的解锁状态** | `Vault { inner: Arc<Mutex<Option<Unlocked>>> }`（`Clone`）；`Unlocked { conn, passphrase }` 同生共死。借库的失败分两种（`ConnError`：`Locked` / `Store`）—— 因为 SSH 那条路要单独认出 `NoSuchRow`（"所选主机不存在"） |
| **`akasha-ssh` 的形状** | 十八个模块：`target` / `credential` / `keys` / **`ending`（plan 0605：一次转发怎么结束的 —— `ForwardEnd` + `ForwardEnding`，转发本体与它的结束通知**分两半**交出去）** / `handshake`（`handshake<S>` = 一跳的握手 + 认证，**底层流由调用方提供**） / `known_hosts` / **`forward`（D9 原语 + `SshConnection` + `hops_chain` + 同步门面 `connect_via` / `connect_via_until`（带取消信号，plan 0606）+ `is_closed` + 连接存活计数 `live_connections`）** / **`relay`（`-L` 与 `-D`：`LocalListener` + `LocalForward` + `ForwardTarget` + `Ingress`）** / **`socks5`（动态转发的协议本体：无认证的 `CONNECT` + `Reply` + 回环限制）** / **`remote`（`-R`：`RemoteForward` + 入站路由 `Inbound`）** / **`sftp`（plan 0701 / 0702：`SftpClient` —— 一条流上的 SFTP 会话，`list()` 先 `canonicalize` 再 `read_dir`，并实现 `Endpoint` 的读写路径）** / **`local`（plan 0702：`LocalEndpoint` —— 本机文件系统作为另一个端点，无字段、`default_dir()` 三层兜底）** / **`in_flight`（plan 0704：并发上限 —— `InFlight` + 它的读数 + 有界入口）** / **`transfer`（plan 0702：引擎与它的词汇 —— `Endpoint` / `PendingWrite` / `Cancel` / `Progress` / `Entry` / `Listing` / `temp_candidates`）** / `transport` / `testing`（进程内测试服务端，**仅用于测试**；支持 `direct-tcpip` 的中继与拒绝两条分支、`tcpip-forward` / `cancel-tcpip-forward` / `forwarded-tcpip`、**`sftp` 子系统**（plan 0701：`ServerOptions.sftp` 给出根目录的条目），并记**连接级**的断开数 `connections_closed`、**子系统认下数** `sftp_subsystems` 与 plan 0704 起的两件事（客户端 `open` 过的路径、`stat` 过的路径 —— 「临时名不再靠探测」这条判据的读数口）；plan 0704 还多了一个 `slow_link`（到某个地址的**带时延链路**：每段字节各延后一段固定的时间，用来把一次往返放大到可量的量级）；plan 0605 起还能**切断已建立的连接**（`cut_connections` / `shutdown`），远端监听**按连接持有**、随连接消失） / `auth` / `error` |
| **错误分域** | `akasha-ssh`：`HostKeyCache`（库那一侧无法读取缓存）、**`Forward { host, port, reason, class }`**（跳板拒绝 / 目标不可达 —— 与"无法连接跳板机"分开；`class` 是 `ForwardFailure`，取自上游结构化的 `ChannelOpenFailure`，plan 0603 起供 SOCKS5 的 `REP` 分类用）、**`Listen { address, reason }`**（本机端口没拿到，plan 0602 —— 与"对端连不上"分开）、**`Sftp { target, reason }`**（SFTP 会话建不起来 / 用不了，plan 0701 —— 与"这条 shell 通道开不出来"分开：用户要看的是**对端有没有开 SFTP**）、**`File { path, reason }`**（plan 0702：**端点上的一个文件操作**失败 —— 与 `Sftp` 分开是因为它要引到**那个路径**上，本机与远端共用这一档）、**`RemoteListen { address, reason }`**（服务端那个端口没拿到，plan 0604 —— 与"本机端口没拿到"分开：用户要动的地方在服务端）、**`NotLoopback { address }`**（SOCKS5 绑了非回环地址，plan 0603 —— 与"端口没拿到"分开：换端口没有用）、**`Cancelled`**（建链被取消信号中止，plan 0606 —— 这不是失败，见 `connect_via_until`）。app 侧 `SshIpcError`：`Locked` / `NoSuchHost` / `Failed { kind, message }`（`kind` = `hostKeyChanged` / `hostKeyRejected` / `hostKeyUnknown` / `hostKeyCache` / `auth` / `connect` / **`jump`** / `other`）/ `Internal` —— **前端按 `kind` 分辨**，不匹配消息字符串。**隧道另有 `TunnelError`**（`locked` / `noSuchForward` / `noSuchHost` / `notATunnel` / **`bind`（本机端口没拿到）** / **`remoteBind`（服务端那个端口没拿到）** / **`notLoopback`（地址不许绑）** / `failed {kind,message}` / `transition` / `internal`），连接失败那一档复用同一个 `SshFailureKind` |
| **前端结构** | `src/ipc/`（`session.ts` / `prompts.ts` / `hosts.ts` / **`tunnels.ts`** / **`sftp.ts`** —— 唯一允许碰后端的目录）、`src/tabs/`、`src/terminal/`、`src/ssh/`（主机选择器 + 导入面板 + 提示面板）、**`src/tunnels/`（隧道面板）**、**`src/sftp/`（SFTP 双栏面板：一栏可以选本机，带传输列表与取消）**、`src/App.tsx`。标签页 `kind`：`terminal` / `ssh`（**都有关闭按钮**，规则写成 `CLOSABLE` 清单）—— **隧道与 SFTP 都不是标签页**：它们是应用级浮层，关面板不停任何会话 |
| **前端的一个 dev-only 陷阱** | React StrictMode（仅开发模式）会把 effect 执行两遍，SSH 会话因此被建立两次。处置：SSH 那条连接**无条件推迟一个微任务**再发起（本地 PTY 不受影响；问题 #118） |
| **SSH 栈**（ADR-0003） | `russh = "=0.63.3"`、features `["ring","rsa"]`；`akasha-ssh` 只收 `tokio::runtime::Handle`；对外是同步 `Transport` 门面 + 两条**有界** mpsc（满 → `TransportError::Busy`）；capability = `resize + exit_status`、`session_leader() = None` |
| **连接取值**（D15 + 0505 的修正） | `connect_timeout = 10s`；`keepalive_interval = Some(30s)`、`keepalive_max = 3`；**Nagle 关闭**（`tcp_stream` 里显式 `set_nodelay(true)`，问题 #120）。⚠️ 那三个数是**有理由的默认值**，不是实测出来的 |
| **库格式与迁移** | `FORMAT_VERSION = 2`；v1 = 四张池表（`DDL_V1` 冻结、公开），v2 = v1 + `known_hosts`。`open` 里 `upgrade()`：`== 2` 什么都不做；`1` → 先按 v1 校验形状 → **一次事务**里加表 + 写版本号；`> 2` 与 `0` 拒绝；写入失败 → `UpgradeFailed`。⚠️ **不支持降级** |
| **库文件的磁盘事实** | `akasha.db`；建库后 **36864 字节 = 9 页**；SQLCipher 4.5.7 + vendored OpenSSL 3.6.3 + 内嵌 SQLite **3.46**；`user_version = 2` 是格式权威；盐 16 字节随机；显式收紧到 **600**；不带 `-wal` / `-shm`；解锁代价 **~105 ms**（KDF） |
| **四类池** | `keys` / `hosts` / `serials` / `forwards`，各 5 个函数 + 反查（`hosts` 另有 `jump_chain`）。`New*`（没有 id）与 `*`（有 id）**是两种类型**；不变量由库强制（`STRICT` + `CHECK` + 外键 `RESTRICT`，D14） |
| **`~/.ssh/config` 导入**（plan 0506，ADR-0003 **D14**） | 解析器在 `akasha-store/src/sshconfig.rs`，**纯函数** `parse(text, default_user)`（不读盘 / 不读环境 / 不碰库）；落库在 `pools/import.rs` —— **一个事务**，先全按 `jump_id = NULL` 插入、再用 `update_host` 挂链（于是成环检查**只有一份实现**：`insert_host` 刻意不做的那份，而导入是第一条"一次插入多行、这些行互相引用"的路径）。三档边界见 ADR-0003 D14；名单与判据只有 `classify` 一处 |
| **导入的三条产品口径** | ① **同名默认不动**（`overwrite` 才整行替换）；② **为跳板补建的条目永不覆盖**用户写的行；③ `IdentityFile` 只让条目落成 `publickey` + `key_id = null`（**私钥不导入**，连接时走 agent），报告里逐条说明 |
| **known_hosts 缓存** | 表 `known_hosts(id, host, port, key_type, key_blob, fingerprint)`，`UNIQUE (host, port, key_type)`。**缓存不是池**；判定材料是 `key_blob`（逐字节比）；`remember` 遇到同键不同值 → `Conflict`（**写路径上就不许静默改写**） |
| **主机密钥的三态判定** | `KnownHostsVerifier`：**库 → 用户的 `~/.ssh/known_hosts`（只读）→ 提问**。库里 / 文件里对不上 → `HostKeyChanged`（**不看不问**）；两边都没有 → 有 `HostKeyPrompt` 就问、确认后 `remember`。两个注入点是**同步** trait |
| **口令与私钥** | `Passphrase` / `PrivateKey`：空值**无法构造**、**没有 `Debug`**、本体位于 `memsafe` 的受保护页；`PassphraseInput` 是口令**经 IPC 进来的唯一形态**（vault 解锁与 SSH 凭据**共用**它） |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| CSP | `default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| capabilities | 仍只有 `core:default` + `opener:default`（+测试用的 `victauri`）。**SSH、托盘、配置、单实例、便携目录检查都没有加任何 permission**（全在 Rust 侧） |
| 前提条件 | **需要能写 `$HOME`**；托盘另需能写 `$XDG_RUNTIME_DIR`、单实例另需会话总线（否则各自只降级）。**便携目录存在时另需可写 —— 不可写是拒绝启动**（退出码 2） |

## 进行中 / 下一步

- [x] **阶段 11 的三条全部完成**（plan 1101 / 1102 / 1103，2026-09-15）：串口能从界面打开、
  字节双向流动、界面上三条输入并列、取值越界当场可见；设备消失时以可读原因结束、
  标签页随之关闭、没有残留注册。ROADMAP 的「阶段 11」下面没有别的条目 —— 这一阶段到此收口。
  ⚠️ 判据里"没有残留注册"的读数口是 `app_state { probe: "sessions" }`（串口没有本地进程）。
- [ ] **下一步 = 阶段 9 的第一条**（`bw` 对 `sshKey` 条目的非交互行为实测）：
  [`0901`](./plans/0901-bw-noninteractive-probe.md) 仍是骨架，但它**不依赖任何代码** ——
  骨架里写的"需要真实 vault"是它唯一的门槛。
- [ ] **Windows 目标的编译被 `akasha-pty` 挡住**（问题 #149）：修它是计划级的活
  （Windows 上没有 POSIX 进程组语义）。它同时压着阶段 8 那半句"Windows / macOS 原生编译"。
- [ ] **并发分档曲线里"上限 2 与串行一样慢"那一段没有定位**（plan 0704）：12 个 1 KiB 文件
  在带时延链路上，上限 1 = 1.17 s、2 = 1.16 s、4 = 0.56 s、8 = 0.38 s、16 = 0.21 s；
  上限 2 时两条传输**几乎同时结束**（166 ms / 207 ms），而各自都比单独执行时（96 ms）慢一倍，
  同时服务端记到的两次 `open` 只差 0.08 ms（请求确实是并发发出去的）。判据不受影响
  （并发确实显著更快），但"往返折叠"在低并发下没有按预期发生。默认值因此不按饱和点取。
- [ ] **并发上限也只在自建的测试服务端上量过**（plan 0704）：真实 `sshd` 的 `MaxSessions`、
  打开句柄上限与 `limits@openssh.com` 声明的 `write_len` 都会改变"一条连接能同时扛几个文件"。
  上限还没有文件形态 —— `config.json` 仍只认 `close_behavior`，默认值之外没有别的取值方式。
- [ ] **host ↔ host 的两档都没有与真实 `sshd` 对照过**（plan 0703）：B 档（`direct_tcpip` +
  隧道里的 SFTP）与 A 档都只在**自建的测试服务端**上验过。真实的 `AllowTcpForwarding no`、
  `PermitOpen` 白名单与转发的缓冲特性都还没有对照物 —— 它们改变的是**失败长什么样**，
  而回退的触发点（建链失败即回退）不依赖具体是哪一种。
- [ ] **耗尽之后说不清为什么放弃**：`tunnel_state` 的载荷只有 `{handle, state, attempt}`，原因只在
  日志里（plan 0605 的非目标 —— 给它加字段就是改一份从 plan 0601 起就已经发出的契约）。用户看到的
  是「失败」，看不到"是认证不对、还是网络不通"。
- [ ] **半死连接的发现延迟是保活量级**（问题 #141）：真实的拔网线场景下，"进「重连中」"之前会有
  最长约 90 秒的"看起来还连着"。要缩短就得调 D15 那三个数 —— 而它们本身也不是实测值。
- [ ] **`-D` 的开放到同网段仍不可选**：无认证的 SOCKS5 一律只许绑回环（plan 0603 的安全项）。
  若将来要支持，需要的是**另一轮明确同意**（D16 的往返只覆盖凭据）—— 尚未规划。
- [ ] **`-R` 的远端绑定地址没有任何检查**：规则里写什么就向服务端请求什么（plan 0604 的
  有意取舍：那个端口开在服务端，合规与否是它的策略）。⚠️ 与上一条的 SOCKS5 限制**不是**
  同一条口径，不得相互搬用。
- [ ] **阶段 5 之后仍有两处界面缺口**（不是缺陷，而是尚未规划的工作）：**解锁界面**（当前 SSH 的
  真实路径上，解锁由 E2E 以 `invoke_command` 完成）与**主机池的增删改查界面**。⚠️ 目前一台机器的
  端口 / 用户名 / 跳板在界面上**无法修改**（只能改配置文件后重新导入并指定覆盖，或直接改库）。
- [ ] **三个"只在模型里、不在文件里"的默认值**：`Config::reconnect`（3 次 / 1s / 2 倍）与
  `Transfer::in_flight`（8）都有默认值，而 `config.json` 仍只认 `close_behavior` ——
  给一个嵌套对象定文件格式要连界面一起设计（plan 0605 / 0704 的非目标）。
  ⚠️ 现状下"把退避改小"或"把并发上限调大"只能改代码。
- [ ] **降级路径未实测**：v2 库在旧版本程序中会以 `UnsupportedVersion { found: 2 }` 被拒绝（有意为之）。
- [~] **plan 0102（CI 平台矩阵）**：本地部分完成，最终判据 = 推送后三个 job 全部通过，当前阻塞于仓库无 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测，剩余 CI 三平台格子
- [ ] **正式 UI**：等待设计稿（见上文「UI 现状」）—— 没有验收标准，因此**不进入 ROADMAP**

### 各轮（已完成，plan 0603–0802）

逐轮的步骤与实施记录随各自的 plan 归档（索引见 [`plans/README.md`](./plans/README.md)），
**本节只留之后会被引用的事实**：各轮的判据与实测。

| plan | 判据（ROADMAP 原文） | 实测 |
|---|---|---|
| 1103 | 把测试用的 PTY 主端关闭（等价于拔掉设备）时会话结束、没有残留注册 | `serial_device_gone` E2E **0.36 s**：拔掉设备（关闭唯一一份主端）→ 从端那一路的 `read` 报 `BrokenPipe`（**不是** `Ok(0)`）→ 会话自己结束、标签页跟着关；通知行里是 `「/dev/pts/0」已结束：串口设备已断开：/dev/pts/0（Broken pipe）`；`sessions` 回到 `{"live":1,"registered":1}`。原因经 `Transport::stream_error` → `SessionEnded.status` 进界面（契约形状未变），crate 层 1 条 + app 层 1 条 + `akasha-pty` 1 条。⚠️ 真设备被拔掉时走的是 EIO 那一支，未实测。见「阶段 11 的形状」 |
| 1102 | 界面上看到本机枚举结果并据此（或手输路径）打开；取值越界时显示字段与取值 | `serial_ports_ui` E2E **1.46 s**：后端枚举 32 条 / 界面上那个计数与它相等；点第一条把路径填进那一栏；手输 PTY 从端的路径打开并双向传字节；数据位填 9 → 那个面的报错行里是 `data_bits = 9`（面已开出来、当场是出错态）而关闭之后注册表回到打开前；填 `abc` → 面板自己拦下、**不开面**。⚠️ "点一条枚举结果"在枚举为空的机器上**显式跳过**（CI 的 runner 就是这一类）。见「阶段 11 的形状」 |
| 1101 | 真实 app 上打开一个串口会话并双向传字节；关闭标签页后 `live` / `registered` 归零 | `serial_session` E2E **1.00 s**：设备是测试进程自己造的一对 PTY 的从端；界面点"串口"→ 选池里那一行 → 标签页"已连接"；**两个方向各一条独立证据**（主端写的 `from-device-1101` 出现在屏幕上 / 标签页里敲的 `to-device-1101` 被主端读到）；关闭标签页后 `sessions` 回到 `{"live":1,"registered":1}`。⚠️ 实测发现一处问题：`serialport` 默认**独占**打开，而 StrictMode 的两次挂载让被丢弃的那次真的占了设备（第二遍收到 `EBUSY`）—— 处置是把"推迟一个微任务再开"从 SSH 扩到串口。见「阶段 11 的形状」 |
| 0802 | 枚举在本机列出真实端口；参数错误时给出可读报错 | `ports()` 在本机（libudev）列 **32 条**且顺序稳定无重复、路径非空；同一套用例在 sysfs 那套实现下返回 **0 条**且照常通过（"没有端口"与"枚举失败"因此是两种结果）；真 tty 上回读参数、越界取值报字段与取值。⚠️ 负例自检：去掉排序只红了手造输入的三条单测 —— **只在真实机器上执行的断言可能是永真的** |
| 0801 | Windows / macOS 构建不链接 libudev | 依赖图核对（`just libudev-check`，两条负例验过）+ 一次性探针让 `serialport` 带该 feature 在三个目标上各编译一次；13 条用例全过（PTY 那 4 条带 `--nocapture` 重新执行过，确认没走跳过分支）。⚠️ 原生编译待问题 #149 |
| 0704 | 大量小文件的吞吐显著优于串行请求 | 上限归**会话持有的** `InFlight`（`limit` / `live` / `peak`；等空位可取消），临时名改成**一次**原子占用（本机 `create_new`、远端 `CREATE|EXCLUDE`）；库内 6 条（7 个文件 / 上限 3 → 目标端点同时只见 3 个 `begin_write`）+ `sftp_pipelining` E2E：12 个 1 KiB 文件在每方向延后 10 ms 的链路上 **1.509 s → 389 ms**（`peak 8`） |
| 0703 | A 无法直连 B 时自动走 A 档；两档均不落盘 | `sftp_host_to_host` E2E **5.29 s**：B 档经跳板直通（跳板记到 1 条 `direct-tcpip`、中继搬了 55776 字节、目标真盘字节相同且没有临时名、源那台一个条目都没多）；直通被拒（`AdministrativelyProhibited`）→ 自动回退本机直连、原因留在 `throughFailure`；第二个文件照样落地而 `via` 为空；库内 3 条含**两半负控** |
| 0702 | 中断传输后目标目录里没有看似完整的文件 | `sftp_transfer_atomic` E2E **6.19 s**：取消前搬了 **2883441 / 4194304** 字节、目标目录实测 `[".big.bin.part"]` → 取消后 **0 个条目**；上传的 300 KiB 在对端真盘上与源逐字节相同；库内 6 条（两个可控假端点，连续执行 8 次全绿） |
| 0701 | 直接打开 SFTP 即可用，无终端依赖 | `sftp_dual_pane` E2E **7.34 s**：不新建任何终端标签页；两栏各连一台服务端、各自列目录，左栏没有右栏的条目；两台服务端各记到 1 次 `sftp` 子系统请求；`live`/`registered` **1 → 2 → 1** |
| 0606 | 关闭转发 `Session` 后连接数与重连任务数都归零 | `tunnel_teardown` E2E **5.48 s**：两条隧道 + `residue = (2, 2)` → 关闭 `-R` 得 `(1, 1)` 且服务端那个端口连不上 → 关闭 `-L` 得 `(0, 0)`；再停一次是 `Ok` 且 1.5 s 后仍是 `(0, 0)`；在途的握手被中止时对端 **7.5 ms** 内读到 EOF |
| 0605 | 拔网线后进入"重连中"，耗尽次数后变"失败"**且托盘可见**；可手动重试 | `tunnel_reconnect` E2E **19.19 s**：服务端断开 → `reconnecting(1)` → 自行回到 `connected`；服务端消失 → `reconnecting(1)(2)(3) → failed`（实测 **7.55 s**，预算 ≥ **7 s**）；`-R` 重连**重新请求**监听且端口不变；重连途中停止能中止循环 |
| 0604 | 远端监听端口可回连到本机服务 | `tunnel_remote_forward` E2E **7.01 s**：`curl http://127.0.0.1:<那个端口>/probe` 退出码 0 且响应体是本机 HTTP 服务写的那一串；服务端记到 `tcpip_forward`；本机目标不可达 → 它看到 `ConnectFailed` 且通道**没有**被接受 |
| 0603 | 配置 SOCKS5 代理后能访问远端网络 | `tunnel_dynamic_forward` E2E **0.85 s**：`curl --socks5-hostname` 退出码 0 且取回远端服务的响应体；非回环绑定在**绑定之前**被拒（那条监听从未建起来） |

未成文但**之后会被引用**的落地事实（各条都由上面的用例或 crate 用例钉住）：

- `-R` 的入站通道**按端口认**（服务端回报的 `connected_address` 是它认为在听的地址，
  按地址字符串认会在真实服务端上静默失配）；没登记过的端口一律拒绝。
- 停止信号是**实体自己持有的一对 `watch`**（不是可替换的 `oneshot`）—— 换发送端会唤醒接收端，
  让 `select!` 在两个分支同时就绪时**随机挑**，一次**成功**的连接因此约有一半机会被报成"已停止"。
- 建链入口带取消信号（`connect_via_until`）：建立连接发生在阻塞线程上，丢掉 `await` 取消不了它。
- SOCKS5 只允许绑回环（无认证 + 不可选）；`-R` 的绑定地址**不做**回环限制（那个端口开在服务端）。
- 失败分档按"哪一层坏了"分（`retryable`）：传输层与端口没拿到会重试，认证 / 主机密钥 /
  配置与内部状态**不重试**；`tunnel_retry` 只认终态，且这条检查排在绑定之前。

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接运行 `cargo …` 会失败（问题 #8），一律通过 `just` 转发。⚠️ **临时脚本同样适用**。
- **六个 crate 的职责**：`akasha-core`（Session 模型 + 配置模型与判据 + **隧道状态机**，
  **零 Tauri 依赖**）、
  `akasha-pty`（`Transport` + portable-pty + 合批 + `teardown` + `watchdog`）、
  `akasha-serial`（串口：`Transport` 的实现 + `ports()` 枚举 + 参数校验 + **设备消失时的原因**；`serialport` 的 `libudev` 只在 Linux；零 Tauri 依赖）、
  `akasha-store`（库的打开 / 创建 / **格式版本与迁移** / 四类池（含 `jump_chain`）/
  **known_hosts 缓存** / dump / 导出与还原 —— 唯一允许 `unsafe` 的 crate）、
  `akasha-ssh`（连接 + 认证 + known_hosts 三态 + **D9 原语与跳板** + 同步 `Transport` 门面 +
  `testing`（进程内服务端））、
  `akasha`（app 包 = IPC 薄壳 + 托盘 + 配置 + 数据目录 + 窗口关闭语义 + 单实例 + 退出钩子 +
  看门狗接线 + 库的解锁状态 + SSH 的 runtime / 提问往返 / 池读取 / **跳板链** + 代码生成 bin）。
- **前端**：`src/ipc/`（唯一允许调用后端的目录）、`src/tabs/`、`src/terminal/`、`src/ssh/`、
  `src/tunnels/`、`src/sftp/`、`src/serial/`、`src/App.tsx`。
- **排查"SSH 为何连不上"**：日志中 `ssh session opening`（带 `hops` = 跳数）/
  `ssh authenticated`（带 `method`）/ **`ssh direct-tcpip opening`（带 `via` = 经过的主机）**；
  主机密钥一档见 `ssh host key accepted` / `rejected` / `unusable`；提问是否有人应答见
  `prompt has no publisher`（未装载发布口时会立即超时）。
- **改 SFTP 之前先看**：`src-tauri/src/sftp.rs`（`Sftp` 实体、两侧、传输的记账与九条命令）→
  `src-tauri/src/session.rs` 的 `sftps`（**第三张表**，与 `live` / `tunnels` 同一把锁）→
  `akasha-ssh/src/transfer.rs`（**引擎**：只认两个端点，落盘不变量与取消都在这里）→
  `akasha-ssh/src/local.rs` 与 `akasha-ssh/src/sftp.rs`（两个端点各自怎么落盘）。
  ⚠️ **关面板不停会话**：结束会话是 `sftp_close`（面板里的显式动作），所以面板打开时会先
  `sftp_sessions` 接回已有会话。⚠️ **`sftp_close` 的顺序不能换**（先中止并等清理、再断连接）：
  临时文件是用那条连接删的。两侧的状态、传输的进度与失败原因**只有后端一个来源**：
  `app_state { probe: "sftp" }`。
- **排查"传输留下的临时文件"**：目标目录里出现 `.name.part` 说明清理没走完 ——
  先看 `app_state { probe: "sftp" }` 里那条传输的 `state`。⚠️ **`state` 还是 `running` 就是
  还没收尾**（取消只推信号，清理是搬字节那条任务做的）；`state = cancelled` 而临时名还在，
  才是真的漏了。⚠️ 一次传输一条任务，`sftp_close` 会**等它们结束**，所以关会话之后
  不该再有 `.part`。
- **排查"某个会话是否仍在"**：`app_state { probe: "sessions" }`（`live` 与 `registered`
  **必须相等**，分叉说明存在"可查询、但无人管理"的会话）。
- **改隧道之前先看**：`akasha-core/src/tunnel.rs`（五态与转移表，**纯逻辑**）→
  `src-tauri/src/tunnel.rs`（实体、三条命令、`tunnel_state` 事件、`tunnels` probe）→
  `akasha-ssh/src/forward.rs` 的 `SshConnection`（⚠️ **它没有通道**）→
  `akasha-ssh/src/relay.rs`（`-L` 与 `-D` 的本地监听与搬运；两者的差别是 `Ingress`）→
  `akasha-ssh/src/socks5.rs`（`-D` 的协议本体：无认证的 `CONNECT` 与 `REP` 分类）→
  `akasha-ssh/src/remote.rs`（`-R`：请服务端监听 + 服务端发起的通道；⚠️ **不复用** D9 的原语）。
  状态名与事件名是**契约**（ADR-0003 §10 第 4 条）：改名要同时改 `bindings.ts`、前端与托盘。
- **改 serial 之前先看**：`akasha-serial/src/enumerate.rs`（枚举；⚠️ 空表是正常结果、列出来的端口
  **不保证能打开** —— 问题 #150）→ `transport.rs`（打开、读循环、**设备消失时那句话**）→
  `settings.rs` / `error.rs`（取值域与四组失败）→ **app 侧的接线** `src-tauri/src/serial.rs`
  （`vault_serials` / `serial_ports` / `open_serial_session` 三条命令 + 三个 IPC 枚举的映射 +
  `SerialIpcError` 的四档）→ `src/serial/SerialPicker.tsx`（三条并列的输入：枚举结果 / 手输路径 / 池行）。
  读数口：`just serial-check`（两种 feature 配置各执行一遍）与 `just libudev-check`；
  会话侧只有 `app_state { probe: "sessions" }`（串口没有本地进程）。
  排查"串口标签页为何消失"：日志里 `session retired` 带 `err=串口设备已断开…`
  （没有退出码的载体走这一支；有结局的载体报 `exit_code` / `signal`），界面上那句话在
  `[data-session-notice]`。
  ⚠️ 打开是**独占**的（`TIOCEXCL`）：第二个会话会得到 `EBUSY`——两个进程抢一个串口本来就不该成功，
  不得为了绕开它去关独占（plan 1101 的实施记录）。
- **新增 command / event 的三处**：`src-tauri/src/bindings.rs` 登记、`just gen-types` 重新生成、
  `just gen-types-check` 比对（`AGENTS.md` §5）；事件还必须在 `.setup()` 里 `mount_events`。
- **排查"隧道为何没连上 / 为何转发不通"**：日志里 `ssh connection opening`（带 `hops` = 跳数）与
  `tunnel state changed`（带 `state`；`重连中` 时还带 `attempt`）；**端口没拿到**是 `SshError::Listen`
  那条（在握手之前，因此日志里没有 `ssh connection opening`）；通道开不出来是 `local forward channel failed`；
  当前状态的**唯一事实**是 `app_state { probe: "tunnels" }`（托盘子菜单与它同源，`bind` 是监听地址）。
- **修改库格式之后先看**：`akasha-store/src/schema.rs`（`DDL_V1` 冻结 + `TABLES`）→
  `lib.rs` 的 `upgrade()` / `FORMAT_VERSION` → `tests/format_migration.rs`。⚠️ 新增表**必须**提升
  `FORMAT_VERSION` 并编写迁移。
- **SSH 层的形状**：`docs/adr/0003-ssh-stack-and-resource-model.md`（状态「已定案」，
  落地它的 plan 已全部归档；改动只能由新的 ADR 取代，§14 含实现期间的修订记录）。动手前先读
  §2 的「事实依据」（版本 / API 均带出处）、D1–D16 与 §12 的遗留清单。
- **新增依赖**：`Cargo.toml` 中用 `=` 钉版本（`russh` 与 `tauri-specta` 同一口径）；
  本沙箱中 `cargo add` 会被拒绝（问题 #105），而 `cargo deny` 还会拉取其它平台的依赖（问题 #106）。
- **修改 E2E**：新增 `tests/*.rs` **必须**登记进 `src-tauri/justfile` 的
  `E2E_TARGETS` / `E2E_TARGETS_EXIT` / `E2E_NO_APP` / `E2E_SELF_APP` 之一（guard 会报错）；
  清单**顺序即执行顺序**，且它们共用同一个 app。
  ✅ **`tests/support/mod.rs` 无需登记**：guard 扫描 `tests/*.rs`（一层），而它由各目标通过
  `mod support;` 引入，是普通模块 —— 与 `akasha-store/tests/common/` 同一先例。

## 已知问题与教训

> 编号 `问题 #N` 是跨文档引用键（`AGENTS.md` 与 `docs/**` 均引用），**保持稳定、不重排**。

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency**。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**。
3. **just 的变量写 `$p`，不是 Make 的 `$$p`**。
4. **just 用 justfile 所在目录作为配方工作目录**。
5. **系统库缺失只在 cargo 构建脚本阶段暴露**；本机是 CachyOS（Arch 系），不是 apt。
6. **Tauri 没有 Rust 热重载，Victauri 也不提供**。
7. **just 的 shebang 配方需要可写的 runtime dir**，受限环境会失败。
8. **仓库根没有 `Cargo.toml`** → 根目录下一切 cargo 命令失败（**临时脚本也算**）。
9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** —— 运行一次 `just fmt`。
10. **CI 里 `libappindicator3-dev` 已不存在**，要用 `libayatana-appindicator3-dev`。
11. **受限环境下"写工作区之外被拒"看起来像工具/代码故障** —— 识别 → **直接提权重试**。
12. **`git checkout <file>` 会静默丢弃未提交的改动** —— 负例自检用 `cp` 备份/还原
    （⚠️ 备份**不得**放 `/tmp`：沙箱每次调用一个私有 `/tmp`）。
13. **正向校验若不限定到目标段落就形同虚设** —— `docs-check` 用 `awk` 取 §2。
14. **`docs-check` 的反向检查**已扩到 `AGENTS.md` / `ROADMAP.md` / `docs/**/*.md`。
    ⚠️ **它不认识"下一步才存在"的命令**：plan 里出现的 `just <新配方>` 会让 docs-check 失败 ——
    展开 plan 的那次提交要么先把配方落上，要么**先不写那个命令名**（plan 落地时就地补）。
15. **后台遗留的 `just dev` 会让 Vite 继续监听 1420 而 app 早已不在**。
16. **含反引号的 grep 模式在 justfile 配方里必须整体放进单引号**。
17. **多文件行数检查要逐文件取**（awk 的 `NR` 会跨文件累加）。
18. **`[profile.*]` 写在 workspace 成员里会被静默忽略**。
19. **整体 `mv` 构建缓存会留下写死的绝对路径**。
20. **`cargo` 在成员目录里只选当前包** —— crate 级配方不写 `--workspace` 会**静默漏掉**成员。
21. **tauri CLI 默认只监听 `src-tauri`** —— 成员放外面 = 开发循环静默失效。
22. **cargo 的空 glob 是硬错误**（`members = ["crates/*"]`）。
23. **justfile 里不能出现完整的 `{{ … }}`**（要写字面量用 `{{{{`）。
24. **"一份工作流同时服务两个 forge"会持续产生额外维护成本**。
25. **兼容层遗留的代码会以"看起来更稳"的形式保留下来**。
26. **阻塞的 `Read` 与"按时间交付"在机制上冲突** —— 正解是读线程 + `recv_timeout(期限)`。
27. **零匹配的测试过滤器在 nextest 里是"报错"**。
28. **`cargo bench` 会同时以 bench 模式运行单测目标**（正常行为）。
29. **仓库里出现第二个 bin 会让 `tauri dev` 无法启动** —— 修法是 `default-run`。
30. **只写 `path` 的依赖等于版本号写 `*`** —— path 依赖要**同时写 `version`**。
31. **`Channel<Vec<u8>>` 不是二进制通道** —— 真正走 raw 的只有
    `Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`。**不得只按字节数验收**。
32. **`u64` 不能直接过 IPC**：改用壳层 `u32` 句柄 + **checked** 转换。⚠️ 被生成器拒绝的是
    **一整类**（`usize` / `isize` / `i64` / `u64` / `i128` / `u128`）。
33. **沙箱里 E2E 必须与 app 在**同一次** bash 调用内**（每次调用都是独立的 bwrap）。
34. **`pkill -f <模式>` 会匹配到自身** —— 用 `pkill -f '[v]ite'` 或按 PID/进程组终止。
35. **`@xterm/addon-unicode11` 需要 `allowProposedApi: true`**。
36. **不在门禁里的测试等于没测**：`just test-e2e` 既不在 `ready` 里、又要真实 app。
37. **`git mv` 之后 `docs-check` 会同时验两件事**（文件在不在、索引指得对不对）。
38. **每加一个依赖就多一份要维护的放行**。
39. **"测试自己抛的异常"会污染同一 app 上后运行的用例**。
40. **vite 默认只监听 `[::1]:1420`**；`/tmp/victauri/<pid>/` 里的 `pid` **就是 app 的 pid**。
41. **`(cmd) &` 在 fish 里是命令替换，不是子 shell**。
42. **`cargo test` 一次收多个 `--test` 时按目标名字母序运行**，不按参数顺序。
43. **`tauri-plugin-log` 默认的 `TargetKind::LogDir` 会让"日志目录不可写"变成 app 打不开**。
44. **`tracing/log-always` 会把依赖树的 TRACE 一起转成 `log` 记录**。
45. **SIGKILL 的投递是异步的**。
46. **portable-pty(unix) 的 `Child::kill()` 不是纯 SIGKILL**。
47. **早于日志插件注册的 `tracing` 事件会静默消失**。
48. **`/proc/<pid>` 存在 ≠ 进程还活着**（僵尸也有目录项）。
49. **按"命令行里含某段文本"找进程会误伤**。
50. **`term.dispose()`（xterm）会抛，而它运行在 React 的 effect 清理函数里**。
51. **多标签之后 DOM 选择器不再唯一**。
52. **断言超时不一定是"慢"**：真实原因可能是**界面已经被卸载**。
53. **`tauri-specta` 的事件必须 `mount_events`**。
54. **官方 `Channel` 不提供"流已结束"的通知**。
55. **会话中还有其它进程持有 PTY 时，主端读不到 EOF**。
56. **接线一次的回调必须走 `ref`**。
57. **日志消息中的"括号解释"会持续膨胀**。
58. **`tauri-plugin-log` 默认 formatter 的时间戳只到秒**。
59. **`signal` 字段的值是本地化的**（zh_CN 下 `SIGKILL` 写成 `已杀死`），尚未修复。
60. **托盘在 Linux 上需要写盘** —— 只读 runtime dir 中无法创建，因此它只能是**可选能力**。
61. **`libayatana-appindicator3` 与老的 `libappindicator3` 都是运行时 dlopen**。
62. **dbusmenu 的 item id 会随菜单重建而改变**。
63. **SNI 注册用的是唯一名**（`:1.x`）。
64. **Victauri 的 `window` 工具能机器验证窗口状态**。
65. **`AppHandle::exit()` 也会触发 `RunEvent::ExitRequested`**。
66. **便携数据目录就在 bin 同目录**，而 `just dev` 与 `just test-e2e` **共用同一个 bin**。
67. **Victauri 的 REST 兜底接口返回的是 `{"result": …}` 包了一层**（而 `just test-e2e` 里的
    `VictauriClient` 走 MCP，`call_tool` 直接返回工具内容本身）。
68. **`/proc/<pid>/exe` 可能带 ` (deleted)` 后缀**。
69. **文档中的"事实"若不核对会持续膨胀**。
70. **cargo 的 workspace lint 继承是"全有或全无"**，而 `forbid` 不能被 `allow` 覆盖。
71. **带 `links = "..."` 的原生库在依赖树里只能有一个版本**。
72. **SQLCipher 的空 key 不是"静默关闭加密"，而是"返回错误且不装载 codec"**。
73. **SQLite 自己建出来的库文件是 644**（umask 022），不是 0600。
74. **`PRAGMA cipher_settings` 的输出是一列 `pragma` 行**。
75. **cargo-deny 的图根是"manifest 指向的那个包"，不是整个 workspace**。
76. **"0 字节的库"不是"空库"，是"还没有密钥"**。
77. **`cipher_memory_security` 是进程级、单向的**。
78. **模块内的 `#[cfg(test)] mod tests` 也要自己 `allow(clippy::unwrap_used)`**。
79. **`/proc/<pid>/mem` 的读用 `FOLL_FORCE`，绕过页保护**。
80. **`/proc/self/smaps` 的字段不是处处都有，而且属性行带缩进**。
81. **子串匹配会使规则失效**：按 `_` 分词、整词比较，并配一对命中 / 诱饵负例。
82. **Victauri 的 `get_registry` 在本仓库是空的**（命令都没标 `#[inspectable]`）。
83. **clippy 会把"两个常量比较"的断言判为失败**（`assertions_on_constants`）—— 搬进 `const { … }`。
84. **E2E 配方两段可能运行在**不同**的数据目录里** —— 两段都先 `mkdir -p` 便携目录。
85. **集成测试的共用脚手架放 `tests/common/mod.rs`，但必须自己 `#![allow(dead_code)]`**。
86. **`pkill -f <pattern>` 会匹配到该命令自身的命令行**。
87. **"什么都没发生"这类判据最容易写成永真式** —— 先用负例确认该判据会失败。
88. **`sqlcipher_export` 写出来的文件默认是版本 0**（它不传递 `user_version`）。
89. **"导出另开一条实现路径"的代价最高**。
90. **扫描器本身位于它要扫描的地址空间内** —— 缓冲复用 + 读完即擦 + 真随机的针。
91. **给"要扫描的段"设上限 = 使应当看见的副本落在扫描窗口之外**（漏扫与未泄漏在判据上无法区分）。
92. **解锁期间 `VmLck` 涨的大头不是我们那一页**：`cipher_memory_security` 会给 SQLCipher 的
    每次分配 `mlock`（实测 152 kB 里 148 kB）。
93. **"发现目录出现" ≠ "app 就绪"**：Victauri 的插件 setup 比 app 自己的 `.setup()` 早，
    所以刚连上时 `lifecycle` probe 还是 `{"initialized":false}`。判据要"等那个字段自己出现"。
    同一类还有第二层：`invoke_command` 走 webview bridge，是**最后**才好的一个。
94. **`no-println` 规则豁免的是 `**/tests/**`，不是 `#[cfg(test)] mod tests`**。
    正解：单测的"跳过"换成**不依赖环境**的 fixture，要输出就放进 `tests/`。
95. **用 `chmod` 造"不可写"在单测中不成立** —— 改用**结构性**造法（路径指向普通文件之下，
    `ENOTDIR` 对 root 同样无法绕过）。
96. **clippy 的 `undocumented_unsafe_blocks` 本来就看私有项**（1.98 实测）。
97. **只加 `undocumented_unsafe_blocks` 会漏掉另一半**：要靠反向的
    `unnecessary_safety_comment` / `unnecessary_safety_doc` 才闭环。
98. **文档里的"或 X"最容易无依据地出现** —— **核查时把每个"或"当成一条待证断言**。
99. **"唯一一处"这类计数若不核对就会失准**：要么写成"唯一**允许**的 crate"（结构性表述），
    要么就不在注释里写数字。
100. **照搬外部规范时要分清"结构"与"语种"**：解释写中文，只将**标签字面量**固定。
101. **断言型正则要按"节点实际文本"写**（`^Channel$` 匹配不到 `tauri::ipc::Channel<Vec<u8>>`）。
102. **探针必须放进规则 `files:` 覆盖的真实路径，且正例与诱饵都要有**。
103. **"词汇表"规则里，词边界是规则的一部分**（裸 `(Tab|Pane|Window|View)` 会误伤 `Table`）。
104. **`ast-grep test` 只测规则逻辑，不测 `files:` / `ignores:`** —— 路径范围仍要真实路径探针。
105. **`cargo add` / `cargo search` 需要写 `~/.cargo` 的索引缓存**：沙箱下该目录只读 → 按问题 #11 提权重试。
106. **`cargo deny` 会为 `cargo metadata` 拉取其它平台的依赖**（`russh` 会带出 `pageant`）。
107. **edition 2024 的 `impl Trait` 会捕获输入生命期** —— 不使用 `run_on_socket`，自行编写 accept 循环。
108. **`Transport::output_stream()` 只能获取一次**（两个读端会互相窃取字节）。
109. **`std::env::set_var` 在 Rust 2024 中是 `unsafe`**：依赖环境变量开关的行为无法在测试中构造前提；
    正解是把它变成**输入**（`SshAuth::agent_socket`）。
110. **`cargo nextest run -p <crate> <关键词>` 过滤的是测试的**函数名**，不是文件名** ——
    按文件过滤需使用 `--test <目标名>`。过滤器写错会使判据**永远不执行**。
111. **上游 `russh` 的 `Error::KeyChanged { line }` 在跳过注释行时不会递增行号** —— 要给出真实行号
    需自行计数（`true_line_of`）。
112. **`thiserror` 会把名为 `source` 的字段当作错误源**（插值会编译失败）—— 改用其它字段名。
113. **`// SAFETY:` 的位置即其含义**：写成 `/// SAFETY:` 挂在安全函数上会同时触发两条 lint。
114. **迁移必须在改动前先按*旧*版本校验形状**（顺序：读版本 → 按该版本查表 → 迁移 → 再查表）。
115. **`i64` 与 `u64` 一样无法通过 IPC**：以 `u32` 代理 + **checked** 转换。
116. **不得将 IPC 投影类型命名为 `*View`**：本仓库的 `no-ui-vocab-in-types` 会命中该词边界。
117. **tokio 1.53 的 `Runtime::handle()` 返回 `&Handle`** —— 存入结构体需 `.clone()`。
118. **React StrictMode（仅开发模式）会执行 effect 两遍**：类似"建立一个会话"的副作用因此发生两次；
    SSH 的两次提示会同时显示在同一面板中，表现为"点击 SSH 后一直连不上"。处置：连接**无条件推迟
    一个微任务**再发起。
119. **沙箱中"同一次 bash 调用"的边界包含重定向写出的日志文件** —— 写入 `/tmp` 后下次调用读不到，
    需写入**工作区**。
120. **`russh` 的 `Config::nodelay` 只在 `client::connect` 中生效**：我们两条路都使用
    `connect_stream`，因此"把 `config.nodelay` 设为 `true`"在这条路上**未生效**
    （plan 0502 曾据此写出一个版本）。正解：自建 `TcpStream` 时显式 `set_nodelay(true)`
    —— 而"自建 TCP"正是跳板所需的形状（底层流由调用方提供）。
121. **服务端的 `Handler::data()` 对**所有**通道都会被调用**（上游把数据**同时**交给通道自身的接收端
    **与** handler，`server/encrypted.rs:1251`）。因此测试服务端的"回显"必须只对 **shell** 通道执行
    —— 否则 `direct-tcpip` 通道上的字节会被原样回送，客户端读到的是**自己刚写入的 SSH id 行**，
    报错为 `Bad packet size: 1397966893`（该数字即 `"SSH-"`）。
    结论：**假服务端的每个回调都要明确它对哪些通道生效**。
122. **`copy_bidirectional` 出错时不会交出已搬运的字节数**，而收尾时报错是常态 —— 把计数建立在
    它的返回值上，会让"搬运了多少字节"这条判据在最需要时恒为 0。正解：包一层
    `AsyncWrite` 计数写入（同时使该值**在会话仍开启时即可读出**）。
123. **`rusqlite` 不是 app 的 dev-dependency**（`akasha-store` 才是）—— 集成测试中需要书写
    `Connection` 类型时，使用 `akasha_store::Connection` 这一**再导出**，不要在 `Cargo.toml` 中
    新增一份需要版本对齐的重复依赖。
124. **全部 E2E 目标运行在同一个 app 进程中，而内存凭据缓存的键是 `(host, port, user, 认证方式)`**
    （ADR-0003 D8，`Arc` 共享）。因此两个 E2E 目标若使用**同一个 `host:port`**（例如都以
    `akasha-e2e-inner.invalid:22` 作为"仅跳板可见的目标"），而后一个目标提供了**另一条口令**，
    第二个目标会**静默使用缓存中的口令**：服务端拒绝 → `password_step` 仅执行 `forget`、该连接
    随即结束 → 整条认证**失败**，用户（与用例）**不会**被重新询问。症状是"提示问答未走完即无法
    连接"，而 `wait_connected` 的诊断会给出真正原因（`认证失败：… 上没有可用的方式`）。
    正解：**每个 SSH E2E 目标使用自己的目标名**（`akasha-e2e-import.invalid` 即由此而来）——
    端口冲突无影响（跳板端口每次随机），**名字会**。
125. **`thiserror` 的 `#[error("…", expr)]` 不接受位置参数** —— 写 `… 有 {} 处 …", problems.len()`
    会得到 `expected an expression`。要么使用字段引用（`{problems}`，但要求该字段实现 `Display`），
    要么**在构造处拼接完整句子**并存入 `message` 字段（`ImportError::Refused` 即如此）。
126. **登记一个实体时的"初始状态"不是一次状态转移**：plan 0601 最初在 `tunnel_open` 里对新登记的
    隧道再走一次状态机（`→ 连接中`），而它登记时**已经**是那个状态 —— `connecting → connecting`
    被状态机（正确地）判为非法边，整条命令随即失败，表现为"点了打开、界面立刻报状态转移被拒"。
    正解：**在登记处发那条事件**，状态机只管"之后的变化"。
127. **`target_host` / `target_port` 在 plan 0601 不参与连接**：隧道只连**规则所属主机**
    （`forwards.host_id`）。因此"连接失败"的构造点在**主机**那一层 —— 把目标端口写成不可达端口
    不会让它失败（这一版根本不连目标），表现为"用例以为在验失败路径，实际验的是成功路径"。
128. **测试服务端原先只有通道级的断开计数**（`sessions_closed`，由 `channel_close` 回调 +1）：
    隧道**没有通道**（ADR-0003 D4），于是"停下来之后连接真的断了"在这条路上**没有任何观察点**，
    只能得到一个永真的断言。正解：新增**连接级**计数（`Observed::connections_closed`，
    在 handler 的 `Drop` 里数 —— 真实 handler 由 `Server::new_client` 标出，
    `Clone` 出的中间副本不计）。
129. **进程级的 `---p` 判据会被"别人的守卫页"搅动**：`credential_protection` 按
     `/proc/self/maps` 里 `---p` 映射的**总量**判断"8 条凭据 → 多 8 页"，而同一个进程里并发的
     那个测试线程结束时，它的**线程栈守卫页**（同样是 `---p`）会被解除映射 —— 实测 8 页只数到
     7 页（+28 kB 而不是 +32 kB），约一半概率红。
     ⚠️ **它只在 `cargo test` 下发作**（一个二进制里的多个测试运行在同一进程的多个线程上）；
     `just test` 用 **nextest**（一个测试一个进程）因此不触发 —— 门禁不受影响，这也是它长期没被发现的原因。
     正确的做法是**按归属**而不是按总量：`akasha-store` 的用例已经在用 `smaps` 的 `VmFlags`
     （`memsafe` 的页带 `dd` / `wf`，守卫页没有），把这条判据改成同一口径即可。
130. **E2E 共用同一个 app，而"面板"是有状态的浮层**：`.tab-new-tunnel` 是**切换**，面板的规则表
     又是**挂载时读一次**（`TunnelPanel`）—— 于是下一个目标点同一按钮会把上一个目标留下的面板
     **关闭**，表现为"面板列不出规则"（超时），而不是"按钮点错了"。
     正解：`support::open_tunnel_panel` —— **先卸下再挂上**（既幂等，又保证重新读一次池子）。
     教训：共用 app 的 E2E 目标里，"打开某个浮层"必须是幂等的，且要假设它已经是打开状态。
131. **"地址不合规"与"地址没填"是两件事，检查顺序决定报哪一句**：plan 0603 的回环检查
      （SOCKS5 不许绑非回环地址）最初排在"空绑定地址"之前 —— 于是规则里没填绑定地址时，
      用户看到的是一句带着**空地址**的「SOCKS5 监听不能绑到 ：…」。正解：**先判空、再判合规**，
      而且那句"该填什么"要按入站类型分开（`-L` 可以提 `0.0.0.0`，SOCKS5 提它等于推荐一个
      下一句就被拒的地址）。
132. **入站通道的回调运行在连接的消息循环上**：`russh` 把
     `Handler::server_channel_open_forwarded_tcpip` 的 future **在连接的消息循环里 `await`**
     （`client/encrypted.rs`）。因此在这个回调里做任何耗时的 `await`（例如连一个不可达的本机
     地址）都会让**整条连接**无响应 —— 保活也停，而外在表现是"隧道看起来还活着"。
     正解：回调里只做同步的派发（把通道与接受句柄一起送进 mpsc），接线在**任务**里做。
     教训：**回调的 `async` 不等于可以慢** —— 要看它被谁 `await`。
133. **匹配用的键要取"双方都认同的那个"**：`-R` 的入站通道带两个身份 ——
     服务端回报的 `connected_address` 与 `connected_port`。前者**由服务端决定**
     （它认为在听的地址；真实 `sshd` 会受 `GatewayPorts` 一类设置影响，请求 `localhost`
     可能回报 `127.0.0.1`），只有**端口**是我们请求过、服务端回报回来的那一个。
     按地址字符串匹配会在真实服务端上**静默失配**（通道被当成"没登记过"而拒绝），
     而测试服务端原样回报请求里的地址，所以它**测不出来**。
     教训：测试替身"原样回报"的字段，在真实对端那里可能被改写 —— 判据要落在双方都认同的字段上。
134. **`tcpip_forward` 的返回值有两种含义，必须按"请求的是什么"来解**：RFC 4254 §7.1 规定服务端
     **只在请求的就是 0 端口时**才在回复里带端口；请求了具体端口时回复**没有**这个字段。
     上游 `russh` 把"没有字段"表示成 `0`（客户端 `client/encrypted.rs`：*If a specific port
     was requested, the reply has no data* → `Some(0)`；服务端那一侧同样只在 `port == 0` 时才写
     该字段）。于是 `Handle::tcpip_forward(地址, 8080)` 返回的 `0` 意思是**"回复里没有端口"**，
     不是"绑到了 0 端口"。plan 0604 最初直接采用返回值 —— 症状是 probe 报 `127.0.0.1:0`，
     而且停止时拿 0 去 `cancel-tcpip-forward`，**那个监听根本不会被撤掉**（服务端按
     `(地址, 端口)` 查，查不到就回失败）。正解：请求非 0 时端口就是请求的那个；请求 0 时
     必须用回复里的值，它也是 0 就报错。⚠️ 它只在真实路径上暴露 —— crate 用例当时只用 0 端口
     （由服务端挑）验过，**具体端口那条路没有人断言过返回值**。
     （由服务端挑）验过，**具体端口那条路没有人断言过返回值**。（plan 0605 补上了那条断言。）
135. **"状态变了"与"表变了"是两件事，而托盘只订阅了后者**：托盘菜单是一份**快照**
     （`tray::refresh` 挂在 `Sessions::on_change` 上重建整份菜单），而 `Sessions::set_tunnel_state`
     起初**只**往注册表发事件、没有调用 `notify_changed` —— 于是菜单永远停在隧道刚登记时那一行
     （`连接中`），`scope.md` §5.2 指定的"失败必须可见"落点是**死的**（plan 0301 建好了那条管线，
     但状态变化从没通知过它）。正解：状态变化也重推菜单。教训：**一个订阅者 + 快照式重建**的组合里，
     "哪几件事算变化"要逐条核对 —— 少一条不会报错，只会让界面长期显示一个旧值。
136. **收尾信号要带原因，因为触发收尾的地方不止一个**：一条转发的结束有两种来路 ——
     `shutdown()`（我们让它停的）与那条 SSH 连接没了。只报"结束了"的话，重连循环会把用户刚停掉的
     隧道**重新拉起来**。正解：`ForwardEnd { Stopped, ConnectionLost }` 跟着结束信号一起送出去，
     并且循环**再加一道状态判据**（只有实体仍在 `已连接` 才重连）兜住竞争。
137. **`abort()` 杀不掉"任务里又 spawn 的那一层"**：`russh::server::run_stream` 内部把真正的会话
     **又 spawn 了一层**（`session.run(...)`，返回值 `RunningSession` 只是它的包装），abort 外层
     只是丢掉包装 —— 里面的会话照常运行，socket 与 handler 都归它。实测症状：用 abort 实现的
     `cut_connections` 让 `connections_closed` **恒为 0**、"切掉"的连接毫发无损，用例看起来在切、
     实际什么都没切。正解：走会话自己的 `Handle::disconnect`（那也正是"服务端断开这条连接"的真实
     形态）。教训：**abort 的语义要按"那个任务里到底有什么"核对**，包装层的句柄不等于里面那条。
138. **测试替身要对它扮演的东西的生命周期负责**：测试服务端的远端监听原先放在一张**全局**表里、
     只有 `cancel_tcpip_forward` 才会停它 —— 于是"连接断了、监听还在"，而重连对同一个端口的
     `tcpip_forward` 会被**自己上一次留下的监听**顶掉（表现为"重连必然第一次失败"）。真实的 `sshd`
     里转发属于**那条连接**，连接一断端口就还回去。正解：表挂在每条连接的 handler 上，
     `Drop` 时把它的监听一起停掉。
139. **代理里包一层就会多一层生命周期**：`Running` 里存的是包装任务的句柄，而"连接还活着"这件事
     要看**会话**（`Connection::session`）。清点"谁还活着"时按**包装**清点会漏 —— plan 0605 的用法是
     "切连接走会话句柄、`is_finished` 只看包装"，两者各自回答一个不同的问题（问题 #137 的近亲）。
140. **失败分档要按"哪一层坏了"分，而不是按错误类型分**（D13 的落地）：`Connect` / `Jump`（网络）
     与"端口没拿到"（资源被占）应当重试；认证、主机密钥、配置与内部状态**不重试**。⚠️ 分错档的
     后果不对称：该重试的没重试只是少一次自动恢复，而**不该重试的却重试了**会以错误的口令连试三次
     —— 那正是账号锁定的经典成因。它写成 `TunnelError::retryable` 并有两组正反例钉住。
141. **上游只给了同步的"连接还在吗"**：`russh` 的 `client::Handle::is_closed()` 是同步的
     （背后是"会话消息循环的接收端还在不在"），**没有可 `await` 的关闭信号**。因此重连的"断开"靠
     转发任务按固定间隔看一眼（plan 0605 取 500 ms）。⚠️ 它对"半死"（TCP 没断、对端不回话）不敏感
     —— 那种情况要等保活耗尽（`keepalive_interval × keepalive_max`，默认约 90 秒），
     所以"拔网线"在真实网络上的发现延迟是**保活量级**，不是秒级。
142. **在阻塞线程上执行的那件事，丢掉 `await` 那一侧取消不了它**（plan 0606）：`spawn_sync`
     （= `spawn_blocking`）里的握手会一直执行到底 —— 扔掉 `JoinHandle` 只是不再等它，
     它建起来的那个 socket 也一直开着（最长一个 `connect_timeout`，D15 的 10 s）。
     表现是"关闭一条正在握手的隧道之后，对端几秒内仍看得到那条连接"。
     **判据**：凡是要能"当场停"的阻塞活儿，都得把取消信号送进**那次调用自己**（这里做成了
     `SshConnection::connect_via_until(…, cancel)`），而不是指望取消 `await`。
143. **`oneshot` 的发送端换一个，接收端会立刻醒** —— 于是 `select!` 会**随机**挑分支
     （plan 0606 的返工）：一个任务在开始前"登记自己的停止入口"，成功的连接由**看护任务**
     再登记一份、把尝试那一份替换掉；那一刻尝试的 `select!` 两个分支同时就绪，
     `tokio::select!` 随机挑一个 —— 一次**成功**的连接因此有约一半的机会被报成"已停止"。
     改为"**实体自己持有一对 `watch`**、每个动作订一份接收端"之后，接收端只会在真的被要求停止
     （或实体没了）时醒。教训：**信号的所有权要跟着被停止的东西，而不是跟着停止它的那一次动作**。
     ⚠️ 附带一条 `watch` 语义：`Sender::send` 在**没有接收端**时返回 `Err` 且**什么都不做**，
     有接收端时即使值没变也会通知 —— 后者正是"第二次停止照样有效"依赖的性质。
144. **界面显示的那行状态，在一次命令在途期间是**上一次**的**（plan 0703）：点「连接」之后
     后端立刻把那一侧置成 `connecting`（`prepare_connect` 在任何 I/O 之前），而面板要等命令
     返回才刷新 —— 于是**重新连接**期间界面上仍写着"已连接"，看起来那一栏还是可用的。
     两处处置：① 面板在命令在途时按后端的事实显示 `connecting`（这正是后端此刻的状态，
     不是界面猜的）；② **E2E 的完成条件改读 `sftp` 探针**（`state = connected` 且
     `origin.id` 是这一次选的那台）。教训：**"状态"这类断言要读后端的事实，界面上的字可能是
     上一次留下的** —— 这条在一栏重新连接时才暴露，第一次连接时旧值恰好也是"未连接"，所以看不出来。
     ⚠️ 同一个用例在**第二次整份执行**时又红了一次，原因是这条教训的**另一半**：判据改读后端
     之后就**不能紧接着读界面**（面板要等命令返回之后才刷新）—— 那一次读到的是还没渲染的空串。
     正解是**先等那一行出现、再断言它的内容**（`wait_js` + 读文本）：后端的事实可以立刻断言，
     界面的呈现要先等到它出现。
145. **汇总类文档里"像量化收益"的一句话，会在实现时被证伪**（plan 0703）：`scope.md` §4.1
     原写"B 档使本机带宽减半（1×）"、A 档"2×"，而协议层面两档都要**读源一遍、写目标一遍**
     —— 字节数相同，B 档真正的收益是**可直达性**（本机只需够得着 A 一台）。它此前没有出处、
     也没有实测，按字面读会让人去优化一个不存在的一半带宽。处置：`scope.md` 那一格改成
     "本机的可直达性要求"，ADR-0006 D5 记下实测口径。教训：**收益要写成可被证伪的量**，
     写"减半"就得同时写下它的分子分母。
146. **`SshConnection::over` 不持有承载它的那条连接**（plan 0703）：它只造下一跳，
     `under` 是空的 —— 链的存活归调用方（`chain` / `connect_via` 会把整条链放进去）。
     单独用 `over` 建一跳看起来像一条能独立存在的连接，而它的"网络"随时会随承载者消失。
     app 那条路走的是 `connect_via_until`，所以不受影响；这条写下来是因为它是**接口上的一处陷阱**：
     名字相同、语义相邻的两个入口，活着的条件不一样。
147. **测试替身没实现的协议标志，会让判据静默失去对象**（plan 0704）：`testing.rs` 的 SFTP
     服务端把 `OpenFlags::EXCLUDE` 与 `CREATE` 合在一个分支里 `create(true)` —— 于是**它不拒绝**
     第二个同名文件，"两个同名文件不能撞在同一个临时名上"这条判据在库内看起来成立、实际没有
     （两条并发传输会交错写同一个临时文件，而用例全绿）。修法照上游：`EXCLUDE` 走 `create_new`
     （`O_EXCL`）。教训：**替身要不要实现某个标志，是"这条判据还算不算数"的问题，不是脚手架细节**；
     凡是判据依赖的协议语义，替身必须实现它，否则用例测的是替身而不是被测代码。
148. **带时延链路的两个方向接反，报出来的是"握手坏了"**（plan 0704）：把"客户端读"接到
     "客户端写"上，客户端收到的就是自己刚发出去的字节，`russh` 报 `Key exchange init failed`
     —— 那读起来像 KEX 实现有问题，实际上是测试脚手架接错了两根线。同一处还有第二条：
     时延要按"段各自计时"实现，写成"读一段、睡一段、写一段"的循环会让链路自己变成一个
     串行瓶颈，于是并发发出去的请求**在链路里排队**，"并发更快"在上限 2、4 上几乎量不出来。
    教训：**测量装置本身要先被怀疑一次** —— 判据给出反直觉的数字时，先问"这个数是不是装置造出来的"。
149. **`akasha-pty` 用了 Windows 上不存在的 `rustix::process`，且没有 `cfg` 守卫**：
     上游把 `rustix::process` 限定在 `#[cfg(not(windows))]`，而 `teardown.rs` / `watchdog.rs`
     直接用它的 `Pid` / `Signal` / `kill_process` / `setsid` —— 于是 **Windows 目标编译不过**：
     `cargo check --target x86_64-pc-windows-msvc` 在 `akasha-pty` 就红（3 个 E0432 / E0433）。
     它长期没暴露的原因是 CI 的 Windows 那一格（`checks-other`）到现在还没有真正执行过
     （状态见「待验证」里的 plan 0102）。修它不是补一个 `cfg` 那么简单：Windows 上"回收整个会话"
     没有 POSIX 进程组语义（要走 Job Object），而 `watchdog`（ADR-0005）的管道 EOF 机制本身也是
     Unix 形状 —— 这是计划级的活。⚠️ 因此阶段 8 那条判据在本机只能用依赖图核对，
     原生编译证据要等这个问题修好。
150. **枚举出来的端口不保证能打开**（plan 0802 实测）：本机（libudev 那套）列出 32 条
     `/dev/ttyS0`…`/dev/ttyS31`，而 `/dev` 下**一个都没有** —— 上游按 udev 设备给 devnode，
     不检查那个节点在 `/dev` 下是否存在；它那句"打不开就跳过"的过滤
     （`serialport` 的 `enumerate.rs`：parent 的驱动是 `serial8250` 且打不开时跳过）本机没有触发。
     同一处还有第二个读数：`--no-default-features`（sysfs 那套）在**同一台机器**上返回 0 条，
     因为它要求 `/dev/<名字>` 存在。教训：**"列出"不等于"能用"** —— 判据写成"枚举到的端口都能
     打开"会得到一条与真机相反的断言（本机恒假、接上设备恒真）；正确的位置是"枚举只负责列，
     打开失败带路径与原因"（`SerialError::Open`），而界面不要把列表当成"可用端口"。
151. **串口接入 app 在 ROADMAP 里没有条目**（plan 0802 收尾时发现）：阶段 8 的两条都在 crate 层 ——
     枚举与参数可用，但没有任何命令、事件或界面把它们接到 app 上，`scope.md` §2 的"三大终端之一
     （serial）"因此没有用户可见的形态。处置（2026-09-15 已落地）：立为**阶段 11**（plan 1101–1103）。
     ⚠️ 编号不复用，教训留给它：**一个阶段的判据全在 crate 层时，产品形态的那一半要有自己的条目**。
