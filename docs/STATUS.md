# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-13

## 一句话

**阶段 2「端到端最小终端」5/5 完成**；**阶段 3「托盘与应用生命周期」6/6 完成**；
**阶段 4「存储与凭据池」9/9 完成**；**阶段 5「SSH 栈」4/6**（0501 ADR / 0502 连接认证 /
0503 known_hosts / **0504 接进 IPC 与前端**）；下一步是 **0505 `direct-tcpip` 原语**。

阶段 4 的八块（一句话各一块）：**SQLCipher 加密库能开**（0401）、**口令只从一条路进来，而且能真的
验证它**（0402 —— 拆开了"打开"与"新建"，原来在没有文件的路径上**任何口令都能开**）、
**口令在内存里也受保护**（0406）、**四套池能增删改查**（0403）、**库里的东西拿得出去也放得回来**
（0404）、**解锁与锁定是一条完整的生命周期**（0407 —— 真 app 上 `VmLck` **0 → 192 → 0 kB**）、
**搬走文件夹之后数据还在且能用**（0405）、**便携目录不可写时拒绝启动**（退出码 2）。

**阶段 5 的形状先定死在 [ADR-0003](./adr/0003-ssh-stack-and-resource-model.md) 里**（plan 0501）：
`russh` 的版本与 feature、运行时归谁、`Transport` 在 SSH 上的映射、`direct-tcpip` 原语的形状、
隧道状态机与重连判据全部**带出处**写死。之后三块依次落地：**连接 + 认证**（0502）、
**主机密钥的信任策略 + 本仓库第一次库格式迁移**（0503）、**接进 IPC 与前端**（0504）。

### 阶段 5 现在到哪了（按依赖排的顺序）

| plan | 做完了什么 | 判据在哪验的 |
|---|---|---|
| 0501 | ADR-0003（D1–D15，带出处的调研 + 资源模型） | `docs/adr/0003`，状态「实现中」 |
| 0502 | `akasha-ssh` 连接 + 认证（D7 的顺序、D8 的内存凭据缓存） | **crate 层**：进程内 SSH 服务端，三个连接只问一次凭据 |
| 0503 | known_hosts 三态判定（D11）+ 库格式 v2 与第一次迁移 | **crate 层**：策略 + 库契约；迁移用真 v1 库 |
| **0504** | **SSH 接进 IPC / 前端**：一条带目标的会话命令、一条"后端问 → 前端答"的往返、一个提示界面 | **真 app**（`ssh_session` E2E + 测试进程内的服务端） |
| 0505 | `direct-tcpip` 原语（跳板 / `-L` / SFTP B 档共用） | 未开始 |
| 0506 | `~/.ssh/config` 受限子集导入 | 未开始 |

**这一轮（0504）把 SSH 真的接到了 app 上**：界面多一个 SSH 入口（列出主机池里已有的行 → 选一台），
后端按那一行连过去，连接中途要问的两件事（**没见过的主机密钥**、**凭据**）走**同一条**往返 ——
一个事件 + 三条回答命令，**120 s 内没人答就拒绝**（D16）。连上之后字节与本地终端走**同一条**
通道（raw 频道）与**同一条**收尾尾巴；关标签页照样是"立刻丢弃这个会话"。

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
| 关一个终端标签页 / 会话自己结束 | **只有那一个会话** |
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
  `.ssh-prompt[data-prompt-kind]`）是**测试接口**，不是 UI 规范。**重做界面属于尚未规划的工作**。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + gen-types-check + docs-check） | 退出码 **0**，**6/6 全绿** |
