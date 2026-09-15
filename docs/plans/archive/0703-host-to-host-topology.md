# Plan 0703: host ↔ host（B 档优先，回退 A 档）

- **关联**：ROADMAP 阶段 7 ·「host ↔ host：**优先 B 档（`direct-tcpip`），失败回退 A 档（内存 relay）」」
- **前置**：plan 0702（传输引擎）· plan 0505（`direct-tcpip` 原语）· ADR-0006 D5（两档都是"端点选择"）
- **状态**：已完成（2026-09-15）

## 目标

两栏都是主机时也能搬文件。两档都实现，运行时**优先 B**（`scope.md` §4.1）：

| 档 | 目标端点从哪来 | 前置 |
|---|---|---|
| **B（优先）** | 在**到源那一栏所连主机**（记作 A）的连接上开 `direct_tcpip(B.host, B.port)`，在那条流上握手 + 认证 + 开会话 | A 能连到 B |
| **A（回退）** | 本机分别连 A 与 B（`sftp_connect` 本来就有的那条连接） | 本机必须能直达 B |

本 plan 把 ADR-0006 D5 留给它的两件事定下来，并做成可观测的：

1. **档位在哪一刻定、由谁定** —— 在**连接目标那一栏**那一刻定：连之前先看另一栏，若它已经连上
   一台**不同的**主机（A），就先试 B 档（本机 → A → B 这条链），试不通再回退本机直连（A 档）。
   于是"这次传输走的是哪一档"是**目标端点怎么来的**这件事的后果，引擎一行都不用改（D5）。
2. **哪些失败算"B 档不可用"** —— **那条隧道建链尝试的任何失败**都算：A 拒绝转发、
   A 连不上 B、到 B 的认证失败、超时。**不分类**：对"要不要回退"这个问题它们没有区别。
   回退可观测 = 那一栏的 `through`（经哪台直通）与 `throughFailure`（回退原因），
   加上传输记录里的 `via`。

**两档都不落盘**：本机这一侧只有内存里的字节 —— 两档的端点都是**远端**端点，引擎拿不到本机
路径（D4）。目标那一台照旧"临时名 → 原子重命名"（`scope.md` §4.2）。

## 非目标

- **"真不中转"**（在 A 上执行 `rsync` / `scp`）：`scope.md` §10 列为 `no`
  （要 A↔B 互信，进度只能解析远端输出）
- 断点续传（`later`）与并发 in-flight（plan 0704）
- **反向不走 B 档**：通道开在"到源那一栏所连主机的连接"上，于是 B 档优化的方向是**源 → 目标**；
  目标是可直达的那一台时，这次传输的形态就是本机内存中转。方向由用户选的来源决定，
  不由"哪台更近"猜。

## 判据（ROADMAP 原文）

「A 无法直连 B 时**自动走 A 档**；两档**均不落盘**」

## 前置检查

```bash
# ① 0505 的原语在位（B 档只消费它，不另写一套通道逻辑）
grep -n "pub async fn over\|pub async fn direct_tcpip\|pub async fn sftp" \
  src-tauri/crates/akasha-ssh/src/forward.rs
# ② 测试服务端能同时扮演"只有它认识 B 的跳板"与"一栏服务端"：中继表 + SFTP 根
grep -n "pub relay: Vec<Relay>\|pub sftp: Option<Vec<SftpItem>>" \
  src-tauri/crates/akasha-ssh/src/testing.rs
# ③ 0702 立的那条拒绝还在（本 plan 要拆掉它）
grep -n "fn reject_host_to_host" src-tauri/src/sftp.rs
```

## 步骤

1. **建链：多一跳前缀**（`src-tauri/src/ssh.rs`）
   `connect_connection_via(ssh, vault, via, host_id, cancel)`：`hops` = A 那一行自己的跳板链
   ++ `[A]` ++ B 那一行自己的跳板链，终点 = B 那一行的 `SshConnect`，交给**已有的**
   `SshConnection::connect_via_until`。于是"经 A 直通 B"与跳板是同一条实现（0505 的
   "只实现一次"），新代码只有"把两行的材料拼起来"这一步。
   *验证*：第 5 步的库内用例从链的另一头证明它通。
2. **连接目标那一栏时先试隧道**（`src-tauri/src/sftp.rs`）
   另一栏已连上一台**不同的**主机时：`connect_connection_via` + `sftp()`；成功就把这一栏标成
   `through = Some(A)`。失败则 `tracing::warn` 记一条、原因放进 `throughFailure`，再走原本的
   本机直连（`through = None`）。两条都不成时 `failure` 报**直连那一条**的原因。
