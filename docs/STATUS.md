# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-13

## 一句话

**阶段 2「端到端最小终端」5/5 完成**；**阶段 3「托盘与应用生命周期」6/6 完成**；
**阶段 4「存储与凭据池」9/9 完成**；**阶段 5「SSH 栈」5/6**（0501 ADR / 0502 连接认证 /
0503 known_hosts / 0504 接进 IPC 与前端 / **0505 `direct-tcpip` 原语 + 跳板**）；
下一步是 **0506 `~/.ssh/config` 受限子集导入**。

阶段 4 的八块（一句话各一块）：**SQLCipher 加密库能开**（0401）、**口令只从一条路进来，而且能真的
验证它**（0402 —— 拆开了"打开"与"新建"，原来在没有文件的路径上**任何口令都能开**）、
**口令在内存里也受保护**（0406）、**四套池能增删改查**（0403）、**库里的东西拿得出去也放得回来**
（0404）、**解锁与锁定是一条完整的生命周期**（0407 —— 真 app 上 `VmLck` **0 → 192 → 0 kB**）、
**搬走文件夹之后数据还在且能用**（0405）、**便携目录不可写时拒绝启动**（退出码 2）。

**阶段 5 的形状先定死在 [ADR-0003](./adr/0003-ssh-stack-and-resource-model.md) 里**（plan 0501）：
`russh` 的版本与 feature、运行时归谁、`Transport` 在 SSH 上的映射、`direct-tcpip` 原语的形状、
隧道状态机与重连判据全部**带出处**写死。之后四块依次落地：**连接 + 认证**（0502）、
**主机密钥的信任策略 + 本仓库第一次库格式迁移**（0503）、**接进 IPC 与前端**（0504）、
**`direct-tcpip` 原语 + 跳板（ProxyJump）**（0505）。

### 阶段 5 现在到哪了（按依赖排的顺序）

| plan | 做完了什么 | 判据在哪验的 |
|---|---|---|
| 0501 | ADR-0003（D1–D16，带出处的调研 + 资源模型） | `docs/adr/0003`，状态「实现中」 |
| 0502 | `akasha-ssh` 连接 + 认证（D7 的顺序、D8 的内存凭据缓存） | **crate 层**：进程内 SSH 服务端，三个连接只问一次凭据 |
| 0503 | known_hosts 三态判定（D11）+ 库格式 v2 与第一次迁移 | **crate 层**：策略 + 库契约；迁移用真 v1 库 |
| 0504 | SSH 接进 IPC / 前端：带目标的会话命令、提问往返、提示界面 | **真 app**（`ssh_session` E2E + 测试进程内的服务端） |
| **0505** | **`direct-tcpip` 原语**（D9 的形状 = 一条流）+ **跳板链**（池里的 `jump_id` 真的走） | **crate 层**（两个真服务端 + 负控）+ **真 app**（`ssh_jump` E2E） |
| 0506 | `~/.ssh/config` 受限子集导入 | 未开始 |

**这一轮（0505）把"一个原语服务三处"的地基落了**：`SshConnection::direct_tcpip` 交出**一条
`AsyncRead + AsyncWrite` 的流**（`SshStream`），跳板那条路把这条流交给 `connect_stream`
当下一跳的"网络"——于是"经跳板连一台**只有跳板看得见**的主机"这条判据在真 app 上成立。
整条链（每一跳各问各的凭据、各校各的主机密钥）**随最终那条连接的 task 一起生灭**：
关标签页 = 整条链一起断。

### 阶段 5 之前那些跨阶段的结论（还在生效）

**点叉的语义由配置 × 托盘共同决定**（0302 + 0303）：

| `close_behavior` | 托盘建成了吗 | 点叉之后 |
|---|---|---|
| `tray`（默认） | 是 | **窗口隐藏、进程留着** —— 会话与终端缓冲原样存活 |
| `tray` | **否** | 退出（**降级**：藏起来就再也叫不回来，坑 #60） |
| `exit` | 任意 | 退出（走 0204 的收尾路径，零残留） |

配置文件 = **数据目录**里的 `config.json`（`docs/portable.md` §3.1）：bin 同目录存在
`akasha-data/` 就用它（便携模式），否则退回 OS 数据目录（Linux 上 =
`~/.local/share/fans.cyrene.akasha-terminal/`）。**只读、不自动创建**；读不到 / 值不认识 →
默认值 + 一条日志。⚠️ **只在启动时读** —— 改完要重启 app。

**单实例（0304）**：第二个实例会把已有窗口**叫回来**（还原 → 显示 → 置前）然后自己退出。
窗口**藏起来时也一样**（只 `set_focus()` 叫不回隐藏窗口）。

"何时回收"因此有七种触发（粒度从一个会话到整个进程）：

| 触发 | 谁被回收 |
|---|---|
| 关一个终端标签页 / 会话自己结束 | **只有那一个会话**（SSH 那条路 = **整条跳板链一起断**） |
| **关窗口（默认 = 收托盘）** | **不回收** —— 进程、会话、终端缓冲全留着（0302） |
| 关窗口（`close_behavior = exit`） | 全部（`RunEvent::Exit` → `Sessions::shutdown_all()`） |
| **从托盘菜单退出** | 全部（`shutdown_all` → `app.exit` → `RunEvent::Exit` 再收一次，幂等） |
| panic | 全部（panic hook：打崩溃现场 → 回收 → `abort()`） |
| `tauri dev` 重载 / `kill -9` / `kill -TERM` | 全部 —— **另一个进程**：看门狗读到管道 EOF（ADR-0005） |

⚠️ **关标签页 ≠ 关窗口 ≠ 退出应用**：关掉**最后一个**标签页只是**空状态**（界面空了、进程留着）；
关窗口（默认）只是**隐藏**。只有**三大终端**（local / ssh / serial）的标签页有关闭按钮；
转发 / 密码库 / 文件传输是**仅渲染**的视图标签页（**无关闭按钮**）—— 见 `docs/scope.md` §5.6。

**可搬迁性（0405）**：一条数据目录规则 + 一条配方。判"可写"的方式是**真的写一个探针文件再删掉**
（`.akasha-writable`）—— mode 位看不出 ACL / 只读挂载 / squashfs。配方 `just portable` 自动跑完
`portable.md` §5 的五步（复制 bin → A 起 → 完全退出 → 搬成 B → B 起 → 断言四套池 **1/1/1/1**
+ 库侧逐项比对内容）。

## ⚠️ UI 现状：**当前界面是功能验证壳层，不是设计稿**

