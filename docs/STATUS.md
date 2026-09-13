# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-13

## 摘要

**阶段 2「端到端最小终端」5/5 完成**；**阶段 3「托盘与应用生命周期」6/6 完成**；
**阶段 4「存储与凭据池」9/9 完成**；**阶段 5「SSH 栈」6/6 完成**（0501 ADR / 0502 连接认证 /
0503 known_hosts / 0504 接入 IPC 与前端 / 0505 `direct-tcpip` 原语 + 跳板 /
**0506 `~/.ssh/config` 受限子集导入**）；
下一步是 **阶段 6 plan 0601（隧道实体 + 状态机）**。

阶段 4 的八项（每项一句）：**SQLCipher 加密库可打开**（0401）、**口令只从一条路径进入且可真正验证**
（0402 —— 拆分"打开"与"新建"；此前在文件不存在的路径上**任何口令都能打开**）、
**口令在内存中同样受保护**（0406）、**四类池可增删改查**（0403）、**库中数据可导出也可还原**
（0404）、**解锁与锁定构成完整生命周期**（0407 —— 真实 app 上 `VmLck` **0 → 176～192 → 0 kB**）、
**目录迁移后数据仍在且可用**（0405）、**便携目录不可写时拒绝启动**（退出码 2）。

**阶段 5 的形状先固化在 [ADR-0003](./adr/0003-ssh-stack-and-resource-model.md) 中**（plan 0501）：
`russh` 的版本与 feature、运行时归属、`Transport` 在 SSH 上的映射、`direct-tcpip` 原语的形状、
隧道状态机与重连判据全部**带出处**确定。之后四块依次落地：**连接 + 认证**（0502）、
**主机密钥的信任策略 + 本仓库首次库格式迁移**（0503）、**接入 IPC 与前端**（0504）、
**`direct-tcpip` 原语 + 跳板（ProxyJump）**（0505）。

### 阶段 5 完成情况（按依赖顺序）

| plan | 完成内容 | 判据落点 |
|---|---|---|
| 0501 | ADR-0003（D1–D16，带出处的调研 + 资源模型） | `docs/adr/0003`，状态「实现中」 |
| 0502 | `akasha-ssh` 连接 + 认证（D7 的顺序、D8 的内存凭据缓存） | **crate 层**：进程内 SSH 服务端，三个连接共享一次凭据询问 |
| 0503 | known_hosts 三态判定（D11）+ 库格式 v2 与首次迁移 | **crate 层**：策略 + 库契约；迁移用真实 v1 库 |
| 0504 | SSH 接入 IPC / 前端：带目标的会话命令、提问往返、提示界面 | **真实 app**（`ssh_session` E2E + 测试进程内的服务端） |
| **0505** | **`direct-tcpip` 原语**（D9 的形状 = 一条流）+ **跳板链**（池中的 `jump_id` 生效） | **crate 层**（两个真实服务端 + 负控）+ **真实 app**（`ssh_jump` E2E） |
| **0506** | **`~/.ssh/config` 受限子集导入**（三档边界：六条导入 / 局部指令警告 / 其余整份报错）+ 池的**第一条写路径** | **crate 层**（29 条解析 + 5 条落库，纯函数）+ **真实 app**（`ssh_config_import` E2E：导入的条目经跳板真实连通） |

**本轮（plan 0506）接通了"用户已有的配置"**：`akasha-store::sshconfig` 是**纯函数**
（一段文本 → 一批条目），求值只有一条规则 —— 每个参数**首次取到的值生效**（写在前的 `Host *`
会压住其后的具体条目）。其余工作都在**边界**上：可识别的局部指令（保活、算法、转发、日志……）
逐条警告；`Match` / `Include`、会改变目的地或信任来源的指令，以及**表中未收录的**一律**整份报错**。
导入的 `ProxyJump` 已可用：E2E 使用**导入得到的条目**连通了仅对跳板机可见的主机。
⚠️ 私钥**不导入**：`IdentityFile` 只令条目落成"公钥认证 + 密钥在 ssh-agent 中"。

### 阶段 5 之前那些跨阶段的结论（还在生效）