3. **契约**：`SftpSideInfo` 多 `through` / `throughFailure`，`SftpTransfer` 多 `via`
   （这次传输的目标端点是经哪台直通到达的；`None` = 本机内存中转）。
   `SftpError::Unsupported` 与 `reject_host_to_host` 整份删除 —— host ↔ host 不再是"还没做"。
4. **传输**：`sftp_transfer` 不再拒绝两栏都是主机；登记那一刻把目标那一栏的 `through` 抄进
   `via`（传输是历史记录：一侧的连接事后可以被换掉，而"这次走的是哪档"不该跟着变）。
5. **库内用例**（`akasha-ssh/tests/host_to_host.rs`）
   - **B 档**：跳板（中继表把 `akasha-sftp-only.invalid:22` 指向目标，自己也有 SFTP 根）
     与目标两台真服务端；客户端连跳板、在它上面 `over` 目标，两条 SFTP 会话一起喂给同一个 `transfer()`
   - **负控**：跳板没有那条映射时 `over` 必须失败（`SshError::Forward`）—— "B 档不可用"的触发点
   - **A 档**：本机分别连两台，同一个 `transfer()` 照样成功
   - 两档都断言：目标盘上是**最终名**且字节相同、源盘一个条目都没多、两处都没有临时名
6. **E2E**（`src-tauri/tests/sftp_host_to_host.rs`，新目标；加进 `E2E_TARGETS`）
   一条用例走两段：B 档（本机**解析不了** B 的名字）与回退（A 拒绝那条转发）
7. **前端壳层**：那一栏显示"经 **X** 直通"或回退原因；传输那一行带 `data-transfer-via`

## 验收命令

```bash
# 1. 库那一层：B 档与 A 档各一次真传输 + "B 档不可用"的负控
just test host_to_host
# 预期：全绿；负控那条报 SshError::Forward

# 2. 端到端：真 app + 两台进程内服务端（跳板带中继表）
just test-e2e
# 预期（sftp_host_to_host 那一段）：退出码 0，且打印
#   B 档：跳板收到 1 条 direct-tcpip → akasha-e2e-sftp.invalid:22，中继搬过 N 字节
#   B 档：目标盘上 far.bin 的字节数与源一致
#   回退：经 … 直通失败（…），已改为本机直连；传输仍然成功
# 判据的读数口（app 在跑时）：app_state { probe: "sftp" }
#   → sides[].through / sides[].throughFailure / transfers[].via

# 3. 门禁
just ready      # 预期：fmt-check / lint / test / deny-offline / gen-types-check / docs-check 全绿
pnpm build      # 预期退出码 0
```

## 回滚

新增两个测试文件整份删除；改动面集中在 `src-tauri/src/sftp.rs`（连接那一档与传输登记）、
`src-tauri/src/ssh.rs`（一个建链函数）、`session.rs` 的一处转发与面板。契约回滚 = 把
`Unsupported` 与 `reject_host_to_host` 装回去，`src/ipc/bindings.ts` 必须一起回滚
（`gen-types-check` 会拦住不一致）。

## 实施记录（2026-09-15）

### 判据实测

库内（`host_to_host`，3 条 / 0.35 s）：B 档把 200 KiB 从跳板那台搬到目标那台（目标的名字
`akasha-sftp-only.invalid:22` 只存在于跳板的映射表里、本机解析不出来）—— 跳板记到**恰好一条**
`direct-tcpip`、`relayed_bytes > 0`，目标真盘上字节逐字节相同且目录里没有临时名，源那一台
**一个条目都没多**；负控两半（不经跳板直连那个名字报 `Connect`；跳板没有那条映射时报
`SshError::Forward` 且 `relayed_bytes = 0`）；A 档（本机分别连两台）走同一条判据，且一次转发都没有。

端到端（`just test-e2e` 退出码 0，**30 个用例 / 23 个目标**；`sftp_host_to_host` **5.29 s**，
它在不同轮次实测过 5.3～11.5 s）：

```
跳板 127.0.0.1:42705 · 目标 akasha-e2e-sftp.invalid:22（跳板表里指向 127.0.0.1:41731）
左栏连上跳板，问到过 ["hostKey:SHA256:qHhE…", "credential:e2e@127.0.0.1:42705"]
右栏经跳板连上目标，问到过 ["hostKey:SHA256:mZKh…", "credential:e2e@akasha-e2e-sftp.invalid:22"]
B 档：跳板收到 1 条 direct-tcpip → akasha-e2e-sftp.invalid:22，中继搬了 55776 字节；目标盘上 alpha.bin 的字节数与源一致
右栏回退到本机直连，问到过 ["hostKey:SHA256:mZKh…", "credential:e2e@127.0.0.1:41731"]
回退：经跳板直通失败（转发到 127.0.0.1:41731 失败：Failed to open channel (AdministrativelyProhibited)），已改为本机直连
A 档：回退之后 gamma.bin 同样落到了目标盘上，传输记录里没有直通那一项
```

