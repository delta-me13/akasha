# ADR-0006：SFTP 栈与传输引擎

- **状态**：**实现中**（Implementing，2026-09-15）
- **日期**：2026-09-15
- **决策者**：cyrene
- **影响范围**：新增依赖 `russh-sftp`；`akasha-ssh` 多一个 SFTP 会话类型与其失败档；
  `SessionKind::Sftp` 拥有的资源形状；阶段 7 的传输引擎接口（plan 0702 / 0703 / 0704 的地基）；
  SFTP 的 IPC 契约（命令、`side` 的表示、只读探针）
- **关联**：[plan 0701](../plans/0701-sftp-dual-pane.md)（本文的落地起点）、
  [0702](../plans/0702-transfer-atomic-rename.md) / [0703](../plans/0703-host-to-host-topology.md) /
  [0704](../plans/0704-pipelining.md)
- **取代**：无

---

## 1. 背景与范围

`docs/scope.md` §4 已定案文件传输的**产品**结论：双栏、两侧独立选主机、host↔host 两档
（优先 B 档直连、失败回退 A 档内存 relay）、一律"临时名 + 原子重命名"落盘、支持并发
in-flight；§5.1 另定 SFTP 是一个**独立 `Session`**，不依赖终端。

本文把这些转成可实现、可核对的架构决策，只钉三件改动代价大的事：

1. **SFTP 协议由谁实现**（引入哪个库、它的形状与本仓库哪一层对齐）
2. **SFTP 会话承载在哪条通道上、这条通道归谁所有**
3. **传输引擎的形状与职责边界**（0702 / 0703 / 0704 在同一处相接）

本文只定义形状与约束，不含实现步骤 —— 步骤在 plan 0701–0704。
按 [`docs/adr/README.md`](./README.md) 的三态规则，落地它的 plan 尚未全部完成，状态因此是
**实现中**：§1–§5 可以改，每次改动在 §6 记一行。

| `scope.md` 已定案的条目 | 本文对应的决策 |
|---|---|
| §4 双栏、两侧独立选主机 | D3 / D7 |
| §4.1 host↔host 两档，B 档复用 `direct-tcpip` 原语 | D5 |
| §4.1 传输引擎支持并发 in-flight | D6 |
| §4.2 临时名 + 原子重命名；失败 / 取消 / 关闭 `Session` 删临时文件 | D4 |
| §5.1 SFTP 是独立 `Session`，资源挂它、事件按 `SessionId` 路由 | D7 |
| §2.1 纯 Rust，不调用系统 `ssh` / `sftp` | D1 |

## 2. 事实依据（调研，2026-09-15）

以下每条均可核对。来源：crates.io API 与从 `static.crates.io` 下载的 `russh-sftp-3.0.0` 源码
（引用的行号为解包后的路径）；本仓库侧为 `src-tauri/Cargo.lock` 与 `crates/akasha-ssh/Cargo.toml`。

| 事实 | 值 | 出处 |
|---|---|---|
| 最新版本 | `3.0.0`（2026-09-08 发布；上一版 `2.4.0` = 2026-08-03） | `crates.io` 的 `/api/v1/crates/russh-sftp/versions` |
| 许可 | `Apache-2.0` | 同一来源的 `versions[].license` 与包内 `Cargo.toml` |
| 与 `russh` 的关系 | **不是普通依赖**：`russh` 只出现在 dev-dependencies（`^0.63.2`），normal 依赖里没有它 | `russh-sftp-3.0.0/Cargo.toml` |
| 客户端入口 | `client::SftpSession::new(stream)`，约束是 `S: AsyncRead + AsyncWrite + Unpin + Send + 'static` | `src/client/session.rs:31`（结构）、`:38`（构造） |
| 高层接口 | 与 `std::fs` 同形：`read_dir` / `canonicalize` / `metadata` / `rename` / `remove_file` / `create_dir` / `open_with_flags` | `src/client/session.rs:98–290` |
| 通道怎么来 | 开一个 session 通道 → `request_subsystem(true, "sftp")` → `channel.into_stream()` 交给 `SftpSession::new` | 包内 `examples/client.rs` |
| 并发 in-flight | 请求表按 id 索引、id 由 `AtomicU32` 分配（`next_req_id`），一次请求一个 oneshot | `src/client/rawsession.rs:171`（结构）、`:243`（分配） |
| 服务端 | `server::run(stream, handler)` 内部 `tokio::spawn`，调用方立即返回 | `src/server/mod.rs:99` |
| 服务端 handler | `server::Handler` 是一族带默认实现的方法（`opendir` / `readdir` / `realpath` / `stat` …），未实现的走 `unimplemented()` | `src/server/handler.rs` |
| 本仓库现状 | `russh = "=0.63.3"`；`SshStream` 已把上游 `ChannelStream` 包成 `AsyncRead + AsyncWrite` | `crates/akasha-ssh/Cargo.toml`、`src/forward.rs:45` |