**关闭窗口的语义由配置与托盘可用性共同决定**（0302 + 0303）：

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
| `just test` | **279 tests run: 279 passed**（`akasha` **67** + `akasha-core` 15 + `akasha-pty` 39 + `akasha-ssh` **31** + `akasha-store` **127**）。⚠️ `akasha` 的 67 条包含 `tests/` 下的集成目标（未设置 `VICTAURI_E2E` 时它们只输出原因并返回 —— 其中 `portable` 3 条、`ssh_session` 1 条、`ssh_jump` 1 条、**`ssh_config_import` 1 条**） |
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
| ↑ **`just test-e2e` 全部通过** | 退出码 **0**：**21 个 E2E 用例**（`E2E_TARGETS` / `E2E_TARGETS_EXIT` 共 18 个 + `E2E_SELF_APP` 的 `portable` 3 个，含新增的 `ssh_config_import`）。`ssh_session` / `ssh_jump` / `ssh_config_import` 均排在 `vault_unlock` **之后**（三个用例操作同一个库文件） |
| ↑ **判据：主机密钥变化即拒绝，未见过的询问一次**（plan 0503） | ✅ `akasha-ssh` 的 7 条：未知且无人可问 → `HostKeyUnknown`（携带用于核对的指纹，且**认证尚未开始**）；确认 → 写入缓存，**第二个连接 0 次询问**；记录不匹配 → `HostKeyChanged`（**两个指纹都在**）且**不发起询问**；用户拒绝 → 拒绝连接且**不记录**；用户文件中已认可 → 连通且文件**逐字节未变** |
| ↑ **本仓库首次格式迁移**（plan 0503） | ✅ `akasha-store` 的 6 条：`DDL_V1` 构造出**真实 v1 库** → `open` 之后 `user_version = 2`、五张表存在、**该 host 行仍在**；再次打开当前格式的库**不写入任何字节**；缺表的 v1 **不迁移**；加密导出与明文导出两条还原路径均**升级副本、来源逐字节不变** |
| ↑ **判据：同主机三个连接仅询问一次凭据**（plan 0502） | ✅ `three_sessions_ask_for_one_credential`：`provider.calls() == 1`、缓存 `len() == 1`、服务端三次均收到**同一口令** |
| ↑ **认证顺序由协议交互验证**（plan 0502） | ✅ 服务端记录的序列：`publickey → password`、`publickey → keyboard-interactive`；agent 不可用时序列中**没有** `publickey` |
| ↑ **`nodelay` 实际生效**（plan 0505 修正） | ✅ `tcp_stream` 自建 TCP 时显式 `set_nodelay(true)`（问题 #120：上游仅在 `client::connect` 中读取 `Config::nodelay`，而两条路都使用 `connect_stream`） |
| ↑ **`Cargo.lock` 增量仅一行**（plan 0504 / 0505） | ✅ 新增 `akasha → akasha-ssh` 这条边**只增加一行**；0505 **未增加任何行**（无新依赖，`rand` 早已是 `akasha-ssh` 的真依赖） |
| `pnpm build`（tsc + vite build） | 退出码 0；产物 **853.84 kB / gzip 234.67 kB**（+3.8 kB：选择器中增加导入面板） |
| `just docs-check` | 全部通过（ROADMAP 58 个条目 ≤3 行且无代码块 / 50 份 plan ≤200 行且索引一致） |
| `ast-grep scan` + `ast-grep test` | 均退出 **0**（本轮未新增 / 修改规则） |
| **三条 unsafe 注释 lint**（clippy，位于 `just lint`） | 退出码 **0**；三条各以一个探针验证其**确实会失败**（探针用后即撤） |
| ↑ **解锁 / 锁定 / 内存回收**（0407） | ✅ 真实 app 上 `VmLck` **0 → 176～192 → 0 kB**；进程内存扫描（带正对照）：口令 `1 → 2 → 2 → 1` 处、派生密钥 `3 → 1` 处 |
| ↑ **导出与还原**（0404） · **目录迁移**（0405） · **退出零残留**（0204/0205） | ✅ 三项判据仍然全部通过（`just portable` **3 passed**；`app 已退出` + `零残留`） |
| ↑ **终端 / 会话判据（未退化）** | `renderer = webgl`；写入 8 MB 数据后仍可交互；raw 通道 10.73 MB / 172 批；收尾帧 1 个、console 零异常 |
| ↑ **§7 的 registry 一条：当前不可满足**（问题 #82） | `get_registry` 返回 **`[]`**（命令均未标 `#[inspectable]`）。**替代证据**为真实路径上的 `invoke_command` 成功 —— 本轮的新命令均以此验证 |
| `cargo tree -p akasha-core \| grep -c tauri` | **0**（分层成立） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数据，本轮未重新运行**） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉取 RustSec 数据库。