| `just test` | **235 tests run: 235 passed**（`akasha` **65** + `akasha-core` 15 + `akasha-pty` 39 + `akasha-store` 89 + `akasha-ssh` 27）。⚠️ `akasha` 那 65 条含 `tests/` 下的集成目标（没有 `VICTAURI_E2E` 时它们只打印原因并返回 —— 其中 `portable` 3 条、`ssh_session` 1 条） |
| ↑ **判据：真 app 上开一个 SSH 会话**（plan 0504） | ✅ `ssh_session` E2E（真 app + **测试进程内**的服务端，**2.15 s**）：界面点 SSH → 选中池里那一行 → **主机密钥提示里那串指纹等于服务端的** → 接受 → 口令提示 → 填答 → 连上（标签页标题 = 池里的名字） |
| ↑ **判据：字节能双向流**（plan 0504） | ✅ 终端里敲 `echo ssh-hello` → 屏幕出现回声，**且服务端也收到同一串**（"发出去过"与"对端收到了"是两件事，两个都断言） |
| ↑ **确认过的主机密钥进我们的库**（plan 0504 补上 0503 的空缺） | ✅ 直连库文件读 `known_hosts`：**1 行**（`127.0.0.1:<port> ssh-ed25519`，指纹 = 服务端的）；第二个会话因此**连主机密钥都不问** |
| ↑ **判据：凭据只问一次**（plan 0504，真 app） | ✅ 第二个会话**一次都没问**就连上 —— 谁都没有回答任何提问，而连接必须自己走完；服务端第 2 次收到**同一句**口令（是复用凭据，不是跳过认证） |
| ↑ **判据：关标签页零残留**（plan 0504；SSH **没有本地进程**，所以进程表上看不到） | ✅ 关掉两个 SSH 标签页 → `sessions` probe 回 **`{"live":1,"registered":1}`**（只剩本地那个）+ **服务端看到 2 条连接断开**（对端的证据，不是自报） |
| ↑ **提问往返自身**（plan 0504，crate 级 6 条） | ✅ 答案到得了问的人手里（字节数对得上）/ 超时（50 ms 的桩）会**撤回**那一问、之后作答报 `Gone` / **取消与超时分得开** / **没人答 = `HostKeyUnknown`（拒绝），不是 `Ok`** / 用户接受之后**真的写进缓存** |
| ↑ **`just test-e2e` 全绿** | 退出码 **0**：**16 个 E2E 用例**（+ `ssh_session`）+ 第三段 3 条（`portable`）。`ssh_session` 排在 `vault_unlock` **之后**（两个用例都在动同一个库文件） |
| ↑ **判据：host key 变了就拒绝，没见过的要问一次**（plan 0503） | ✅ `akasha-ssh` 的 7 条：未知且没人可问 → `HostKeyUnknown`（带去核对的指纹，且**认证一步没开始**）；确认 → 进缓存，**第二个连接 0 次提问**；记录对不上 → `HostKeyChanged`（**两个指纹都在**）且**一次都不问**；用户否认 → 拒绝且**不记录**；用户文件里认得 → 连上且文件**逐字节没变**；文件里对不上 → 拒绝并指出**真实行号** |
| ↑ **本仓库第一次格式迁移**（plan 0503） | ✅ `akasha-store` 的 6 条：`DDL_V1` 造出**真 v1 库** → `open` 之后 `user_version = 2`、五张表在、**那条 host 还在**；再开一次当前格式的库**一个字节都不写**；版本 0 仍拒绝；缺表的 v1 **不迁移**；导出与明文导出两条还原路都**升的是副本、来源逐字节不变** |
| ↑ **判据：同主机三个连接只问一次凭据**（plan 0502） | ✅ `three_sessions_ask_for_one_credential`：`provider.calls() == 1`、缓存 `len() == 1`、服务端三次都收到**同一句口令** |
| ↑ **认证顺序是线协议上的事实**（plan 0502） | ✅ 服务端记下的序列：`publickey → password`、`publickey → keyboard-interactive`；agent 不可用时序列里**没有** `publickey` 痕迹 |
| ↑ **D13 对"凭据缓存"这个新用途的重验**（plan 0502） | ✅ `VmLck` 0 → **32 kB**、静止态无权限页 +8；清空并释放后**两者都回到起点** |
| ↑ **`russh` 从"读源码核对"变成编译期事实**（plan 0502） | ✅ `cargo tree -p akasha-ssh \| grep -c aws-lc` = **0**；`ring v0.17.14` 是 lock 里**唯一**的 ring |
| ↑ **`cargo.lock` 的增量只有一行**（plan 0504） | ✅ 加 `akasha → akasha-ssh` 这条边**只多一行**：`russh` / `tokio` / `rand` 本来就在图里（前两者是 app 的 dev-dep、`rand` 是 akasha-ssh 的 dev-dep） |
| `pnpm build`（tsc + vite build） | 退出码 0；产物 **849.86 kB / gzip 233.28 kB**（+6.9 kB：SSH 入口、主机选择器、提示面板） |
| `just docs-check` | 全过（ROADMAP 58 个条目 ≤3 行且无代码块 / 50 份 plan ≤200 行且索引一致） |
| `ast-grep scan` + `ast-grep test` | 都退出 **0**。⚠️ **本轮被自己的规则拦下一次**：`HostView` / `AuthView` / `CredentialKindView` 命中 `no-ui-vocab-in-types`（`View` 是词边界命中）—— 规则工作正常，改名的是代码 |
| **三条 unsafe 注释 lint**（clippy，`just lint` 里） | 退出码 **0**；三条各用一个探针证明**它们真的会红**（探针跑完即撤） |
| ↑ **解锁 / 锁定 / 内存还回去**（0407） | ✅ 真 app 上 `VmLck` **0 → 176～192 → 0 kB**；进程内存扫描（带正对照）：口令 `1 → 2 → 2 → 1` 处、派生密钥 `3 → 1` 处 |
| ↑ **导出与还原**（0404） | 加密导出可在另一目录还原（逐字段一致）；明文导出两道门槛；不泄密带对照组（**0 命中 vs 1 命中**）；失败不留半成品；导出件 **600** |
| ↑ **搬走整个文件夹之后数据还在**（0405） | ✅ `just portable` **3 passed**（搬家 / 不可写拒绝 / 没有便携目录也不拒） |
| ↑ **退出零残留**（0204/0205） | ✅ `app 已退出`；`零残留：忽略 SIGHUP 的子进程已随会话被收掉` |
| ↑ **终端 / 会话判据（未退化）** | `renderer = webgl`；8 MB 灌流后仍可交互；raw 通道 10.73 MB / 164 批；收尾帧 1 个、console 零异常 |
| ↑ **§7 的 registry 那一条：当前不可满足**（坑 #82） | `get_registry` 回 **`[]`**（命令都没标 `#[inspectable]`）。**替代证据**是真路径上的 `invoke_command` 成功 —— 本轮 5 条新命令都是这么验的 |
| `cargo tree -p akasha-core \| grep -c tauri` | **0**（分层成立） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数字，本轮未复跑**） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。

## 待验证（本地跑不了 / 沙箱跑不了）