## 3. 决策

### D1：SFTP 协议实现取 `russh-sftp = "=3.0.0"`，版本钉死

与 `russh` / `tauri-specta` 同一口径（`Cargo.toml` 里用 `=`）。理由按重要性排：

1. **它的会话定义在一条流上**，而本仓库已经有一条这样的流（D9 的 `SshStream`）——
   三种拓扑（直连目标 / 经跳板的目标 / host↔host 的 B 档）因此是同一个类型的同一份用法，
   不需要为任何一种另写一套通道逻辑。
2. **它不依赖 `russh`**：上游类型只在 dev-dependencies 里出现，于是引入它不会把
   `russh` 的版本钉在它那一侧 —— 这与本仓库"`russh` 升级只改 `akasha-ssh` 一个 crate"
   的既有安排相容（ADR-0003 §11）。
3. **它自带服务端**：E2E 的判据需要一台进程内的 SFTP 服务端，而"自己写一个假 SFTP 服务端"
   与"自己写一个 SFTP 客户端"是同一份协议工作量的两半。

**否决的替代路**：自研 SFTP 客户端。v3 的包编解码与扩展协商本身不算难，难的是同一层的另外三件
事：并发 in-flight 下请求 id 的生命周期、`limits@openssh.com` 这类扩展缺失时的降级、
以及"三种拓扑共用一条流"这个约束在重连与取消时的一致性。这三件事没有产品差异，
却正是最容易写出静默错误的地方。

### D2：SFTP 会话承载在一条 `AsyncRead + AsyncWrite` 流上，对外类型是 `SftpClient`

- 入口是 `SshConnection::sftp()`：开一个 session 通道 → 请求 `sftp` 子系统 →
  把 `channel.into_stream()` 交给上游的会话。两者任一失败都报同一个失败档（见 §5）。
- ⚠️ **"请求子系统"这一步只发送、不等回复**（上游 `russh` 的 `request_subsystem` 走的是
  `send_msg`）：对端没开 SFTP 时它不会回 `VERSION`，于是"它拒绝了"实际表现为
  **初始化会话等满期限**。因此两件事是必须的：错误消息里说明"多半是对端没开 SFTP"，
  且那个期限**可调**（`SshConnection::sftp_with_timeout`）—— 否则一条"这里会失败"的
  负例用例只能靠等满默认的 10 秒来证明。
- **上游类型不外漏**（同 `SshStream` 的纪律，ADR-0003 §7）：对外的操作是
  `list(path)` 之类的方法，返回本仓库自己的条目类型。上游换 API 时改动止步于
  `akasha-ssh/src/sftp.rs`。
- 会话只**借用**连接：`SshConnection` 归调用方持有，`SftpClient` 不延长它的生命期。
  这条约束是 D3 与 D5 能同时成立的原因 —— 谁持有连接，谁决定它何时结束。
- 端点的动作是**异步**的，而两端在运行期才决定是本机还是某台主机 —— 于是 trait 的每个方法
  返回 `Box<dyn Future>`（trait 里的 `async fn` 不是 dyn 兼容的）。一个类型别名，
  不新增依赖，也不把上游的 future 类型漏出去。

### D3：一个 SFTP `Session` 拥有**两侧**，每侧一条**独立**连接

- 两侧可以是两台不同的机器（`scope.md` §4），所以"每侧一条连接"不是优化，是语义。
- 每侧的资源是 `{主机行 id, SshConnection, SftpClient}`，随这个 `Session` 生灭。
- **不复用同一条连接**：复用会让"一侧断开"变成两侧一起断开，而两侧的失败必须各自可见、
  各自可重试。
- 两侧的存在与状态是后端的事实，与前端怎么画它们无关（`scope.md` §1.2 / §1.3）。

### D4：传输引擎只认"两个端点"，落盘不变量归端点

- 引擎放 `akasha-ssh`（零 Tauri 依赖，可脱离 app 测；`AGENTS.md` §3.1）。
- 引擎的输入是**源端点**与**目标端点**，各自提供"读一个文件 / 写一个文件 / 列目录"这几种
  能力；引擎不认识 SSH、SFTP，也不认识本地路径。