## 待验证（本地或沙箱环境无法执行）

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

## 当前基线（2026-09-13 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` / `akasha-store` / `akasha-ssh` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，问题 #29） |
| 后端模块 | `bindings` / `session` / `tray` / `config` / `lifecycle` / `single_instance` / `vault` / `watchdog` / `ssh`（长住状态 + 那条命令 + 跳板链 + 库内 known_hosts 适配器） / `prompt`（提问往返） / `pools`（池的读取 + **导入**） |
| **命令清单** | `greet` · `vault_status` / `vault_unlock` / `vault_lock` · `vault_hosts` · **`import_ssh_config`** · `open_session` · `open_ssh_session` · `write_session` / `resize_session` / `close_session` · `ssh_prompt_credential` / `ssh_prompt_host_key` / `ssh_prompt_cancel`（事件：`session_ended` · `ssh_prompt` / `ssh_prompt_dismissed`）。**plan 0505 没有新增命令**（跳板是既有那条命令内部多走几跳）；**0506 新增一条**，而它是**池的第一条写路径** |
| **probe** | `lifecycle` → `{close_behavior, tray_ready, close_action}`（没登记时 `{"initialized":false}`，问题 #93）；`single_instance` → `{registered, activations}`；`sessions` → `{live, registered}`（SSH 没有本地进程，"零残留"只能看注册表）。**库没有 probe**：状态本身就是命令（`vault_status`） |
| 出字节路径 | PTY / SSH read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write`。**两条载体共用同一段输出路径的后半段**（`session::open_terminal`） |
| **`direct-tcpip` 原语**（plan 0505，ADR-0003 **D9**） | `akasha-ssh/src/forward.rs`：`SshStream`（自己实现 `AsyncRead + AsyncWrite`，**不把 `russh::ChannelStream` 漏进公开签名**）+ `SshConnection`（已认证、**没有通道**的连接，持有 `Handle`）+ `SshConnection::direct_tcpip(host, port)`。三处消费者（跳板 / `-L` / SFTP B 档）使用的都是**这条流**；`-L` 与 SFTP 尚未接上（plan 0602 / 0703） |
| **跳板链**（plan 0505） | 库侧：`hosts::jump_chain`（**目标在前**、有界、成环报 `StoreError::JumpChain`）。app 侧：`ssh.rs::plan_chain` 按 id 解出各跳，`open_ssh_session` 再将其反转为"最外层在前"后调用 `SshTransport::connect_via(runtime, hops, target)`（`connect` 即空链的那一次）。**每一跳各一份 `SshConnect`**（各自询问凭据、各自校验主机密钥）；链上每一跳是一个 `SshConnection`，随 `Established::carriers` **move 进最终那条连接的 `pump` task** —— "task 结束 = 整条链结束"，收尾按**最内层先断** |
| **连接的 originator** | `direct-tcpip` 要求带发起方地址（RFC 4254 §7.2）：用**最外层那条 TCP 的本地地址**（我们唯一真知道的），往下每一跳复用；拿不到就空串 + 0（不得伪造一个看似真实的地址写入对端日志） |
| **SSH 的 IPC 层**（plan 0504） | `src-tauri/src/ssh.rs`：app 启动时建**一个**专用 tokio runtime（**4 个 worker**，D2）；`open_ssh_session` 是 **async 命令**（不阻塞 IPC），内部起一条**普通 `std::thread`** 运行同步门面（`spawn_blocking` 的线程**也算** tokio 上下文，会触发 `BlockingInsideRuntime`），结果经 `tokio::sync::oneshot` 回来。`SshConnect` 的材料按池行组：`password` → 不用 agent、不带钥匙；`agent` → 只用 agent；`publickey` + `key_id` → 那一把钥匙（PEM → 受保护页 → `KeyCandidate`，标识 `key#<id>`） |
| **提问往返**（plan 0504，ADR-0003 **D16**） | `src-tauri/src/prompt.rs`：`Prompts`（`Arc` + 待答表 + 可注入的发布口）；事件 `ssh_prompt`（判别式：`hostKey` / `credential`）+ `ssh_prompt_dismissed`；三条回答命令；编号从 1 起、只增不减；**超时 120 s → 拒绝 + 撤回**；答过 / 超时的 id → `PromptError::Gone`；**主机密钥那一问只认"接受 / 拒绝"**，超时 / 取消 / 答错类型一律 `Err(HostKeyUnknown)`（= 拒绝连接）。⚠️ 跳板链上**每一跳各产生一轮**（密钥 + 口令），E2E 实测四问按序 |
| **库内主机密钥缓存**（plan 0504 接线） | `VaultHostKeys`：`Vault` 可 `Arc` 克隆，`with_conn` **短借**连接；库处于锁定状态 → `SshError::HostKeyCache` → **拒绝连接**（不视为未知）。⚠️ **不得在持锁期间连接**：`remember` 会在连接中途回锁库（跳板链因此先**一次读完整条链**再开始连接） |
| **库的解锁状态** | `Vault { inner: Arc<Mutex<Option<Unlocked>>> }`（`Clone`）；`Unlocked { conn, passphrase }` 同生共死。借库的失败分两种（`ConnError`：`Locked` / `Store`）—— 因为 SSH 那条路要单独认出 `NoSuchRow`（"所选主机不存在"） |
| **`akasha-ssh` 的形状** | 十个模块：`target` / `credential` / `keys` / `handshake`（`handshake<S>` = 一跳的握手 + 认证，**底层流由调用方提供**） / `known_hosts` / **`forward`（D9 原语 + `SshConnection`）** / `transport` / `testing`（进程内测试服务端，**仅用于测试**；支持 `direct-tcpip` 的中继与拒绝两条分支） / `auth` / `error` |
| **错误分域** | `akasha-ssh`：`HostKeyCache`（库那一侧无法读取缓存）、**`Forward { host, port, reason }`**（跳板拒绝 / 目标不可达 —— 与"无法连接跳板机"分开）。app 侧 `SshIpcError`：`Locked` / `NoSuchHost` / `Failed { kind, message }`（`kind` = `hostKeyChanged` / `hostKeyRejected` / `hostKeyUnknown` / `hostKeyCache` / `auth` / `connect` / **`jump`** / `other`）/ `Internal` —— **前端按 `kind` 分辨**，不匹配消息字符串 |
| **前端结构** | `src/ipc/`（`session.ts` / `prompts.ts` / `hosts.ts` —— 唯一允许碰后端的目录）、`src/tabs/`、`src/terminal/`、`src/ssh/`（主机选择器 + **导入面板** + 提示面板）、`src/App.tsx`。标签页 `kind`：`terminal` / `ssh`（**都有关闭按钮**，规则写成 `CLOSABLE` 清单） |
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