- **SSH 的测试服务端仍是我们自己搭的**（现在住在 `akasha-ssh::testing`）：它证明的是**客户端这条链**
  与 app 的接线，**不是**与 OpenSSH 的互操作 —— 没连过真的 `sshd`，也没连过任何真实服务器。
  协议细节（算法协商、`exit-status` 的时序、`ssh-rsa` 那类历史包袱）都可能在我们这套服务端上
  恰好成立而在真服务器上不成立。
- **解锁 / 锁定仍然没有界面**：命令、状态、生命周期都在（plan 0407），而界面上**没有能输口令的地方**
  —— 所以 SSH 那条真路径上，**解锁这一步是 E2E 用 `invoke_command` 完成的**（界面只到"选主机"为止）。
  ⚠️ 别把"能开 SSH 会话了"读成"用户从冷启动就能自己走通"：他要先有个能输库口令的地方。
- **主机池的增删改查仍然没有界面**：plan 0504 只加了**只读**的 `vault_hosts`（界面要选主机）。
  要连一台新机器，今天得直接往库里写（E2E 就是这么种数据的）。
- **真实 agent 那条路只有"不可用"被覆盖了**：agent 里真有钥匙、且服务端认它这条从未跑过。
- **并发未命中会各问一次**：`CredentialCache::resolve` 没有 single-flight。
- **并发提问没有实测**：两条连接同时提问时，两条提示会**并排**在面板上（前端按 id 列表渲染），
  但只跑过"一条连接一次一问"。
- **保活那三个数是默认值、不是实测值**（ADR D15）：本 plan 没造出"半死连接"与高延迟链路。
- **私钥与口令各有一份够不着的明文副本**（D8 照实记）：`russh` 要 `ssh-key::PrivateKey` 才能签名、
  要 `String` 才能送口令，两者都在**普通堆**上，我们擦不掉。
- **私钥候选的稳定标识只有行 id**：池里的 `update` 会保留 id，于是"换了材料没换 id"会在**同一个
  解锁窗口内**留下一条过期口令（自愈是现成的：解不开就 `forget` 再问一次；缓存本身内存-only）。
- **提问占住一个 runtime worker**：这是 D16 照实记的代价，靠 4 个 worker + 120 s 超时兜着；
  "两三条连接同时提问"这条路径**没有实测**。
- **只读介质上的 v1 库没有实测**：迁移要写文件，那条路会以 `UpgradeFailed` 失败 —— 代码有这一支，
  但没有造出真只读文件系统来验它。**降级也没有实测**（v2 的库用旧二进制打开）。
- **与 OpenSSH 的 `known_hosts` 互操作没有实测**：`@cert-authority` / `@revoked` 这类标记行的行为
  没有验过；用户文件里读不动的行的处置（`warn` + 当作未知）没有用例守着。
- **解锁 / 导出 / 口令经 IPC 的边界**（照旧）：tauri 自己的两份口令副本够不着（ADR-0002 §7.5）；
  `mlock` 失败那条路只有单测；内存扫描只在 Linux、只扫匿名段。
- **权限位、单实例、托盘在非 Linux 上未验**：CI 的类型检查挡不住运行期差异；CI 三个 job 至今
  没跑过（仓库没有 remote）。
- **前端类型检查不在任何门禁里**：`just ready` 只覆盖 Rust + 文档，`pnpm build`（tsc）要手动跑。
- **`just dev-web` 的模拟后端没在真浏览器里点过**：SSH 两条新命令在里面**明确报错**
  （"没有 SSH 客户端"），所以主机选择器在浏览器里只会显示那句话。