- **"临时名 + 原子重命名"是目标端点的职责**：本地端点用 `std::fs`（同目录下写 `.name.part`
  再 `rename`），远端端点用 SFTP 的 `open` / `write` / `rename`。引擎因此不知道目标在哪，
  也不知道"落盘"意味着什么 —— 这正是"三种拓扑共用同一个引擎"能成立的原因。
- **临时名的取名规则在引擎那一层，挑一个没被占用的由端点做**（`.name.part`，
  被占用时退到 `.name.N.part`，最多试 16 个）。这样 D6 要的唯一性落在**接口**上，
  而不是落进某一种端点的实现里；占用探测与创建之间不是原子的，0704 认真做并发时
  要重新回答那一点（见 §4）。
- **取消只有一条路径**：用户取消与关闭 `Session` 走同一个中止入口，临时文件的清理由目标
  端点的清理动作完成（`scope.md` §4.2 把"关闭 `Session`"与"失败 / 取消"列在同一格）。
- **引擎自己不落盘**：host↔host 两档的输出都是另一个端点（`scope.md` §4.1）。

### D5：host↔host 的两档都是"端点选择"，引擎不新增模式

| 档 | 源端点 | 目标端点 | 前置 |
|---|---|---|---|
| **B（优先）** | A 上的 `SftpClient` | 在到 A 的连接上开 `direct_tcpip(B.host, B.port)`，在那条流上握手 + 认证 + 开会话得到的 `SftpClient` | A 能连到 B |
| **A（回退）** | A 上的 `SftpClient` | B 上的 `SftpClient`（本机分别连 A 与 B） | 无 |

于是"优先 B、失败回退 A"是**选哪一对端点**，引擎一行都不用改；而 B 档消费的正是
ADR-0003 D9 的那条流，不另写一套通道逻辑（`AGENTS.md` 的"只实现一次"）。
**哪些失败算"B 不可用"、回退过程怎么被看见 —— 留给 plan 0703**：那是实现时才会暴露的细节，
现在写死只会变成一段与实际行为不符的描述。

### D6：并发 in-flight 的上限在引擎，且不得破坏落盘不变量

- 上游按请求 id 支持并发（§2 的最后几行），但"同时开多少个文件"是**我们的策略**：
  上限是引擎的一个参数（默认值写在配置模型里，同 ADR-0003 D13 的口径）。
- 并发下每个文件自己的临时名必须唯一 —— 同一批里两个同名文件不能撞在同一个临时名上。
  这条是 plan 0704 的判据之一，写在这里是因为它约束的是端点接口（临时名怎么生成由谁定）。

### D7：SFTP 是一个独立 `Session`，IPC 契约按"两侧"表达

- 资源的登记复用**同一张会话注册表**（ADR-0003 D6 的纪律：两张表分叉正是"查得到、却没人管"
  的来源）。
- 命令（`AGENTS.md` §5：这些名字由 Rust 生成到 TS，前端不得手写第二份）：

| 命令 | 作用 |
|---|---|
| `sftp_open` | 登记一个两栏 SFTP 会话，返回句柄 |
| `sftp_connect(handle, side, origin)` | 让某一侧对着 `origin`（**本机**或池里的一行）建立可用状态 |
| `sftp_list(handle, side, path)` | 列某一侧某个目录；返回规范化之后的路径与条目 |
| `sftp_transfer(handle, from, fromPath, to, toPath)` | 起一次传输，**立刻**返回编号（搬运在后台）|
| `sftp_transfer_cancel(handle, id)` | 推一次中止信号（清理由目标端点完成）|
| `sftp_transfers(handle)` | 这个会话发起过的传输与它们的进度 / 结局 |
| `sftp_sides(handle)` | 读一个会话两侧的状态与当前路径 |
| `sftp_sessions()` | 列出后端已经登记的**全部** SFTP 会话（含各自的传输）|
| `sftp_close(handle)` | 中止正在进行的传输、**等清理落地**，再回收两侧连接并注销这个 `Session` |

> 下面三条是**实现期补上的一组**（2026-09-15，plan 0702，见 §6）：D4 说引擎只认两个端点，
> 而"两个端点"在契约上就是"两侧各一个" —— 于是传输的输入输出都用 `side` 表达，
> 不需要第二个方向概念。`sftp_connect` 的第二个参数也从 `hostId` 变成了 `origin`：
> 一栏现在可以对着**本机**，而"池里的第 N 行"表达不了它。