- [ ] **下一步 = 阶段 6 的 plan 0601（隧道实体 + 状态机）**：该 plan 目前是**未规划（骨架）**，
  需先补齐「步骤」与「验收命令」。⚠️ 阶段 6 触及以上**不可逆点**之一（隧道状态名与事件名会进入
  `bindings.ts` 与前端）—— 开工前先读 ADR-0003 §10 与 D12。
- [ ] **阶段 5 之后仍有两处界面缺口**（不是缺陷，而是尚未规划的工作）：**解锁界面**（当前 SSH 的
  真实路径上，解锁由 E2E 以 `invoke_command` 完成）与**主机池的增删改查界面**。⚠️ plan 0506 只
  补上了**导入**这一条写路径：目前一台机器的端口 / 用户名 / 跳板在界面上**无法修改**（只能修改
  配置后重新导入并指定 `overwrite`，或直接改库）。
- [ ] **`-L` / SFTP B 档尚未接入 `direct_tcpip`**：形状已定（一条流），实际适配在 plan 0602 / 0703。
- [ ] **降级路径未实测**：v2 库在旧版本程序中会以 `UnsupportedVersion { found: 2 }` 被拒绝（有意为之）。
- [~] **plan 0102（CI 平台矩阵）**：本地部分完成，最终判据 = 推送后三个 job 全部通过，当前阻塞于仓库无 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测，剩余 CI 三平台格子
- [ ] **正式 UI**：等待设计稿（见上文「UI 现状」）—— 没有验收标准，因此**不进入 ROADMAP**