**正式 UI 的布局 / 视觉 / 交互尚未有设计稿。** `src/**` 现有的界面（标签栏、状态栏、主机选择器、
提示面板、配色、空状态文案）只有一个用途：让后端行为能被看见、能被验证。规则写在 `AGENTS.md` §4.0，
展开在 `docs/scope.md` §1.3。三句话：

- **不要**把当前界面当产品约束或"既有风格"，不要在它上面做视觉打磨；
  前端改动的判据是"**这条后端行为能不能被验证**"，不是"好不好看"。
- 后端**不得**依赖前端的呈现方式：界面整体重做时，命令 / 事件 / `Session` 状态机与收尾路径
  应当**原样可用**。
- 验证用的探针与选择器（`window.__akashaTerminal`、`.tab-pane.is-active …`、
  `.ssh-prompt[data-prompt-kind]`、`.host-picker-jump`）是**测试接口**，不是 UI 规范。
  **重做界面属于尚未规划的工作**。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + gen-types-check + docs-check） | 退出码 **0**，**6/6 全绿** |
| `just test` | **243 tests run: 243 passed**（`akasha` **66** + `akasha-core` 15 + `akasha-pty` 39 + `akasha-ssh` **31** + `akasha-store` **92**）。⚠️ `akasha` 那 66 条含 `tests/` 下的集成目标（没有 `VICTAURI_E2E` 时它们只打印原因并返回 —— 其中 `portable` 3 条、`ssh_session` 1 条、**`ssh_jump` 1 条**） |
| ↑ **判据：ProxyJump 可连通只对跳板机可见的目标**（plan 0505） | ✅ `ssh_jump` E2E（真 app + **测试进程内两台**服务端）：池里那一行的 `host` 是 `akasha-e2e-inner.invalid`（用例自己解析一次并**断言失败**）→ 界面选它 → **四条提示按序答完**（跳板的密钥 / 跳板的口令 / 目标的密钥 / 目标的口令）→ 连上 → **跳板服务端记到恰好 1 条 `direct-tcpip → akasha-e2e-inner.invalid:22`** → 敲的字到了**目标**服务端 |
| ↑ **那个名字在本机解析不出来**（构造前提） | ✅ `(INNER_NAME, 22).to_socket_addrs()` **返回 Err** —— 用例自己断言。于是"字节到了目标"这件事**只可能**经过跳板（无特权环境做不出真网络隔离，这条是替代口径，见「待验证」） |
| ↑ **每一跳各问各的凭据**（D8 的缓存键含 host） | ✅ 跳板服务端收到的是 `jump-host-password`、目标收到的是 `inner-host-password`（**两句话不一样**，给错就认证失败）；两台的**指纹也不同**（各自被问过一次） |
| ↑ **跳板上的中继真的通了**（服务端那一侧的证据） | ✅ 跳板的 `relayed_bytes` **> 0**（实测 5240 字节，会话还开着时就读得到）；关标签页后中继收工、目标服务端看到连接断开 |
| ↑ **库内的原语验收**（plan 0505，crate 级 4 条） | ✅ `jump_host` **4 passed / 0.25 s**：正例（跳板恰好 1 条 `direct-tcpip`，host/port 与配置一致）+ **负控**（不经跳板直连那个名字**必须失败**）+ 跳板拒绝转发时是 `SshError::Forward` 而不是 `Connect` + 同步门面在 tokio 上下文里被拒 |
| ↑ **跳板链住**（plan 0505，store 级 3 条） | ✅ `pools_roundtrip` **11 passed**：链**目标在前**读得出来；**手工用 SQL 塞进去的环**在读路径上被拦住（写入路径挡不住有人改库，而环会让连接**挂住**，不是报错）；深过 `MAX_JUMP_DEPTH` 的链报错 |
| ↑ **判据：真 app 上开一个 SSH 会话**（plan 0504） | ✅ `ssh_session` E2E（真 app + **测试进程内**的服务端，**2.32 s**）：界面点 SSH → 选中池里那一行 → **主机密钥提示里那串指纹等于服务端的** → 接受 → 口令提示 → 填答 → 连上（标签页标题 = 池里的名字） |
| ↑ **字节能双向流 / 确认过的密钥进我们的库 / 凭据只问一次 / 关标签页零残留**（plan 0504） | ✅ 四条各有一断言：回声在屏幕上**且服务端收到同一串**；直连库文件读到 `known_hosts` **1 行**；第二个会话**一次都没问**就连上（服务端第 2 次收到**同一句**口令）；关两个标签页 → `sessions` probe 回 **`{"live":1,"registered":1}`** + 服务端看到 **2 条**连接断开 |
| ↑ **提问往返自身**（plan 0504，crate 级 6 条） | ✅ 答案到得了问的人手里 / 超时会**撤回**那一问、之后作答报 `Gone` / **取消与超时分得开** / **没人答 = `HostKeyUnknown`（拒绝），不是 `Ok`** / 用户接受之后**真的写进缓存** |
| ↑ **`just test-e2e` 全绿** | 退出码 **0**：**20 个 E2E 用例**（含新增的 `ssh_jump`）+ 第三段 3 条（`portable`）。`ssh_session` / `ssh_jump` 都排在 `vault_unlock` **之后**（三个用例都在动同一个库文件） |
| ↑ **判据：host key 变了就拒绝，没见过的要问一次**（plan 0503） | ✅ `akasha-ssh` 的 7 条：未知且没人可问 → `HostKeyUnknown`（带去核对的指纹，且**认证一步没开始**）；确认 → 进缓存，**第二个连接 0 次提问**；记录对不上 → `HostKeyChanged`（**两个指纹都在**）且**一次都不问**；用户否认 → 拒绝且**不记录**；用户文件里认得 → 连上且文件**逐字节没变** |
| ↑ **本仓库第一次格式迁移**（plan 0503） | ✅ `akasha-store` 的 6 条：`DDL_V1` 造出**真 v1 库** → `open` 之后 `user_version = 2`、五张表在、**那条 host 还在**；再开一次当前格式的库**一个字节都不写**；缺表的 v1 **不迁移**；导出与明文导出两条还原路都**升的是副本、来源逐字节不变** |
| ↑ **判据：同主机三个连接只问一次凭据**（plan 0502） | ✅ `three_sessions_ask_for_one_credential`：`provider.calls() == 1`、缓存 `len() == 1`、服务端三次都收到**同一句口令** |
| ↑ **认证顺序是线协议上的事实**（plan 0502） | ✅ 服务端记下的序列：`publickey → password`、`publickey → keyboard-interactive`；agent 不可用时序列里**没有** `publickey` 痕迹 |
| ↑ **`nodelay` 从空话变成真的**（plan 0505 修） | ✅ `tcp_stream` 里自建 TCP 时显式 `set_nodelay(true)`（坑 #120：上游只在 `client::connect` 里看 `Config::nodelay`，而两条路都用 `connect_stream`） |
| ↑ **`cargo.lock` 的增量只有一行**（plan 0504/0505） | ✅ 加 `akasha → akasha-ssh` 这条边**只多一行**；0505 **一行都没多**（没有新依赖，`rand` 早就是 `akasha-ssh` 的真依赖） |
| `pnpm build`（tsc + vite build） | 退出码 0；产物 **850.09 kB / gzip 233.41 kB**（+0.2 kB：选择器上多一个"经跳板"标记） |
| `just docs-check` | 全过（ROADMAP 58 个条目 ≤3 行且无代码块 / 50 份 plan ≤200 行且索引一致） |
| `ast-grep scan` + `ast-grep test` | 都退出 **0**（本轮没有新增 / 改动规则） |
| **三条 unsafe 注释 lint**（clippy，`just lint` 里） | 退出码 **0**；三条各用一个探针证明**它们真的会红**（探针跑完即撤） |
| ↑ **解锁 / 锁定 / 内存还回去**（0407） | ✅ 真 app 上 `VmLck` **0 → 176～192 → 0 kB**；进程内存扫描（带正对照）：口令 `1 → 2 → 2 → 1` 处、派生密钥 `3 → 1` 处 |
| ↑ **导出与还原**（0404） · **搬走文件夹**（0405） · **退出零残留**（0204/0205） | ✅ 三项判据照旧全绿（`just portable` **3 passed**；`app 已退出` + `零残留`） |
| ↑ **终端 / 会话判据（未退化）** | `renderer = webgl`；8 MB 灌流后仍可交互；raw 通道 10.73 MB / 172 批；收尾帧 1 个、console 零异常 |
| ↑ **§7 的 registry 那一条：当前不可满足**（坑 #82） | `get_registry` 回 **`[]`**（命令都没标 `#[inspectable]`）。**替代证据**是真路径上的 `invoke_command` 成功 —— 本轮的新命令都是这么验的 |
| `cargo tree -p akasha-core \| grep -c tauri` | **0**（分层成立） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数字，本轮未复跑**） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。