> `sftp_sessions` 是**实现期补上的一条**（2026-09-15，见 §6）：面板是仅渲染的视图，
> 关面板不停会话 —— 那么面板重新打开时必须能找回那个会话，否则"关前端不影响后端执行"
> 就变成了"关前端等于失控"（会话还在，而没有任何地方能再操作它）。

- `side` 的过 IPC 表示是稳定短名 `left` / `right`。**命名理由**：它是**呈现位置**，
  而"哪一侧"在后端本来没有语义 —— 之所以允许它出现在契约里，是因为两侧的连接与失败
  必须能被分别指着说；后端不为它赋予任何其它含义（不参与资源归属，资源归那个 `Session`）。
- 只读探针 `sftp` 报两侧的状态与当前路径 —— 判据的读数口（`AGENTS.md` §7 的探针纪律）。
- **不依赖终端**：SFTP 自己建立连接，不引用任何终端的 `Session`
  （`scope.md` §5.1 的第 3 条）。

## 4. 遗留与未决

- **`russh-sftp` 的扩展支持面只在测试服务端上验过**：`limits@openssh.com` /
  `fsync@openssh.com` 缺失时的降级路径未与真实 `sshd` 对照（测试服务端**一个扩展都不声明**，
  所以降级那一侧每次都被走到，但"与真实 `sshd` 对上"仍是空白）。
- **吞吐没有数字**：读写路径已经实测通了（4 MiB 在回环上 2.5 秒，那是测试服务端每块
  20 ms 的人为延迟），但"多快"这件事没有基线 —— 它是 plan 0704 的判据之一。
- **临时名的占用探测不是原子的**：挑名字用一次"存不存在"的查询，然后创建；两批并发
  传输理论上可以撞上（多开 16 个也不解决，只是变窄）。0704 做并发时要换成
  `CREATE|EXCLUDE` 那种由对端保证的唯一性，或者把临时名带上一个进程内唯一的部分。
- **"B 档回退 A 档"的判据未定**（见 D5）—— 落在 plan 0703。
- **断点续传（`later`）与本文的形状有一处冲突**：D4 的"临时名 + 原子重命名"假定一次传输
  要么完成要么删掉；续传需要保留 `.part` 并记录偏移量。两者不是不相容，但那时要重新回答
  "临时文件归谁清理"。

## 5. 失败档（`akasha-ssh` 侧）

SFTP 的失败新增**一档**，与既有的 `Listen`（本机端口没拿到）/ `RemoteListen`（服务端那个
端口没拿到）/ `Forward`（跳板拒绝转发）并列，判据同 ADR-0003 §7 的分法 —— 按**用户的
下一步动作**分：这一档说的是"会话建不起来 / 会话用不了了"，用户要看的是对端与网络，
而不是自己的端口或转发规则。

plan 0702 又补上**第二档** `SshError::File { path, reason }`：**端点上的一个文件操作**
（列目录、打开、读、写、重命名、清理）失败了。两者分开的理由与上面同一条：
`Sftp` 那一档要把人引到对端的 `sshd_config` 上，而"这个文件这一步没成"要引到**那个路径**
上 —— 用会话那一档报一次重命名失败，排查方向会整个偏掉。本机与远端共用这一档
（用户的下一步动作是同一个：去看那个文件），"是本机还是对端"由路径本身说清。

## 6. 修订记录

- **2026-09-15**：初稿（plan 0701 开工时）。D1–D7 一次写全；D5 / D6 只定职责边界，
  判据留给对应 plan。
- **2026-09-15**（plan 0701 落地时）：D7 的命令表补上 `sftp_sessions()` —— 理由见该表下面
  那段；同时把"上游 `request_subsystem` 只发送、不等回复"这条事实写进 D2（它决定
  "对端没开 SFTP"表现为**等第一条回复超时**，而不是一句明确的拒绝）。
- **2026-09-15**（plan 0702 落地时）：D4 的端点接口定形 —— `Endpoint` 的每个方法返回
  `Box<dyn Future>`（`async fn` 不 dyn 兼容），**临时名的取名规则归引擎那一层**、
  挑一个没被占用的归端点；D7 的命令表补上传输那三条、`sftp_connect` 的第二个参数由
  `hostId` 改成 `origin`（一栏可以对**本机**）；§5 增补 `SshError::File` 一档；
  §4 的"大文件读写未实测"换成"吞吐没有基线"，并记下临时名占用探测不是原子的。
  落地时实测出的三处问题（`watch::Sender::send` 无订阅者时不写入、协议类型位互相重叠、
  服务端响应 id 必须回填）不在本文的范围里，记在 plan 0702 的实施记录。