> 阶段 5 的编号按依赖重排过（2026-09-13）：0503 known_hosts · 0504 接入 IPC / 前端 ·
> 0505 `direct-tcpip` 原语（原 0503）· 0506 `~/.ssh/config` 导入（原 0504）。
> 理由：信任策略属于**接口形状**（先行），而原语的消费者都需要先有一条**从 app 建立起来的**
> SSH 会话才能验证。编号与执行顺序现已一致，索引中有一段重排说明。

### 本轮完成（plan 0506：`~/.ssh/config` 受限子集导入）

**判据（ROADMAP 原文）**：含 `Match` 的配置产生**明确报错**，而非静默误解析。

- [x] **三档边界确定**（ADR-0003 **D14** 展开，§14 记一行）：六条导入 / **可识别的局部指令逐条警告
  并继续** / `Match`、`Include`、会改变目的地或信任来源的、`IgnoreUnknown`、**表中未收录的**一律
  **整份报错**（一次列全，带行号）。判据取**后果**这一维度（是否连到别的机器、是否改变信任的
  密钥），而非"是否认识"；兜底方向是**默认报错**
- [x] **求值语义按 OpenSSH 实测**（`ssh -G`）：每个参数**首次取到的值生效**（写在前的 `Host *` 会压住
  其后的具体条目 —— 按"一个 `Host` 块 = 一行"解读会**静默连接到错误端口**）；文件开头到第一个
  `Host` / `Match` 之间为全局段；关键字不区分大小写而 `Host` 模式区分；通配块不产生条目
- [x] **解析器是纯函数**（`akasha-store/src/sshconfig.rs`）：不读盘、不读环境、不碰库 —— 整套语义
  因此在没有 app / 库 / 网络的环境下由 29 条用例固化
- [x] **落库使用单个事务**（`pools/import.rs`）：先全部插入（`jump_id = NULL`）再挂链，成环检查**复用**
  `update_host` 的那一份（`insert_host` 的"新行没有入边"论证对导入不成立 —— **plan 之外的发现**）；
  同名默认不动、补建的跳板条目**永不覆盖**
- [x] **池的第一条写路径接入 IPC 与前端**：`import_ssh_config(path, overwrite)` + 报告
  （新增 / 更新 / 跳过 / **未生效的指令** / 结构性说明）+ 界面上列出**支持集**
- [x] **判据实测**（真实 app）：见上表三行 —— 含 `Match` 的整份报错且一行不写 / 正常配置导入的两条
  **经跳板真实连通** / 私钥路径未进入库
- [x] **门禁**：`just ready` **6/6**；`just test` **279 passed**（+36）；`just test-e2e` **退出码 0**
  （21 个用例）；`pnpm build` 退出码 0；`Cargo.lock` 零增量
- [x] **文档同步**：plan 0506 置「已完成」并 `git mv` 进 `archive/`（索引与 ROADMAP 指针同步）；
  ADR-0003 D14 展开 + §14 记一行；`scope.md` §3 / §8 各补一句；本文件覆盖写

### 上一轮完成（plan 0505：`direct-tcpip` 原语 + 跳板）

**判据（ROADMAP 原文）**：ProxyJump 可连通**仅对跳板机可见**的目标。

- [x] **原语形状按 D9 落地**：`SshStream`（一条 `AsyncRead + AsyncWrite` 的流）+ `SshConnection`
  （已认证、没有通道的连接）+ `direct_tcpip`；`establish` 拆成 `handshake<S>`（**底层流由调用方提供**）
  与 shell 收尾 —— 因此"以通道作为下一跳的网络"与"直连"走**同一条路**
- [x] **`Handle` 归属确定**：跳板路径**自己持有连接**（另一条路是给 `SshTransport` 开受控借用口）
  —— 句柄不出 `akasha-ssh`，也不需要回答"谁在何时可以访问它"
- [x] **整条链一起生灭**：carriers move 进最终那条连接的 `pump` task，收尾**最内层先断**
- [x] **错误分域**：新增 `SshError::Forward`（跳板拒绝 / 目标不可达）→ `SshFailureKind::Jump`
  —— "无法连接跳板机"与"跳板机无法连接目标主机"必须区分，否则排查方向将被误导
- [x] **库侧**：`hosts::jump_chain`（目标在前、有界、成环报错）；**读路径同样拦截**环与深度
  （写入路径无法阻止直接改库，而环会使连接**永久阻塞**）
- [x] **判据实测**（真实 app + 测试进程内两台服务端）：见上表四行 —— 名字不可解析 / 四条提示按序 /
  跳板记到 1 条 `direct-tcpip` / 字节到达目标 / 关闭标签页后链断开