## 待验证（本地跑不了 / 沙箱跑不了）

- **"只对跳板机可见"是**构造**出来的，不是真的网络隔离**：无特权环境里换 netns / 加防火墙规则
  都要 root，所以那条性质靠**名字**（`.invalid` + 跳板侧的中继表）实现 —— 用例自己解析一次并断言
  失败。它与"跳板机能看见、我看不见"在行为上等价，但**不是**同一件事。
- **多跳链（`jump_id` 的跳板还有跳板）没有端到端跑过**：crate 级用例是**一跳**，E2E 也是**一跳**；
  链的顺序（目标在前 → 连接时反转）只有 store 的用例与代码在读。真机上配两跳跳板**没试过**。
- **SSH 的测试服务端仍是我们自己搭的**（住在 `akasha-ssh::testing`）：它证明的是**客户端这条链**
  与 app 的接线，**不是**与 OpenSSH 的互操作 —— 没连过真的 `sshd`，也没连过任何真实服务器。
  跳板这条路上还多一层：真实跳板机对 `direct-tcpip` 的策略（`AllowTcpForwarding`、
  `PermitOpen`、`originator` 相关的审计）**一条都没实测**。
- **解锁 / 锁定仍然没有界面**：命令、状态、生命周期都在（plan 0407），而界面上**没有能输口令的地方**
  —— 所以 SSH 那条真路径上，**解锁这一步是 E2E 用 `invoke_command` 完成的**（界面只到"选主机"为止）。
  ⚠️ 别把"能开 SSH 会话了"读成"用户从冷启动就能自己走通"：他要先有个能输库口令的地方。
- **主机池的增删改查仍然没有界面**：plan 0504 只加了**只读**的 `vault_hosts`，plan 0505 让它多带了
  `jumpId`。**配置一条跳板链今天只能直接往库里写**（E2E 就是这么种数据的）—— 界面上看得见
  "经跳板 X"，但改不了它。
- **真实 agent 那条路只有"不可用"被覆盖了**：agent 里真有钥匙、且服务端认它这条从未跑过。
- **并发提问没有实测**：两条连接同时提问时，两条提示会**并排**在面板上（前端按 id 列表渲染），
  但只跑过"一条连接一次一问"。跳板链上的提示是**串行**的（四问按序），链一长就是这个量级 × 跳数。
- **保活那三个数是默认值、不是实测值**（ADR D15）：没造出"半死连接"与高延迟链路。
- **私钥与口令各有一份够不着的明文副本**（D8 照实记）：`russh` 要 `ssh-key::PrivateKey` 才能签名、
  要 `String` 才能送口令，两者都在**普通堆**上，我们擦不掉。
- **私钥候选的稳定标识只有行 id**：池里的 `update` 会保留 id，于是"换了材料没换 id"会在**同一个
  解锁窗口内**留下一条过期口令（自愈是现成的：解不开就 `forget` 再问一次；缓存本身内存-only）。
- **提问占住一个 runtime worker**：这是 D16 照实记的代价，靠 4 个 worker + 120 s 超时兜着。
- **只读介质上的 v1 库没有实测**：迁移要写文件，那条路会以 `UpgradeFailed` 失败 —— 代码有这一支，
  但没有造出真只读文件系统来验它。**降级也没有实测**（v2 的库用旧二进制打开）。
- **与 OpenSSH 的 `known_hosts` 互操作没有实测**：`@cert-authority` / `@revoked` 这类标记行的行为
  没有验过；用户文件里读不动的行的处置（`warn` + 当作未知）没有用例守着。
- **解锁 / 导出 / 口令经 IPC 的边界**（照旧）：tauri 自己的两份口令副本够不着（ADR-0002 §7.5）；
  `mlock` 失败那条路只有单测；内存扫描只在 Linux、只扫匿名段。
- **权限位、单实例、托盘在非 Linux 上未验**：CI 的类型检查挡不住运行期差异；CI 三个 job 至今
  没跑过（仓库没有 remote）。
- **前端类型检查不在任何门禁里**：`just ready` 只覆盖 Rust + 文档，`pnpm build`（tsc）要手动跑。
- **`just dev-web` 的模拟后端没在真浏览器里点过**：SSH 两条命令在里面**明确报错**
  （"没有 SSH 客户端"），所以主机选择器在浏览器里只会显示那句话。