`just ready` **6/6**；`just test` **353 passed**（`akasha` 79 + `akasha-core` 30 + `akasha-pty` 39 +
`akasha-ssh` 78 + `akasha-store` 127）；`pnpm build` 退出码 0（869.46 kB / gzip 239.29 kB）。

### 引擎一行都没改，`akasha-ssh` 的 `src/` 一个字也没动

ADR-0006 D5 的预判成立：B 档要的三样东西**全都已经在库里** —— `direct_tcpip`（0505）、
`SshConnection::sftp()`（0701 起）、`connect_via_until(hops, options)`（0606 起）。
新代码只有两处："把两行的材料拼成一条链"（`src-tauri/src/ssh.rs::connect_connection_via`）
与"连目标那一栏之前先试它"（`sftp_connect`）。

### 与 ADR-0006 的偏差（已记进那份 ADR 的修订记录）

| 偏差 | 为什么 |
|---|---|
| 新增 `through` / `throughFailure` / `via` 三个字段 | D5 只说"回退过程要能被看见"，没写"看见"落在哪；探针与界面都要读它 |
| 删掉 `SftpError::Unsupported` 与 `reject_host_to_host` | host ↔ host 不再是"还没做"。留着一个永远出不来的错误档就是在文档里留一句假话（同 0604 删 `TunnelError::Unsupported`） |
| `scope.md` §4.1 的"带宽减半"改成"可直达性要求" | 两档下文件都要过一次本机（读源一遍、写目标一遍）—— 按字面读会让人去优化一个不存在的一半带宽 |

### 落地时看清的三件事

1. **`SshConnection::over` 不持有承载它的那条连接**：它只造下一跳（`under` 是空的），链的存活
   归调用方。app 那条路走 `connect_via_until`（`chain` 把整条链放进 `under`），所以不受影响；
   但"用 `over` 单独建一跳"看起来像一条能独立存在的连接 —— 库内用例因此自己把承载者留在
   作用域里，并把这一点写进文件头。
2. **一栏重新连接期间，界面上那行状态是上一次的**（`data-sftp-state` 要等命令返回才刷新），
   E2E 第一次就是这么红的：它按界面判断"连上了"，而那一刻后端还在 `connecting`。
   处置：面板在命令在途时按后端的事实显示 `connecting`（`prepare_connect` 在任何 I/O 之前
   就已经置上它），E2E 的完成条件改读 `sftp` 探针（`state` + `origin.id`）。
3. **判据读后端之后，界面还要一段时间才跟上来**：同一个 E2E 在**整份门禁第二次执行**时又红了一次
   —— 那次断言的是界面上那句回退原因，而面板要等命令返回之后才刷新两侧状态，读得早会读到空串。
   处置：**先等界面出现那一行，再断言它的内容**（`wait_js` + 读文本）—— 面板的刷新有自己的节奏，
   断言不能抢在它前面。
   教训与第 2 条是同一件事的两半：**后端的事实可以立刻断言，界面的呈现要先等到它出现**。

### 遗留

- **两档都没有与真实 `sshd` 对照过**：`AllowTcpForwarding no` / `PermitOpen` 白名单 / 转发通道
  的缓冲特性都只有 RFC 与自建服务端作依据（ADR-0006 §4 已记）。
- **"两档均不落盘"的判据是形状 + 目录事实**：两档的端点都是**远端**端点（引擎拿不到本机路径）、
  源那台目录没变、目标那台只有"临时名 → 最终名"一次改名。"先下到本机再上传"那种实现形态
  无法被直接证伪（没有本机路径可读）。
- **B 档在 A 上多开一条连接**（隧道那条；A 那一栏自己还有一条）。复用另一栏那条要求把连接的
  所有权变成可共享的（`Arc`），而"独立一条"换来的正是两侧互不影响（D3）——
  代价是 A 上多一次认证（凭据与主机密钥都已缓存，用户看不到提示）。
- **隧道那一跳的认证只用口令验过**：E2E 与库内用例走的都是口令认证；`agent` / `publickey`
  那一档没有专门的用例（它们与直连共用同一份 `connect_plan`，"凭据怎么来"只有一处实现）。