- [x] **修正一处未生效的配置**：`Config::nodelay` 在 `connect_stream` 这条路上从未生效
  （问题 #120），现改为自建 TCP 时显式关闭 Nagle
- [x] **门禁**：`just ready` **6/6**；`just test` **243 passed**（+8）；`just test-e2e` **退出码 0**；
  `pnpm build` 退出码 0；`Cargo.lock` **零增量**
- [x] **文档同步**：plan 0505 置「已完成」并 `git mv` 进 `archive/`（索引与 ROADMAP 指针同步）；
  ADR-0003 **D9 标记为已落地**并新增一行 §14 修订；本文件覆盖写

### 更早几轮（plan 0504 及之前）

- [x] **0504**（SSH 接入 IPC / 前端）：一条带目标的会话命令 + 一条"后端询问 → 前端作答"的往返
  （ADR-0003 **D16**，120 s 超时即拒绝）+ 库连接改为可共享句柄（⚠️ **不得在持锁期间连接**）。
  细节见 [`archive/0504`](./plans/archive/0504-ssh-into-ipc-frontend.md) 与上面的基线表
- [x] **0503**：known_hosts 三态判定（库 → 用户文件只读 → 提问）+ **本仓库首次库格式迁移**
  （v1 → v2；`UpgradeFailed` / 缺表的 v1 不迁移 / 导出不迁移来源）
- [x] **0502**：`akasha-ssh` 连接 + 认证（D7 的顺序、D8 的内存凭据缓存、`TransportError::Busy`）、
  `VmLck` 对新用途的重新验证；同时修正 ADR 自身的两处错误（`-R` 的验收归属改到 plan 0604；新增 D15）
- [x] **0501**：调研先行、结论带出处（`russh-0.63.3` 逐条核对 + 两条实测硬事实）

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接运行 `cargo …` 会失败（问题 #8），一律通过 `just` 转发。⚠️ **临时脚本同样适用**。
- **五个 crate 的职责**：`akasha-core`（Session 模型 + 配置模型与判据，**零 Tauri 依赖**）、
  `akasha-pty`（`Transport` + portable-pty + 合批 + `teardown` + `watchdog`）、
  `akasha-store`（库的打开 / 创建 / **格式版本与迁移** / 四类池（含 `jump_chain`）/
  **known_hosts 缓存** / dump / 导出与还原 —— 唯一允许 `unsafe` 的 crate）、
  `akasha-ssh`（连接 + 认证 + known_hosts 三态 + **D9 原语与跳板** + 同步 `Transport` 门面 +
  `testing`（进程内服务端））、
  `akasha`（app 包 = IPC 薄壳 + 托盘 + 配置 + 数据目录 + 窗口关闭语义 + 单实例 + 退出钩子 +
  看门狗接线 + 库的解锁状态 + SSH 的 runtime / 提问往返 / 池读取 / **跳板链** + 代码生成 bin）。
- **前端**：`src/ipc/`（唯一允许调用后端的目录）、`src/tabs/`、`src/terminal/`、`src/ssh/`、`src/App.tsx`。
- **排查"SSH 为何连不上"**：日志中 `ssh session opening`（带 `hops` = 跳数）/
  `ssh authenticated`（带 `method`）/ **`ssh direct-tcpip opening`（带 `via` = 经过的主机）**；
  主机密钥一档见 `ssh host key accepted` / `rejected` / `unusable`；提问是否有人应答见
  `prompt has no publisher`（未装载发布口时会立即超时）。
- **排查"某个会话是否仍在"**：`app_state { probe: "sessions" }`（`live` 与 `registered`
  **必须相等**，分叉说明存在"可查询、但无人管理"的会话）。
- **修改库格式之后先看**：`akasha-store/src/schema.rs`（`DDL_V1` 冻结 + `TABLES`）→
  `lib.rs` 的 `upgrade()` / `FORMAT_VERSION` → `tests/format_migration.rs`。⚠️ 新增表**必须**提升
  `FORMAT_VERSION` 并编写迁移。
- **SSH 层的形状**：`docs/adr/0003-ssh-stack-and-resource-model.md`（状态「实现中」，
  §14 含修订记录）。动手前先读 §2 的「事实依据」（版本 / API 均带出处）、D1–D16 与 §12 的未决清单。
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