## 当前基线（2026-09-13 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` / `akasha-store` / `akasha-ssh` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 后端模块 | `bindings` / `session` / `tray` / `config` / `lifecycle` / `single_instance` / `vault` / `watchdog` / `ssh`（长住状态 + 那条命令 + 跳板链 + 库内 known_hosts 适配器） / `prompt`（提问往返） / `pools`（池的只读读取） |
| **命令清单** | `greet` · `vault_status` / `vault_unlock` / `vault_lock` · `vault_hosts` · `open_session` · `open_ssh_session` · `write_session` / `resize_session` / `close_session` · `ssh_prompt_credential` / `ssh_prompt_host_key` / `ssh_prompt_cancel`（事件：`session_ended` · `ssh_prompt` / `ssh_prompt_dismissed`）。**plan 0505 没有新增命令** —— 跳板是既有那条命令内部多走几跳 |
| **probe** | `lifecycle` → `{close_behavior, tray_ready, close_action}`（没登记时 `{"initialized":false}`，坑 #93）；`single_instance` → `{registered, activations}`；`sessions` → `{live, registered}`（SSH 没有本地进程，"零残留"只能看注册表）。**库没有 probe**：状态本身就是命令（`vault_status`） |
| 出字节路径 | PTY / SSH read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write`。**两条载体共用同一条尾巴**（`session::open_terminal`） |
| **`direct-tcpip` 原语**（plan 0505，ADR-0003 **D9**） | `akasha-ssh/src/forward.rs`：`SshStream`（自己实现 `AsyncRead + AsyncWrite`，**不把 `russh::ChannelStream` 漏进公开签名**）+ `SshConnection`（已认证、**没有通道**的连接，持有 `Handle`）+ `SshConnection::direct_tcpip(host, port)`。三处消费者（跳板 / `-L` / SFTP B 档）吃的都是**这条流**；`-L` 与 SFTP 尚未接上（plan 0602 / 0703） |
| **跳板链**（plan 0505） | 库那侧：`hosts::jump_chain`（**目标在前**、有界、成环报 `StoreError::JumpChain`）。app 那侧：`ssh.rs::plan_chain` 读出链 → 反转成"最外层在前" → `SshTransport::connect_via(runtime, hops, target)`（`connect` = 空链那次）。**每一跳各一份 `SshConnect`**（各问各的凭据、各校各的主机密钥），链上每一跳是一个 `SshConnection`，随 `Established::carriers` **move 进最终那条连接的 `pump` task** —— "task 结束 = 整条链结束"，收尾按**最内层先断** |
| **连接的 originator** | `direct-tcpip` 要求带发起方地址（RFC 4254 §7.2）：用**最外层那条 TCP 的本地地址**（我们唯一真知道的），往下每一跳复用；拿不到就空串 + 0（不许编一个看着像真的地址进对端日志） |
| **SSH 的 IPC 层**（plan 0504） | `src-tauri/src/ssh.rs`：app 启动时建**一个**专用 tokio runtime（**4 个 worker**，D2）；`open_ssh_session` 是 **async 命令**（不挡 IPC），内部起一条**普通 `std::thread`** 跑同步门面（`spawn_blocking` 的线程**也算** tokio 上下文，会吃到 `BlockingInsideRuntime`），结果经 `tokio::sync::oneshot` 回来。`SshConnect` 的材料按池行组：`password` → 不用 agent、不带钥匙；`agent` → 只用 agent；`publickey` + `key_id` → 那一把钥匙（PEM → 受保护页 → `KeyCandidate`，标识 `key#<id>`） |
| **提问往返**（plan 0504，ADR-0003 **D16**） | `src-tauri/src/prompt.rs`：`Prompts`（`Arc` + 待答表 + 可注入的发布口）；事件 `ssh_prompt`（判别式：`hostKey` / `credential`）+ `ssh_prompt_dismissed`；三条回答命令；编号从 1 起、只增不减；**超时 120 s → 拒绝 + 撤回**；答过 / 超时的 id → `PromptError::Gone`；**主机密钥那一问只认"接受 / 拒绝"**，超时 / 取消 / 答错类型一律 `Err(HostKeyUnknown)`（= 拒绝连接）。⚠️ 跳板链上**每一跳各来一轮**（密钥 + 口令），E2E 实测四问按序 |
| **库内主机密钥缓存**（plan 0504 接线） | `VaultHostKeys`：`Vault` 是 `Arc` 可克隆的，`with_conn` **短借**连接；库锁着 → `SshError::HostKeyCache` → **拒绝连接**（不当作未知）。⚠️ **绝不在持锁期间连接**：`remember` 会在连接中途回头锁库（跳板链因此是**一次读完整条**再开始连） |
| **库的解锁状态** | `Vault { inner: Arc<Mutex<Option<Unlocked>>> }`（`Clone`）；`Unlocked { conn, passphrase }` 同生共死。借库的失败分两种（`ConnError`：`Locked` / `Store`）—— 因为 SSH 那条路要单独认出 `NoSuchRow`（"你挑错了主机"） |
| **`akasha-ssh` 的形状** | 八个模块：`target` / `credential` / `keys` / `handshake`（`handshake<S>` = 一跳的握手 + 认证，**底层流由调用方给**） / `known_hosts` / **`forward`（D9 原语 + `SshConnection`）** / `transport` / `testing`（进程内测试服务端，**生产代码别用**；它支持 `direct-tcpip` 的中继与拒绝两条分支） |
| **错误分域** | `akasha-ssh`：`HostKeyCache`（库那一侧读不动缓存）、**`Forward { host, port, reason }`**（跳板拒绝 / 够不着目标 —— 与"我连不上那台机器"分开）。app 侧 `SshIpcError`：`Locked` / `NoSuchHost` / `Failed { kind, message }`（`kind` = `hostKeyChanged` / `hostKeyRejected` / `hostKeyUnknown` / `hostKeyCache` / `auth` / `connect` / **`jump`** / `other`）/ `Internal` —— **前端按 `kind` 分辨**，不匹配消息字符串 |
| **前端结构** | `src/ipc/`（`session.ts` / `prompts.ts` / `hosts.ts` —— 唯一允许碰后端的目录）、`src/tabs/`、`src/terminal/`、`src/ssh/`（主机选择器 + 提示面板）、`src/App.tsx`。标签页 `kind`：`terminal` / `ssh`（**都有关闭按钮**，规则写成 `CLOSABLE` 清单） |
| **前端的一个 dev-only 陷阱** | React StrictMode 把 effect 走两遍 → SSH 会话会被开两次。处置：SSH 那条连接**推迟一个微任务**再发（坑 #118） |
| **SSH 栈**（ADR-0003） | `russh = "=0.63.3"`、features `["ring","rsa"]`；`akasha-ssh` 只收 `tokio::runtime::Handle`；对外是同步 `Transport` 门面 + 两条**有界** mpsc（满 → `TransportError::Busy`）；capability = `resize + exit_status`、`session_leader() = None` |
| **连接取值**（D15 + 0505 的修正） | `connect_timeout = 10s`；`keepalive_interval = Some(30s)`、`keepalive_max = 3`；**Nagle 关掉**（`tcp_stream` 里显式 `set_nodelay(true)`，坑 #120）。⚠️ 那三个数是**有理由的默认值**，不是实测出来的 |
| **库格式与迁移** | `FORMAT_VERSION = 2`；v1 = 四张池表（`DDL_V1` 冻结、公开），v2 = v1 + `known_hosts`。`open` 里 `upgrade()`：`== 2` 什么都不做；`1` → 先按 v1 校验形状 → **一次事务**里加表 + 写版本号；`> 2` 与 `0` 拒绝；写不动 → `UpgradeFailed`。⚠️ **降级不行** |
| **库文件的磁盘事实** | `akasha.db`；建库后 **36864 字节 = 9 页**；SQLCipher 4.5.7 + vendored OpenSSL 3.6.3 + 内嵌 SQLite **3.46**；`user_version = 2` 是格式权威；盐 16 字节随机；显式收紧到 **600**；不带 `-wal` / `-shm`；解锁代价 **~105 ms**（KDF） |
| **四套池** | `keys` / `hosts` / `serials` / `forwards`，各 5 个函数 + 反查（`hosts` 另有 `jump_chain`）。`New*`（没有 id）与 `*`（有 id）**是两种类型**；不变量写在库上（`STRICT` + `CHECK` + 外键 `RESTRICT`，D14） |
| **known_hosts 缓存** | 表 `known_hosts(id, host, port, key_type, key_blob, fingerprint)`，`UNIQUE (host, port, key_type)`。**缓存不是池**；判定材料是 `key_blob`（逐字节比）；`remember` 遇到同键不同值 → `Conflict`（**写路径上就不许静默改写**） |
| **主机密钥的三态判定** | `KnownHostsVerifier`：**库 → 用户的 `~/.ssh/known_hosts`（只读）→ 提问**。库里 / 文件里对不上 → `HostKeyChanged`（**不看不问**）；两边都没有 → 有 `HostKeyPrompt` 就问、确认后 `remember`。两个注入点是**同步** trait |
| **口令与私钥** | `Passphrase` / `PrivateKey`：空值**造不出来**、**没有 `Debug`**、本体住在 `memsafe` 的受保护页；`PassphraseInput` 是口令**经 IPC 进来的唯一形态**（vault 解锁与 SSH 凭据**共用**它） |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| CSP | `default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| capabilities | 仍只有 `core:default` + `opener:default`（+测试用的 `victauri`）。**SSH、托盘、配置、单实例、便携目录检查都没有加任何 permission**（全在 Rust 侧） |
| 前提条件 | **需要能写 `$HOME`**；托盘另需能写 `$XDG_RUNTIME_DIR`、单实例另需会话总线（否则各自只降级）。**便携目录存在时另需可写 —— 不可写是拒绝启动**（退出码 2） |

## 进行中 / 下一步

- [ ] **下一步 = 阶段 5 的 plan 0506**（[`~/.ssh/config` 受限子集导入](./plans/0506-ssh-config-subset-import.md)）：
  它是**未规划（骨架）**，要先补齐「步骤」与「验收命令」。**0505 已经让池里的 `jump_id` 真能用**，
  所以导进来的 `ProxyJump` 这才不是一句空话。⚠️ 它要动的是**写入路径**（池的第一个写命令），
  而写入路径的判据（重名 / 成环 / 引用）在 `akasha-store` 那侧已经写好了。
- [ ] **阶段 5 之后还有两处界面缺口**（不是 bug，是没有规划的工作）：**解锁界面**（今天 SSH 那条
  真路径上，解锁由 E2E 的 `invoke_command` 完成）与**主机池的增删改查界面**（今天只能直接写库 ——
  连跳板链也一样）。
- [ ] **`-L` / SFTP B 档还没接上 `direct_tcpip`**：形状已定（一条流），真正的适配在 0602 / 0703。
- [ ] **降级路径没有实测**：v2 的库在旧版本程序里会以 `UnsupportedVersion { found: 2 }` 被拒（有意）。
- [~] **plan 0102（CI 平台矩阵）**：本地部分完成，最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测，剩 CI 三平台格子
- [ ] **正式 UI**：等设计稿（见上面「UI 现状」）—— 没有验收标准，故**不进 ROADMAP**

> 阶段 5 的编号按依赖重排过（2026-09-13）：0503 known_hosts · 0504 接进 IPC / 前端 ·
> 0505 `direct-tcpip` 原语（原 0503）· 0506 `~/.ssh/config` 导入（原 0504）。
> 理由：信任策略是**接口形状**（先行），而原语的消费者都要先有一条**从 app 打得开的**
> SSH 会话才验得了。编号与执行顺序现在一致，索引里有一段重排说明。

### 本轮完成（plan 0505：`direct-tcpip` 原语 + 跳板）

**判据（ROADMAP 原文）**：ProxyJump 可连通**只对跳板机可见**的目标。

- [x] **原语的形状按 D9 落地**：`SshStream`（一条 `AsyncRead + AsyncWrite` 的流）+ `SshConnection`
  （已认证、没有通道的连接）+ `direct_tcpip`；`establish` 拆成 `handshake<S>`（**底层流由调用方给**）
  与 shell 尾巴 —— 于是"通道当下一跳的网络"与"直连"走**同一条路**
- [x] **`Handle` 归谁定案**：跳板那条路**自己持有连接**（另一条路是给 `SshTransport` 开受控借用口）
  —— 句柄不出 `akasha-ssh`，也不需要回答"谁在什么时候能碰它"
- [x] **整条链一起生灭**：carriers move 进最终那条连接的 `pump` task，收尾**最内层先断**
- [x] **错误分域**：新增 `SshError::Forward`（跳板拒绝 / 够不着目标）→ `SshFailureKind::Jump`
  —— "我连不上跳板机"与"跳板机连不上那台"要分得开，否则排查方向会指错
- [x] **库那一侧**：`hosts::jump_chain`（目标在前、有界、成环报错）；**读路径也挡**环与深度
  （写入路径挡不住有人手工改库，而环会让连接**挂住**）
- [x] **判据实测**（真 app + 测试进程内两台服务端）：见上表四行 —— 名字不可解析 / 四条提示按序 /
  跳板记到 1 条 `direct-tcpip` / 字到了目标 / 关标签页链断干净
- [x] **顺手修掉一个真空话**：`Config::nodelay` 在 `connect_stream` 这条路上从来没生效过
  （坑 #120），现在自建 TCP 时显式关 Nagle
- [x] **门禁**：`just ready` **6/6**；`just test` **243 passed**（+8）；`just test-e2e` **退出码 0**；
  `pnpm build` 退出码 0；`cargo.lock` **零增量**
- [x] **记账**：plan 0505 改「已完成」并 `git mv` 进 `archive/`（索引与 ROADMAP 指针同步）；
  ADR-0003 **D9 标上已落地**并新增一行 §14 修订；本文件覆盖写

### 上一轮完成（plan 0504：SSH 接进 IPC / 前端）

- [x] **一条带目标的会话命令**：`open_ssh_session` 按主机池的一行连过去；SSH 与本地终端**共用**
  注册 / 频道 / 收尾那条尾巴（D3 的兑现）
- [x] **一条"后端问 → 前端答"的往返**（ADR-0003 **D16**）：一个事件 + 三条回答命令，
  **120 s 超时即拒绝**，被撤下时再发一个 `ssh_prompt_dismissed`
- [x] **库连接做成可共享句柄**：`Vault` → `Arc` + `with_conn` 短借；`HostKeyCache` 适配器接到
  `akasha_store::known_hosts`；⚠️ **绝不在持锁期间连接**
- [x] **服务端提成 `akasha_ssh::testing`**（app 的 E2E 也要在它自己的进程里起它）、新增 `sessions`
  probe、`session.rs` 抽出 `open_terminal`

### 上一轮完成（plan 0503 / 0502 / 0501）

- [x] **0503**：known_hosts 三态判定（库 → 用户文件只读 → 提问）+ **本仓库第一次库格式迁移**
  （v1 → v2，`UpgradeFailed` / 缺表的 v1 不迁移 / 导出不迁移来源）
- [x] **0502**：`akasha-ssh` 连接 + 认证（D7 的顺序、D8 的内存凭据缓存、`TransportError::Busy`）、
  `VmLck` 对新用途的重验；**顺带改了两处 ADR 自己的错**（`-R` 的验收归属改到 0604；新增 D15）
- [x] **0501**：调研先行、结论带出处（`russh-0.63.3` 逐条核对 + 两条量出来的硬事实）

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接跑 `cargo …` 会失败（坑 #8），一律用 `just` 转发。⚠️ **临时脚本里也一样**。
- **五个 crate 的分工**：`akasha-core`（Session 模型 + 配置模型与判据，**零 Tauri 依赖**）、
  `akasha-pty`（`Transport` + portable-pty + 合批 + `teardown` + `watchdog`）、
  `akasha-store`（库的打开 / 创建 / **格式版本与迁移** / 四套池（含 `jump_chain`）/
  **known_hosts 缓存** / dump / 导出与还原 —— 唯一允许 `unsafe` 的地方）、
  `akasha-ssh`（连接 + 认证 + known_hosts 三态 + **D9 原语与跳板** + 同步 `Transport` 门面 +
  `testing`（进程内服务端））、
  `akasha`（app 包 = IPC 薄壳 + 托盘 + 配置 + 数据目录 + 关窗语义 + 单实例 + 退出钩子 +
  看门狗接线 + 库的解锁状态 + SSH 的 runtime / 提问往返 / 池读取 / **跳板链** + 代码生成 bin）。
- **前端**：`src/ipc/`（唯一允许碰后端）、`src/tabs/`、`src/terminal/`、`src/ssh/`、`src/App.tsx`。
- **调试"SSH 为什么连不上"**：日志里 `ssh session opening`（带 `hops` = 跳了几跳）/
  `ssh authenticated`（带 `method`）/ **`ssh direct-tcpip opening`（带 `via` = 经过谁）**；
  主机密钥那一档在 `ssh host key accepted` / `rejected` / `unusable` 上；提问有没有人答看
  `prompt has no publisher`（忘了装发布口时会立刻超时）。
- **调试"某个会话还在不在"**：`app_state { probe: "sessions" }`（`live` 与 `registered`
  **必须相等**，分叉说明有会话"查得到、却没人管"）。
- **改了库格式之后先看哪里**：`akasha-store/src/schema.rs`（`DDL_V1` 冻结 + `TABLES`）→
  `lib.rs` 的 `upgrade()` / `FORMAT_VERSION` → `tests/format_migration.rs`。⚠️ 加表**必须**升
  `FORMAT_VERSION` 并写迁移。
- **SSH 这一层的形状在哪**：`docs/adr/0003-ssh-stack-and-resource-model.md`（状态「实现中」，
  §14 有修订记录）。动手前先读 §2 的「事实依据」（版本 / API 都带出处）、D1–D16 与 §12 的未决清单。
- **加依赖时**：`Cargo.toml` 写 `=` 钉版本（`russh` 与 `tauri-specta` 同一条口径）；
  本沙箱里 `cargo add` 会拒（坑 #105），而 `cargo deny` 还会去拉别的平台的依赖（坑 #106）。
- **改 E2E**：新增 `tests/*.rs` **必须**登记进 `src-tauri/justfile` 的
  `E2E_TARGETS` / `E2E_TARGETS_EXIT` / `E2E_NO_APP` / `E2E_SELF_APP` 之一（guard 会红）；
  清单**顺序就是执行顺序**，而且它们共用同一个 app。
  ✅ **`tests/support/mod.rs` 不用登记**：guard 扫的是 `tests/*.rs`（一层），而它是被各个目标
  `mod support;` 引进来的普通模块 —— 与 `akasha-store/tests/common/` 同一个先例。

## 踩过的坑（避免重复踩）

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency**。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**。
3. **just 的变量写 `$p`，不是 Make 的 `$$p`**。
4. **just 用 justfile 所在目录作为配方工作目录**。
5. **系统库缺失只在 cargo 构建脚本阶段暴露**；本机是 CachyOS（Arch 系），不是 apt。
6. **Tauri 没有 Rust 热重载，Victauri 也不提供**。
7. **just 的 shebang 配方需要可写的 runtime dir**，受限环境会失败。
8. **仓库根没有 `Cargo.toml`** → 根目录下一切 cargo 命令失败（**临时脚本也算**）。
9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** —— 跑一次 `just fmt`。
10. **CI 里 `libappindicator3-dev` 已不存在**，要用 `libayatana-appindicator3-dev`。
11. **受限环境下"写工作区之外被拒"看起来像工具/代码故障** —— 识别 → **直接提权重试**。
12. **`git checkout <file>` 会静默丢弃未提交的改动** —— 负例自检用 `cp` 备份/还原
    （⚠️ 备份别放 `/tmp`：沙箱每次调用一个私有 `/tmp`）。
13. **正向校验若不限定到目标段落就形同虚设** —— `docs-check` 用 `awk` 取 §2。
14. **`docs-check` 的反向检查**已扩到 `AGENTS.md` / `ROADMAP.md` / `docs/**/*.md`。
    ⚠️ **它不认识"下一步才存在"的命令**：plan 里出现的 `just <新配方>` 会让 docs-check 红 ——
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
24. **"一份工作流喂两个 forge"是一笔持续交的税**。
25. **兼容层的遗产会以"看起来更稳"的样子留下来**。
26. **阻塞的 `Read` 与"按时间交付"天生冲突** —— 正解是读线程 + `recv_timeout(期限)`。
27. **零匹配的测试过滤器在 nextest 里是"报错"**。
28. **`cargo bench` 会顺带用 bench 模式跑一遍单测目标**（正常）。
29. **仓库里出现第二个 bin 会让 `tauri dev` 起不来** —— 修法是 `default-run`。
30. **只写 `path` 的依赖等于版本号写 `*`** —— path 依赖要**同时写 `version`**。
31. **`Channel<Vec<u8>>` 不是二进制通道** —— 真正走 raw 的只有
    `Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`。**别只看字节数验收**。
32. **`u64` 不能直接过 IPC**：改用壳层 `u32` 句柄 + **checked** 转换。⚠️ 被生成器拒绝的是
    **一整类**（`usize` / `isize` / `i64` / `u64` / `i128` / `u128`）。
33. **沙箱里 E2E 必须与 app 在**同一次** bash 调用内**（每次调用都是独立的 bwrap）。
34. **`pkill -f <模式>` 会匹配到自己** —— 用 `pkill -f '[v]ite'` 或按 PID/进程组杀。
35. **`@xterm/addon-unicode11` 需要 `allowProposedApi: true`**。
36. **不在门禁里的测试等于没测**：`just test-e2e` 既不在 `ready` 里、又要真 app。
37. **`git mv` 之后 `docs-check` 会同时验两件事**（文件在不在、索引指得对不对）。
38. **每加一个依赖就多一份要维护的放行**。
39. **"测试自己抛的异常"会污染同一 app 上后跑的用例**。
40. **vite 默认只监听 `[::1]:1420`**；`/tmp/victauri/<pid>/` 里的 `pid` **就是 app 的 pid**。
41. **`(cmd) &` 在 fish 里是命令替换，不是子 shell**。
42. **`cargo test` 一次收多个 `--test` 时按目标名字母序跑**，不按参数顺序。
43. **`tauri-plugin-log` 默认的 `TargetKind::LogDir` 会让"日志目录不可写"变成 app 打不开**。
44. **`tracing/log-always` 会把依赖树的 TRACE 一起转成 `log` 记录**。
45. **SIGKILL 的投递是异步的**。
46. **portable-pty(unix) 的 `Child::kill()` 不是纯 SIGKILL**。
47. **早于日志插件注册的 `tracing` 事件会静默消失**。
48. **`/proc/<pid>` 存在 ≠ 进程还活着**（僵尸也有目录项）。
49. **按"命令行里含某段文本"找进程会误伤**。
50. **`term.dispose()`（xterm）会抛，而它跑在 React 的 effect 清理函数里**。
51. **多标签之后 DOM 选择器不再唯一**。
52. **断言超时不一定是"慢"**：真实原因可能是**界面已经被卸载**。
53. **`tauri-specta` 的事件必须 `mount_events`**。
54. **官方 `Channel` 不会告诉你"流结束了"**。
55. **会话里还有别的进程握着 PTY 时，主端读不到 EOF**。
56. **接线一次的回调必须走 `ref`**。
57. **日志消息里的"括号解释"会自己长大**。
58. **`tauri-plugin-log` 默认 formatter 的时间戳只到秒**。
59. **`signal` 字段的值是本地化的**（zh_CN 下 `SIGKILL` 写成 `已杀死`），尚未修。
60. **托盘在 Linux 上要写盘** —— 只读 runtime dir 里建不起来，所以它只能是**可选能力**。
61. **`libayatana-appindicator3` 与老的 `libappindicator3` 都是运行时 dlopen**。
62. **dbusmenu 的 item id 会随菜单重建而改变**。
63. **SNI 注册用的是唯一名**（`:1.x`）。
64. **Victauri 的 `window` 工具能机器验证窗口状态**。
65. **`AppHandle::exit()` 也会触发 `RunEvent::ExitRequested`**。
66. **便携数据目录就在 bin 同目录**，而 `just dev` 与 `just test-e2e` **共用同一个 bin**。
67. **Victauri 的 REST 兜底接口返回的是 `{"result": …}` 包了一层**（而 `just test-e2e` 里的
    `VictauriClient` 走 MCP，`call_tool` 直接返回工具内容本身）。
68. **`/proc/<pid>/exe` 可能带 ` (deleted)` 后缀**。
69. **文档里的"事实"没人核对就会自己长大**。
70. **cargo 的 workspace lint 继承是"全有或全无"**，而 `forbid` 不能被 `allow` 覆盖。
71. **带 `links = "..."` 的原生库在依赖树里只能有一个版本**。
72. **SQLCipher 的空 key 不是"静默关掉加密"，而是"返回错误且不挂 codec"**。
73. **SQLite 自己建出来的库文件是 644**（umask 022），不是 0600。
74. **`PRAGMA cipher_settings` 的输出是一列 `pragma` 行**。
75. **cargo-deny 的图根是"manifest 指向的那个包"，不是整个 workspace**。
76. **"0 字节的库"不是"空库"，是"还没有密钥"**。
77. **`cipher_memory_security` 是进程级、单向的**。
78. **模块内的 `#[cfg(test)] mod tests` 也要自己 `allow(clippy::unwrap_used)`**。
79. **`/proc/<pid>/mem` 的读用 `FOLL_FORCE`，绕过页保护**。
80. **`/proc/self/smaps` 的字段不是处处都有，而且属性行带缩进**。
81. **子串匹配会把规则变成笑话**：按 `_` 分词、整词比较，并配一对命中 / 诱饵负例。
82. **Victauri 的 `get_registry` 在本仓库是空的**（命令都没标 `#[inspectable]`）。
83. **clippy 会把"两个常量比较"的断言判红**（`assertions_on_constants`）—— 搬进 `const { … }`。
84. **E2E 配方两段可能跑在**不同**的数据目录里** —— 两段都先 `mkdir -p` 便携目录。
85. **集成测试的共用脚手架放 `tests/common/mod.rs`，但必须自己 `#![allow(dead_code)]`**。
86. **`pkill -f <pattern>` 会匹配到你自己那条命令行**。
87. **"什么都没发生"这类判据最容易写成永真式** —— 先用负例确认它会红。
88. **`sqlcipher_export` 写出来的文件默认是版本 0**（它不传递 `user_version`）。
89. **"导出另开一条实现路径"是最贵的那种省事**。
90. **扫描器自己就住在它要扫的地址空间里** —— 缓冲复用 + 读完即擦 + 真随机的针。
91. **给"要扫的段"设上限 = 让该看见的副本落在窗口外**（漏扫与没泄是同一条绿）。
92. **解锁期间 `VmLck` 涨的大头不是我们那一页**：`cipher_memory_security` 会给 SQLCipher 的
    每次分配 `mlock`（实测 152 kB 里 148 kB）。