## 当前基线（2026-09-13 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` / `akasha-store` / `akasha-ssh` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 后端模块 | `bindings` / `session` / `tray` / `config` / `lifecycle` / `single_instance` / `vault` / `watchdog` / **`ssh`（SSH 的长住状态 + 那条命令 + 库内 known_hosts 适配器）** / **`prompt`（提问往返）** / **`pools`（池的只读读取）** |
| **命令清单** | `greet` · `vault_status` / `vault_unlock` / `vault_lock` · **`vault_hosts`** · `open_session` · **`open_ssh_session`** · `write_session` / `resize_session` / `close_session` · **`ssh_prompt_credential` / `ssh_prompt_host_key` / `ssh_prompt_cancel`**（事件：`session_ended` · **`ssh_prompt` / `ssh_prompt_dismissed`**） |
| **probe** | `lifecycle` → `{close_behavior, tray_ready, close_action}`（没登记时 `{"initialized":false}`，坑 #93）；`single_instance` → `{registered, activations}`；**`sessions` → `{live, registered}`**（plan 0504 加：SSH 没有本地进程，"零残留"只能看注册表）。**库没有 probe**：状态本身就是命令（`vault_status`） |
| 出字节路径 | PTY / SSH read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write`。**两条载体共用同一条尾巴**（`session::open_terminal`） |
| **SSH 的 IPC 层**（plan 0504） | `src-tauri/src/ssh.rs`：app 启动时建**一个**专用 tokio runtime（**4 个 worker**，D2）；`open_ssh_session` 是 **async 命令**（不挡 IPC），内部起一条**普通 `std::thread`** 跑同步门面（`spawn_blocking` 的线程**也算** tokio 上下文，会吃到 `BlockingInsideRuntime`），结果经 `tokio::sync::oneshot` 回来；连上之后与本地终端**共用**注册 / 频道 / 收尾那条尾巴。`SshConnect` 的四样材料按池行组：`password` → 不用 agent、不带钥匙；`agent` → 只用 agent；`publickey` + `key_id` → 那一把钥匙（PEM → 受保护页 → `KeyCandidate`，标识 `key#<id>`） |
| **提问往返**（plan 0504，ADR-0003 **D16**） | `src-tauri/src/prompt.rs`：`Prompts`（`Arc` + 待答表 + 可注入的发布口）；事件 `ssh_prompt`（判别式：`hostKey` / `credential`）+ `ssh_prompt_dismissed`；三条回答命令；编号从 1 起、只增不减；**超时 120 s → 拒绝 + 撤回**；答过 / 超时的 id → `PromptError::Gone`；**主机密钥那一问只认"接受 / 拒绝"**，超时 / 取消 / 答错类型一律 `Err(HostKeyUnknown)`（= 拒绝连接） |
| **库内主机密钥缓存**（plan 0504 接线） | `VaultHostKeys`：`Vault` 是 `Arc` 可克隆的（照 `Sessions`），`with_conn` **短借**连接；库锁着 → `SshError::HostKeyCache` → **拒绝连接**（不当作未知）。⚠️ **绝不在持锁期间连接**：`remember` 会在连接中途回头锁库 |
| **库的解锁状态** | `Vault { inner: Arc<Mutex<Option<Unlocked>>> }`（`Clone`）；`Unlocked { conn, passphrase }` 同生共死。借库的失败分两种（`ConnError`：`Locked` / `Store`）—— 因为 SSH 那条路要单独认出 `NoSuchRow`（"你挑错了主机"） |
| **`akasha-ssh` 的形状** | 七个模块：`target` / `credential` / `keys` / `handshake` / `known_hosts` / `transport` / **`testing`（进程内测试服务端，plan 0504 从 `tests/support/` 提上来 —— app 的 E2E 也要起它；生产代码别用）**。⚠️ `rand` 因此从 dev-dep 变成**真依赖** |
| **错误分域** | `akasha-ssh` 新增 `HostKeyCache(String)`（库那一侧读不动缓存）；app 侧 `SshIpcError`：`Locked` / `NoSuchHost` / `Failed { kind, message }`（`kind` 区分 `hostKeyChanged` / `hostKeyRejected` / `hostKeyUnknown` / `hostKeyCache` / `auth` / `connect` / `other`）/ `Internal` —— **前端按 `kind` 分辨警报**，不匹配消息字符串 |
| **前端结构** | `src/ipc/`（`session.ts` 会话 + `prompts.ts` 提问往返 + `hosts.ts` 池读取 —— 唯一允许碰后端的目录）、`src/tabs/`、`src/terminal/`、**`src/ssh/`（主机选择器 + 提示面板）**、`src/App.tsx`。标签页 `kind`：`terminal` / `ssh`（**都有关闭按钮**，规则写成 `CLOSABLE` 清单） |
| **前端的一个 dev-only 陷阱** | React StrictMode 把 effect 走两遍 → SSH 会话会被开两次（两次的提示叠在同一个面板里）。处置：SSH 那条连接**推迟一个微任务**再发（坑 #118） |
| **SSH 栈**（ADR-0003） | `russh = "=0.63.3"`、features `["ring","rsa"]`；`akasha-ssh` 只收 `tokio::runtime::Handle`；对外是同步 `Transport` 门面 + 两条**有界** mpsc（满 → `TransportError::Busy`）；capability = `resize + exit_status`、`session_leader() = None` |
| **连接取值**（D15） | `connect_timeout = 10s`；`keepalive_interval = Some(30s)`、`keepalive_max = 3`；`nodelay = true`。默认值只有一处：`SshConfig::default()`。⚠️ 这三个数是**有理由的默认值**，不是实测出来的 |
| **库格式与迁移** | `FORMAT_VERSION = 2`；v1 = 四张池表（`DDL_V1` 冻结、公开），v2 = v1 + `known_hosts`。`open` 里 `upgrade()`：`== 2` 什么都不做；`1` → 先按 v1 校验形状 → **一次事务**里加表 + 写版本号；`> 2` 与 `0` 拒绝；写不动 → `UpgradeFailed`。⚠️ **降级不行**。**导出/还原不迁移来源** |
| **库文件的磁盘事实** | `akasha.db`；建库后 **36864 字节 = 9 页**；SQLCipher 4.5.7 + vendored OpenSSL 3.6.3 + 内嵌 SQLite **3.46**；`user_version = 2` 是格式权威；盐 16 字节随机；显式收紧到 **600**；不带 `-wal` / `-shm`；解锁代价 **~105 ms**（KDF） |
| **四套池** | `keys` / `hosts` / `serials` / `forwards`，各 5 个函数 + 反查。`New*`（没有 id）与 `*`（有 id）**是两种类型**；不变量写在库上（`STRICT` + `CHECK` + 外键 `RESTRICT`，D14） |
| **known_hosts 缓存** | 表 `known_hosts(id, host, port, key_type, key_blob, fingerprint)`，`UNIQUE (host, port, key_type)`。**缓存不是池**；判定材料是 `key_blob`（逐字节比）；`remember` 遇到同键不同值 → `Conflict`（**写路径上就不许静默改写**） |
| **主机密钥的三态判定** | `KnownHostsVerifier`：**库 → 用户的 `~/.ssh/known_hosts`（只读）→ 提问**。库里 / 文件里对不上 → `HostKeyChanged`（**不看不问**）；两边都没有 → 有 `HostKeyPrompt` 就问、确认后 `remember`。两个注入点是**同步** trait（`HostKeyCache` / `HostKeyPrompt`） |
| **口令与私钥** | `Passphrase` / `PrivateKey`：空值**造不出来**、**没有 `Debug`**、本体住在 `memsafe` 的受保护页；`PassphraseInput` 是口令**经 IPC 进来的唯一形态**（vault 解锁与 SSH 凭据**共用**它） |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| CSP | `default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| capabilities | 仍只有 `core:default` + `opener:default`（+测试用的 `victauri`）。**SSH、托盘、配置、单实例、便携目录检查都没有加任何 permission**（全在 Rust 侧） |
| 前提条件 | **需要能写 `$HOME`**；托盘另需能写 `$XDG_RUNTIME_DIR`、单实例另需会话总线（否则各自只降级）。**便携目录存在时另需可写 —— 不可写是拒绝启动**（退出码 2） |

## 进行中 / 下一步

- [ ] **下一步 = 阶段 5 的 plan 0505**（[`direct-tcpip` 原语](./plans/0505-direct-tcpip-primitive.md)）：
  它是**未规划（骨架）**，要先补齐「步骤」与「验收命令」。**0504 已经把它要用的那条会话打通了**
  （真 app 上能开 SSH 会话），所以现在验得了"ProxyJump 可连通只对跳板机可见的目标"。
  ⚠️ 接口归属：**`Handle` 仍归 `SshTransport` 自己**（交给那条 `pump` task，收尾的唯一出口），
  0504 **没有**改这一点 —— 原语要么由 `SshTransport` 暴露一条受控借用口，要么跳板那条路自己持有连接。
- [ ] **plan 0506**（`~/.ssh/config` 受限子集导入）排在 0505 之后：导进来的 `ProxyJump` 要能真用。
- [ ] **阶段 4 留下的界面缺口**（不是 bug，是没有规划的工作）：**解锁界面**（今天 SSH 那条真路径上，
  解锁由 E2E 的 `invoke_command` 完成）与**主机池的增删改查界面**（今天只能直接写库）。
- [ ] **降级路径没有实测**：v2 的库在旧版本程序里会以 `UnsupportedVersion { found: 2 }` 被拒（有意）。
- [~] **plan 0102（CI 平台矩阵）**：本地部分完成，最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测，剩 CI 三平台格子
- [ ] **正式 UI**：等设计稿（见上面「UI 现状」）—— 没有验收标准，故**不进 ROADMAP**

> 阶段 5 的编号按依赖重排过（2026-09-13）：0503 known_hosts · **0504 接进 IPC / 前端** ·
> 0505 `direct-tcpip` 原语（原 0503）· 0506 `~/.ssh/config` 导入（原 0504）。
> 理由：信任策略是**接口形状**（先行），而原语的消费者都要先有一条**从 app 打得开的**
> SSH 会话才验得了。编号与执行顺序现在一致，索引里有一段重排说明。

### 本轮完成（plan 0504：SSH 接进 IPC / 前端）

**判据（ROADMAP 原文）**：真 app 上开一个 SSH 会话 —— 字节能双向流、凭据只问一次、关标签页零残留。

- [x] **一条带目标的会话命令**：`open_ssh_session` 按主机池的一行连过去；`Sessions::register`
  本来就是 `<T: Transport>`，所以 SSH 与本地终端**共用**注册 / 频道 / 收尾那条尾巴（ADR-0003 D3 的兑现）
- [x] **一条"后端问 → 前端答"的往返**（ADR-0003 新增 **D16**）：一个事件 + 三条回答命令，
  **120 s 超时即拒绝**，被撤下时再发一个 `ssh_prompt_dismissed`（界面不能留着一问早已不存在的提示）
- [x] **库连接做成可共享句柄**（0503 留下的第一件事）：`Vault` → `Arc` + `with_conn` 短借；
  `HostKeyCache` 适配器接到 `akasha_store::known_hosts`；⚠️ **绝不在持锁期间连接**
- [x] **判据实测**（真 app + 测试进程内的服务端）：见上表三行 —— 指纹核对 / 双向流 / 库里有记录 /
  凭据只问一次 / 关标签页服务端看到断开
- [x] **顺手做掉的三件**：服务端提成 `akasha_ssh::testing`（app 的 E2E 也要用它）、
  新增 `sessions` probe（SSH 没有本地进程）、`session.rs` 抽出 `open_terminal`
- [x] **门禁**：`just ready` **6/6**；`just test` **235 passed**（+7）；`just test-e2e` **退出码 0**
  （16 个用例 + 第三段 3 条）；`pnpm build` 退出码 0
- [x] **记账**：plan 0504 改「已完成」并 `git mv` 进 `archive/`（索引与 ROADMAP 指针同步）；
  ADR-0003 加 D16 + 四处小改（D2 标"已落地"、§11 补 drop 顺序、§12 删两条、§14 记一行）；
  `scope.md` §2.2 补一行"交互式提问"

⚠️ **没做到的照实记**：与真 OpenSSH / 真 `sshd` 的互操作、并发提问、解锁界面与池的 CRUD 界面
（都进了「待验证」与「下一步」）。

### 上一轮完成（plan 0503：库格式 v2 迁移 + known_hosts 三态判定）

- [x] **先做了一次计划重排**（用户当次指令）：阶段 5 的编号改由**依赖**决定，并新立 plan 0504
  （= 上一轮发现的缺口：SSH 没接进 IPC / 前端）
- [x] **三态判定**：库里同类型一致 → 连；**对不上 → `HostKeyChanged`（两个指纹都在，且不问）**；
  两边都没有 → 有 `HostKeyPrompt` 就问、确认后进缓存、否则 `HostKeyUnknown`
- [x] **用户的 `~/.ssh/known_hosts` 只读**：认它就连，且**逐字节不变的断言**守着
- [x] **本仓库第一次格式迁移**：v1 的库在 `open` 时自动升到 v2（**一次事务**）；迁移**先按库里写的
  版本校验形状**（缺表的 v1 是坏库，不是"待迁移"）
- [x] **导出件不被改写**：`restore` 用 `open_unmigrated` 打开来源，升级发生在**新写出来的目标**上。
  ⚠️ **偏差照实记**：ADR-0002 §6 那句"走 `open`"字面上不再成立（`open` 现在会为了迁移而写），
  但那条**决定**一字未变；ADR-0002 已定案不可改，偏差留在这里
- [x] **门禁**：`just ready` **6/6**；`just test` **228 passed**

### 上一轮完成（plan 0502：`akasha-ssh` 连接 + 认证）

- [x] **新建 `src-tauri/crates/akasha-ssh`**：五个模块，对外**只是 `akasha_pty::Transport`**
- [x] **判据实测**：三个连接只问一次凭据（`provider.calls() == 1`）
- [x] **认证顺序是线协议上的事实**；**凭据缓存**（D8）的值住**同一页受保护内存**
- [x] **顺带改了两处 ADR 自己的错**（实现中状态可改，§14 记了修订）：`-R` 的验收归属改到 0604；
  新增 **D15**（超时 / 保活 / `nodelay`）
- [x] **门禁**：`just ready` **6/6**；`just test` **208 passed**

### 更早（plan 0501：ADR-0003 进入「实现中」）

- [x] **调研先行、结论带出处**：下载 `russh-0.63.3` 源码逐条核对（行号可复查）+ crates.io API
- [x] **14 条决策**（每条带理由与否决的替代路）+ 两条量出来的硬事实（`ring 0.17.14` 正是 lock 里
  已有的那个；上游把 `ssh-key` 钉在预发布）
- [x] **顺带查出两处"实现时要改的现状"**（写进 ADR §12，plan 0502 落地）
- [x] **门禁**：`just docs-check` 0；`just ready` **6/6**

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接跑 `cargo …` 会失败（坑 #8），一律用 `just` 转发。⚠️ **临时脚本里也一样**。
- **五个 crate 的分工**：`akasha-core`（Session 模型 + 配置模型与判据，**零 Tauri 依赖**）、
  `akasha-pty`（`Transport` + portable-pty + 合批 + `teardown` + `watchdog`）、
  **`akasha-store`**（库的打开 / 创建 / **格式版本与迁移** / 四套池 / **known_hosts 缓存** /
  dump / 导出与还原 —— 唯一允许 `unsafe` 的地方）、
  **`akasha-ssh`**（连接 + 认证 + known_hosts 三态 + **同步 `Transport` 门面** +
  **`testing`（进程内服务端）**）、
  `akasha`（app 包 = IPC 薄壳 + 托盘 + 配置 + 数据目录 + 关窗语义 + 单实例 + 退出钩子 +
  看门狗接线 + **库的解锁状态** + **SSH 的 runtime / 提问往返 / 池读取** + 代码生成 bin）。
- **前端**：`src/ipc/`（唯一允许碰后端）、`src/tabs/`、`src/terminal/`、**`src/ssh/`**、`src/App.tsx`。
- **调试"SSH 为什么连不上"**：日志里 `ssh session opening` / `ssh authenticated`（带 `method`）；
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
    所以刚连上时 `lifecycle` probe 还是 `{"initialized":false}`。**它正是我们那条拒绝启动的检查
    所在的位置之后**才登记 —— 判据要"等那个字段自己出现"（有界），别把中间态当答案。
    同一类还有第二层：`invoke_command` 走 webview bridge，是**最后**才好的一个，
    所以"能连上 MCP"也不等于"能 `invoke_command`"。
94. **`no-println` 规则豁免的是 `**/tests/**`，不是 `#[cfg(test)] mod tests`** ——
    在 `src/` 的测试模块里写 `eprintln!("跳过：…")` 会直接红在 lint 上。
    正解：单测的"跳过"换成**不依赖环境**的 fixture（见下一条），要打印就放进 `tests/`。
95. **用 `chmod` 造"不可写"在单测里站不住**（以 root 跑、或文件系统不理会 mode 位时就造不出来，
    而"跳过"又会踩上一条）。改用**结构性**造法：把路径指到一个普通文件底下 —— `ENOTDIR`
    连 root 也绕不过去。要"app 真的看见一个不可写目录"时再用 `chmod`，且**自己先试写一次**
    当正对照（写不进去才继续）。
96. **clippy 的 `undocumented_unsafe_blocks` 本来就看私有项**（1.98 实测）：去掉
    `clippy.toml` 之后，**私有**函数 `apply_key` 里的 `unsafe` 照样被报（把 `// SAFETY:`
    改名即红）。所以别照抄内核的 `check-private-items`（它服务的是另一类 lint）；
    反过来，哪天它不再报私有项，就是**静默失效** —— 那时才需要那个开关。
97. **只加 `undocumented_unsafe_blocks` 会漏掉另一半**：注释**写在了安全块上**要靠反向的
    `unnecessary_safety_comment` 才拦得住（探针：在一条安全语句上挂 `// SAFETY:` 会红）；
    `/// # Safety` 挂在安全函数上同理，靠 `unnecessary_safety_doc`。三条一起才闭环。
98. **文档里的"或 X"最容易凭空长出来**：`portable.md` 与 `scope.md` 都写着
    "（或便携标记文件）"，而代码里从来没有第二个标记 —— `config.rs` 的注释反而写着
    "目录本身就是标记，不需要额外再放一个标记文件"。**核查时把每个"或"当成一条待证断言**。
99. **"唯一一处"这类计数没有人核对就会腐烂**：规则 note 与两处代码注释都写着
    "全仓库唯一的 `unsafe` 单点"，实际是 **3 处**（生产 1 + 契约测试 2）。要么写成
    "唯一**允许**的 crate"（结构性说法，能被规则守住），要么就别在注释里写数字。
100. **照搬外部规范时要分清"结构"与"语种"**：内核的 unsafe 规范其实只有两件事 ——
    `// SAFETY:` 紧贴块前说明"为什么 sound"、`# Safety` 写明契约。第一版把
    "英文、句首大写、句末句号"也一起搬了过来，而那只是内核注释本来就写英文。
    结论：**解释写中文**（与本仓库其余注释一致），只把**标签字面量**钉死（clippy 认它）。
101. **断言型正则要按"节点实际文本"写，不是按心里那个短名字**：`^Channel$` 匹配不到
    `tauri::ipc::Channel<Vec<u8>>` —— 那个 `type` 节点的文本是**整条路径**。写成
    `(^|::)Channel$` 才拦得住。教训：规则的正例必须覆盖**真实写法**（限定路径、类型别名、
    泛型嵌套）；只测最顺手的写法，等于给规则留了一个静默的缺口。
102. **探针必须放进规则 `files:` 覆盖的真实路径，且正例与诱饵都要有**：只放正例分不清
    "规则在工作"与"规则范围写窄了"，只放诱饵分不清"规则在工作"与"什么都没匹配到"。
    ⚠️ 探针放错路径时，`files:` 即使写错也照样全绿 —— 那正是最该抓到的失败。
    跑完用 `--json` 逐条对账（本轮：正例 10 条命中 / 诱饵 0 条），然后删掉探针。
103. **"词汇表"规则里，词边界是规则的一部分**：`no-ui-vocab-in-types` 原来的裸
    `(Tab|Pane|Window|View)` 是**子串**匹配，于是 `Table` / `Panel` / `Viewport` 一起被拦
    （它们是另一批词，却会让规则看起来"很严"）。改成 CamelCase 词边界
    `(^|[^A-Z])(Tab|Pane|Window|View)([^a-z]|$)` 之后：`Table`/`Panel`/`Viewport` 放行，
    `TabItem`/`NegTabProbe`/`MyWindowSpec` 照样命中。**误报会让人把规则整个关掉，比漏报更伤。**
104. **`ast-grep test` 只测规则逻辑，不测 `files:` / `ignores:`**（实测：测例不是真实路径
    下的文件，路径范围被完全忽略）。而且 `invalid` 用例要 `__snapshots__/` 基线 ——
    改规则后用 `ast-grep test -U` 更新，**那份差异就是"规则行为变了"的评审点**。
    路径范围仍要按 §6 用真实路径探针复核；两者是左右手，不是替代关系。
105. **`cargo add` / `cargo search` 要写 `~/.cargo` 的索引缓存**：本沙箱下报
     `failed to write cache … Read-only file system (os error 30)`，而 `cargo add --dry-run`
     会因此**在解析之前就中止**（输出仍以"aborting add due to dry run"收尾，看起来像成功）。
     → 加依赖时别把 `--dry-run` 的退出码当成"解析通过"；真要加就按坑 #11 那一类**直接提权**。
     这也是"ADR 里的版本结论只到源码级、没有编译级证据"的原因（见「待验证」）。
106. **`cargo deny` 会为 `cargo metadata` 去拉**别的平台**的依赖**：加了 `russh` 之后，
    `just deny-offline` 开始尝试下载 `pageant` / `windows-numerics`（`russh` 的
    `cfg(windows)` 那一支），而 `~/.cargo/registry/cache` 在沙箱里是只读的 →
    `failed to open … Read-only file system (os error 30)`。这与坑 #11 同一类（写工作区之外），
    **提权重试即过**。教训：一条依赖**跨平台**的 crate 会让门禁在**所有平台**上拉齐依赖树 ——
    `cargo build` 成功不代表 `cargo deny` 能跑（后者要全目标）。
107. **edition 2024 的 `impl Trait` 会捕获输入生命期**：`russh::server::Server::run_on_socket`
    返回的 future 借了 `&mut self` 与 `&listener`，于是它**不能**被丢进 `tokio::spawn`
    （`E0597: does not live long enough`，而报错指向那两个局部变量，不指向 `impl Trait`）。
    正解不是 `Box::leak`，而是**别用那个便利方法**：自己写 accept 循环 +
    `russh::server::run_stream(config, socket, handler)` —— 三样都被 move 进任务，没有借出。
    测试服务端（`akasha-ssh/tests/support`）就是这么搭的。
108. **`Transport::output_stream()` 只能取一次**，所以"同一载体上的多段断言"要**读在同一次**里：
    先写、再 resize、然后一次 `read_until` 读到两个证据（服务端把收到的尺寸回声回来）。
    分成两次 `round_trip` 会直接 panic 在"读端只能取一次"上 —— 而那条契约是**故意**的
    （两个读端会互相偷字节，`akasha-pty/src/transport.rs`）。
109. **`std::env::set_var` 在 Rust 2024 里是 `unsafe`**，而本仓库只允许 `akasha-store`
    出现 `unsafe`（§3.4 + ast-grep 规则）—— 也就是说**凡是靠环境变量开关的行为，测试就造不出前提**。
    正解：把它变成**输入**（`SshAuth::agent_socket`），而不是在测试里改环境。
    这条对将来所有"靠 env 配的东西"都适用。
110. **`cargo nextest run -p <crate> <关键词>` 过滤的是测试的**函数名**，不是文件名 ——
    一个都不匹配时会打印 `Starting 0 tests` 并**以 `error: no tests to run` 退出**，
    看起来像"这个目标里没有测试"。按文件过滤要用 `--test <目标名>`。
    计划的「验收命令」里写错过滤器，等于让判据**永远跑不到**（而它还是会"通过"地失败）。
111. **上游 `russh` 的 `Error::KeyChanged { line }` 跳过注释行时不给行号递增**
    （`russh-0.63.3/src/keys/known_hosts.rs` 的 `continue` 排在 `line += 1` 之前），
    于是它给的是"非注释行的序号"。实测：注释行 + 记录行拿到 `1` 而不是 `2` ——
    **照抄它写进给用户看的消息，就是把用户指到别的行**。要真行号就自己数一遍
    （`true_line_of`，顺带把 `RecordedIn::UserFile::line` 做成 `Option`：找不到就说文件名，不猜）。
112. **`thiserror` 把名为 `source` 的字段当成错误源**：`#[error("…{source}…")]` 想要的是
    文本插值，实际会编译失败（`the method as_dyn_error exists for &T … not satisfied`）。
    改个名字（`recorded_in`）就好 —— 这条只在**报错信息**里看得出来，别去怀疑 Display 实现。
113. **`// SAFETY:` 的位置就是它的意思**：把说明写成 `/// SAFETY: …` 挂在**安全函数**上，
    clippy 会同时报两条 —— `unnecessary_safety_comment`（挂错了地方）与
    `undocumented_unsafe_blocks`（真正的 `unsafe` 块前一行什么都没有）。
    正解是把那段说明写成 `//`（不是 `///`）紧贴在 `unsafe` 块之前，函数自己的文档另写。
114. **迁移必须在动手前先按*旧*版本校验形状**：`open` 原来是"版本号不等就拒"，
    改成"旧版本 → 迁移"之后，一个**缺表的 v1 库**会被当成"待迁移"接下去 ——
    结果是升完版本之后才报缺表，库里已经多了一张永远不该出现的表。
    顺序是：读版本 → 按**那个**版本查表 → 迁移（一次事务里连 `user_version` 一起写）→ 再按当前版本查表。
115. **`i64` 与 `u64` 一样过不了 IPC**（生成器拒绝整类 64 位整数）：池行的 id 照
    `SessionHandle` 的做法用 `u32` 代理 + **checked** 转换。⚠️ 别在壳层开那个"危险地当
    number 用"的开关 —— 它会让**将来每一个** 64 位字段都悄悄失去保护。
116. **别给 IPC 投影类型起名 `*View`**：本仓库自己的 `no-ui-vocab-in-types` 会拦下
    `Tab` / `Pane` / `Window` / `View` 的**词边界**命中（`HostView` 就是这么撞上的）。
    改名要连生成物一起动：`HostEntry` / `AuthMethod` / `SecretKind`。
    教训：加一条词汇表规则之后，**取名时先在心里过一遍它**，比事后改名便宜。
117. **tokio 1.53 的 `Runtime::handle()` 返回 `&Handle`，不是 `Handle`** ——
    `.map(Runtime::handle)` 得到的是引用（`E0277`：期望 `Handle`、拿到 `&Handle`）。
    存进自己的结构体时要 `.map(|runtime| runtime.handle().clone())`。
118. **React StrictMode（dev）把 effect 走两遍（挂载 → 清理 → 挂载，同一次 commit 内）**：
    对"开一个会话"这种副作用的后果是**开两次**。本地 PTY 那份无害（多起一个 shell 再收掉），
    而 SSH 会在连接中途**弹提示** —— 两次的提示叠在同一个面板里，用户只答其中一个，
    另一个一直等（最长 `PROMPT_TIMEOUT`），表现是"点了 SSH 一直连不上"。
    处置：把 SSH 那条路的连接**推迟一个微任务**再发（同一次 commit 里排的微任务在清理之后才跑，
    于是第一遍那次根本没发出去）。本地那条不变 —— 少一次 IPC 往返更好。
119. **沙箱里"同一次 bash 调用"的边界包括你重定向出去的日志文件**：把 E2E 的完整输出写到
    `/tmp` 里，下一次调用读不到它（每次调用一个私有 `/tmp`，坑 #33 的同一个机制）。
    要看就得写进**工作区**（例如 `src-tauri/target/e2e-run.log`）。