93. **"发现目录出现" ≠ "app 就绪"**：Victauri 的插件 setup 比 app 自己的 `.setup()` 早，
    所以刚连上时 `lifecycle` probe 还是 `{"initialized":false}`。判据要"等那个字段自己出现"。
    同一类还有第二层：`invoke_command` 走 webview bridge，是**最后**才好的一个。
94. **`no-println` 规则豁免的是 `**/tests/**`，不是 `#[cfg(test)] mod tests`**。
    正解：单测的"跳过"换成**不依赖环境**的 fixture，要打印就放进 `tests/`。
95. **用 `chmod` 造"不可写"在单测里站不住** —— 改用**结构性**造法（路径指到普通文件底下，
    `ENOTDIR` 连 root 也绕不过去）。
96. **clippy 的 `undocumented_unsafe_blocks` 本来就看私有项**（1.98 实测）。
97. **只加 `undocumented_unsafe_blocks` 会漏掉另一半**：要靠反向的
    `unnecessary_safety_comment` / `unnecessary_safety_doc` 才闭环。
98. **文档里的"或 X"最容易凭空长出来** —— **核查时把每个"或"当成一条待证断言**。
99. **"唯一一处"这类计数没有人核对就会腐烂**：要么写成"唯一**允许**的 crate"（结构性说法），
    要么就别在注释里写数字。
100. **照搬外部规范时要分清"结构"与"语种"**：解释写中文，只把**标签字面量**钉死。
101. **断言型正则要按"节点实际文本"写**（`^Channel$` 匹配不到 `tauri::ipc::Channel<Vec<u8>>`）。
102. **探针必须放进规则 `files:` 覆盖的真实路径，且正例与诱饵都要有**。
103. **"词汇表"规则里，词边界是规则的一部分**（裸 `(Tab|Pane|Window|View)` 会误伤 `Table`）。
104. **`ast-grep test` 只测规则逻辑，不测 `files:` / `ignores:`** —— 路径范围仍要真实路径探针。
105. **`cargo add` / `cargo search` 要写 `~/.cargo` 的索引缓存**：沙箱下只读 → 按坑 #11 提权。
106. **`cargo deny` 会为 `cargo metadata` 去拉**别的平台**的依赖**（`russh` 带出 `pageant`）。
107. **edition 2024 的 `impl Trait` 会捕获输入生命期** —— 别用 `run_on_socket`，自己写 accept 循环。
108. **`Transport::output_stream()` 只能取一次**（两个读端会互相偷字节）。
109. **`std::env::set_var` 在 Rust 2024 里是 `unsafe`** —— 靠环境变量开关的行为，测试造不出前提；
    正解是把它变成**输入**（`SshAuth::agent_socket`）。
110. **`cargo nextest run -p <crate> <关键词>` 过滤的是测试的**函数名**，不是文件名** ——
    按文件过滤要用 `--test <目标名>`。写错过滤器 = 判据**永远跑不到**。
111. **上游 `russh` 的 `Error::KeyChanged { line }` 跳过注释行时不给行号递增** —— 要给用户真行号
    就自己数一遍（`true_line_of`）。
112. **`thiserror` 把名为 `source` 的字段当成错误源**（插值会编译失败）—— 换个字段名。
113. **`// SAFETY:` 的位置就是它的意思**：写成 `/// SAFETY:` 挂在安全函数上会同时报两条。
114. **迁移必须在动手前先按*旧*版本校验形状**（顺序：读版本 → 按那个版本查表 → 迁移 → 再查表）。
115. **`i64` 与 `u64` 一样过不了 IPC**：用 `u32` 代理 + **checked** 转换。
116. **别给 IPC 投影类型起名 `*View`**：本仓库自己的 `no-ui-vocab-in-types` 会拦下词边界命中。
117. **tokio 1.53 的 `Runtime::handle()` 返回 `&Handle`** —— 存进结构体要 `.clone()`。
118. **React StrictMode（dev）把 effect 走两遍** → "开一个会话"这种副作用会开两次；SSH 那两次的
    提示叠在同一个面板里，表现是"点了 SSH 一直连不上"。处置：连接**推迟一个微任务**再发。
119. **沙箱里"同一次 bash 调用"的边界包括你重定向出去的日志文件** —— 写 `/tmp` 下次读不到，
    要写进**工作区**。
120. **`russh` 的 `Config::nodelay` 只在 `client::connect` 里生效**：我们两条路都走
    `connect_stream`，所以"把 `config.nodelay` 设成 `true`"在这条路上是**一句空话**
    （0502 那句话就这么写了一个版本）。正解：自己建 `TcpStream` 时就 `set_nodelay(true)`
    —— 而"自己建 TCP"正是跳板需要的形状（底层流由调用方给）。
121. **服务端的 `Handler::data()` 对**所有**通道都会被调用**（上游把数据**同时**交给通道自己的
    接收端**和** handler，`server/encrypted.rs:1251`）。所以测试服务端的"回声"必须只对 **shell**
    通道做 —— 否则一条 `direct-tcpip` 通道上的字节会被原样打回去，客户端读到的是**自己刚写的
    SSH id 行**，报出来的是 `Bad packet size: 1397966893`（那串数字就是 `"SSH-"`）。
    教训：**假服务端的每个回调都要问一句"它对哪些通道会响"**。
122. **`copy_bidirectional` 出错时不交出已搬的字节数**，而收尾那一下报错是常态 —— 把计数建在
    它的返回值上，等于让"搬过多少字节"这条判据在最需要它的时候永远是 0。正解：包一层
    `AsyncWrite` 数着写（顺带**会话还开着的时候就读得到**）。
123. **`rusqlite` 不是 app 的 dev-dependency**（`akasha-store` 才是）—— 集成测试里要写
    `Connection` 这个类型时，走 `akasha_store::Connection` 这个**再导出**，别去 `Cargo.toml` 里
    加一份版本要对齐的重复依赖。
