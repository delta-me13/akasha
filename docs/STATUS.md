# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。
>
> 本文件**只留现在**：历史会话段、被取代的读数、已处置的问题条目由 `just docs-archive`
> 移入 [`archive/status-history.md`](./archive/status-history.md)（规则见 `AGENTS.md` §8.3），
> 行数预算写在根 `justfile` 的 `STATUS_BUDGET` 里、只降不升。

**最后更新**：2026-09-27

## 摘要

**2026-09-27：macOS 覆盖审计（本轮）** —— 按 `AGENTS.md` / `docs/scope.md` / `ROADMAP.md` 的
需求清单逐条核对 macOS 的实现与验证状态，并用两处独立读数校正文档：CI 最新两次运行
（`36249980440`、`36253379376`）**六格全绿**（含 `E2E（macOS）`），本次另取 `E2E（macOS）`
的作业日志逐行读。结论：平台无关的规则域与绝大多数平台能力都有 macOS 读数，**两处真实缺陷**
与**两处口径不一致**记为新问题 **#181** / **#182**，并开三份 plan：**0112**（进程级判据真实化）、
**0307**（从 Dock 唤回窗口，骨架）、**0408**（可搬迁性的平台口径分档）。⚠️ **#181 是静默失效**：
`exit_residue` 在 macOS 上 0.22 s 空过，此前"第二段 1/1"不能当作零残留判据成立。

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
| **`just ready`（合并后，本机 Windows 11 / MSVC）** | **6/6**：`fmt-check` 1s · `lint` 3s · `test` **18s**（前一轮同配方 139s，差别只在缓存）· `deny-offline` 3s · `gen-types-check` 28s · `docs-check` 40s。⚠️ 在此之前同一个配方在本机红了三轮，三次都停在那条已知偶发（`bw::acquire` 的 install 用例，连续执行 10 次里 7 次红）—— 定位与处置见问题 #165，修后复测 10 次全过 |
| **`just test-e2e`（合并后，本机）** | 退出码 **0**：第一段 **28 个目标 / 33 条用例**、第二段 `exit_residue` **1/1**、第三段 `portable` **3/3**；跳过的目标与原因不变（见 #176）。`tunnel_reconnect` **27.69 s** 通过 —— #179 是在 macOS 的 CI 上红的，本机一直过，处置是否消掉那次红要等 CI |
| **CI 运行 `36249980440`（@ `bda9324`，把分支合入 `main` 之后）** | **六格全绿**：`检查（Linux / Windows / macOS）` 与三个平台的 E2E 全部通过。上一轮的两处红都消掉了：**Linux 的 `vault_unlock`** 报出三次真实读数（`VmLck_解锁前=0 VmLck_解锁中=160 VmLck_锁定后=0`，问题 #180）、**macOS 的 `tunnel_reconnect`** 通过（问题 #179）。Windows 的 E2E 日志 **180,895 字节 / 2,230 行**、`LNK4099` **0 行**（#178 的处置在这条链上仍然成立）|
| **`just test-e2e`（Windows 11 / MSVC，本机实测）** | 退出码 **0**，**全绿**：第一段 **28 个目标全部通过**（33 条用例，0 失败）、第二段 `exit_residue` **1/1**、第三段 `portable` **3/3**。整体跳过 4 个目标（`tab_close` / `window_close` / `bitwarden_login` / `bw_import`），另有 5 个目标里各 1 条用例按平台跳过（`vault_unlock` 的 `VmLck`、三条串口的设备节点、`single_instance` 的 `/proc` 实例计数），原因都在日志里逐条写明（见 #176）。走到这一步修掉的是四类**与平台绑定**的问题：`$!` 的 PID 命名空间与运行中覆盖 exe（#162）、ConPTY 启动时要求先答 `ESC[6n` 且行尾必须是 CR（#175）、tokio 的连接在 Windows 上读不出"连接被拒"（#173）、隧道用例清场时先删主机行后删规则行（#174）。⚠️ 仍被跳过的那两类就是**平台缺口的现状**：会话级的进程回收（Job Object，plan 0108 的遗留）与假 `bw` 的可执行形态 |
| **`just test-e2e`（Windows runner，CI）** | 六个 job 里 `E2E（Windows）` **通过**（run `36231407751` @ `afc3d11`）—— 本轮三轮读数：第一轮 4 个目标红（`terminal_render` ×2、`single_instance`、`windows_ports`、`portable`）、第二轮只剩 `portable`、第三轮 **0 个**。⚠️ 与 Windows 无关的两处仍在：Linux 的 E2E 红在 `vault_unlock`（`读不到 app 的 /proc/<pid>/status`，三轮都在、与本次改动无关），第三轮另有一次 `sftp_host_to_host` 的凭据超时（该目标前两轮都过、文件未被本次改动触碰）|
| **`just test-e2e`（macOS 26.6.2 / arm64，经 `just runner-run test-e2e` 在沙箱外执行）** | 退出码 **0**，**全绿**：第一段 **28 个目标 / 37 个用例**全过（0 失败），第二段（`close_behavior=exit`）**1/1**，第三段（可搬迁性）**3/3**。⚠️ 走到这一步之前红过六处，全部是**"这条路径自己的前提"**：`tab_close` / `window_close` 的进程判活读 `/proc`（非 Linux 上门控，改用 `sessions` probe 那条与平台无关的断言）、`vault_unlock` 的 `VmLck` 同理、`tab_close` 关闭最后一个标签页之后注册表**就该是 0**（写成"回到起点"会让它必红，还把界面留在空状态，后续目标由此连带红）、E2E 发现目录的 `TMPDIR` 分叉（问题 #171）、导入要的 `USER`（问题 #172）。⚠️ **第二段这个 1/1 在 macOS 上是空过**（本会话发现，问题 #181）：该用例的 `alive()` 读 `/proc` 且无平台门控，CI 日志里它 `0.22s` 通过 —— 不得把这一格当作"零残留判据成立" |
| **`just ready`（经 `just runner-run ready` 在沙箱外执行）** | **6/6 通过**（退出码 0，`test` 一步 254s）。此前同一环境上红过两次 `just test`：一次是 `pty::local` 的 `openpty`（沙箱内，见上），一次是 4 条 `bw::acquire`（回环假上游 `Connection reset by peer`）—— 后者与上面 `just test` 那行同一条已知偶发，紧接着重新执行**全绿** |
| `just ready`（fmt-check + lint + test + deny-offline + gen-types-check + docs-check） | 退出码 **0**，**6/6 全部通过** |
| `just test`（macOS 26.6.2 / arm64，经 `just runner-run test` 在沙箱外执行） | **458 tests run: 458 passed**（连续两次结果相同）。红过并已处置的六处：三条看门狗用例（会话枚举缺失，本会话补齐）、`pty` 的三条 `/proc` 判活与探针引号（问题 #45 那一类）、`store` 的权限对比断言（macOS 上 SQLite 建库本就 0600，对比判据退化为直接断言）、串口的 pty 脚手架（门控到 Linux，见「待验证」）。⚠️ `akasha-bw` 的 `install_takes_the_newest_release…` 在一次运行里报 `Peer disconnected`（问题 #165），随后两次运行都通过 |
| `just test` | **473 tests run: 473 passed**（`akasha` **98** + `akasha-bw` **56** + `akasha-core` 30 + `akasha-pty` **40** + `akasha-serial` **25** + `akasha-ssh` **88** + `akasha-store` 136）。⚠️ `akasha` 的 91 条包含 `tests/` 下的集成目标（未设置 `VICTAURI_E2E` 时它们只输出原因并返回；目标与用例数见下面那条 `just test-e2e` 与 `src-tauri/justfile` 的 `E2E_TARGETS`） |
| `cargo check -p akasha-core -p akasha-pty -p akasha-serial --target x86_64-pc-windows-msvc`（plan 0108） | 三条都**退出码 0** —— 改之前 `akasha-pty` 是 3 个错误（E0432 `rustix::process` + E0433 ×2）、`akasha-serial` 因依赖它同样红。负例：撤掉 `teardown.rs` 的 `#[cfg(unix)]` 立刻重新变红（`cannot find module or crate rustix`），恢复后又回到 0 |
| `just serial-check`（plan 0802） | 退出码 **0**：`akasha-serial` 的 **25 条**在两种 feature 配置下**各执行一遍**（默认走 libudev 的枚举实现，`--no-default-features` 走 sysfs 的），两次都是 **25 passed / 0 skipped** |

| `cargo nextest run --package akasha-bw`（plan 0902） | 退出码 **0**：**47 passed / 0 failed**（39 单测 + 7 条假 `bw` 的集成用例 + 1 条 session key 那一页的保护读数） |
| `pnpm build`（前端，plan 0905） | 退出码 0；产物 **886.44 kB / gzip 243.89 kB**（Bitwarden 面板 +7.69 kB；CSS 14.96 kB 未变） |

| `pnpm build`（tsc + vite build） | 退出码 0；产物 **878.75 kB / gzip 241.58 kB**（**+0.28 kB / +0.14 kB**：plan 1103 的应用级通知行；CSS 14.96 kB） |
| `just docs-check` | 全部通过（语体零命中 / 命令与 `justfile` 同步 / ROADMAP **64** 个条目 ≤3 行且无代码块 / **55** 份 plan ≤200 行且索引一致）。**非快速失败已用负例验证**：一份含 25 处语体命中的文档与一份提到不存在配方的文档同时在场时，两项在**同一轮**里分别报出（前者列出 20 条并写明"另有 5 条"），退出码 1 |
| `ast-grep scan` + `ast-grep test` | 均退出 **0**（plan 0704 未新增 / 修改规则） |
| **三条 unsafe 注释 lint**（clippy，位于 `just lint`） | 退出码 **0**；三条各以一个探针验证其**确实会失败**（探针用后即撤） |
| `cargo tree -p akasha-core \| grep -c tauri` | **0**（分层成立） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数据，0704 未重新运行**） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉取 RustSec 数据库。

## 待验证（本地或沙箱环境无法执行）

- **串口的真实硬件一次都没有被碰过**（plan 0801 / 0802 / 1101 / 1102 / 1103）：本机 `/dev` 下
  没有任何串口设备节点（libudev 那套枚举仍列出 32 条 `/dev/ttyS*`，它们**全部打不开** ——
  问题 #150），所以"枚举出**可用**的端口"与"与真实设备互操作"都没有本地证据：三条 E2E 的设备都是
  测试进程自己造的 PTY 从端，字节确实经内核走了一个来回（两个方向各是独立证据），但真设备上的
  USB 串口芯片、模数转换与流控引脚 PTY 一样都没有。⚠️ 与这条替代口径有关的四条边界：
  **数据位与校验位被内核归一化**（请求 7 位 / 偶校验 → 回读 8 位 / 不校验），那两项只到"映射是全的"；
  `serial_ports_ui` 里"点一条枚举结果 → 路径栏被填上"在枚举为空的机器上**显式跳过并写明原因**
  （CI 的 runner 就是这一类），"据此打开"因此没有跨机器的证据；实测的"设备消失"是**关闭 PTY 主端**
  （`POLLHUP` → `BrokenPipe`），真 USB 转串口被拔掉时内核报 EIO（`wait_fd` 的另一支）——
  两条都进 `Err`、不会静默，但 **EIO 那一支没有实测**；`portable-pty` 的从端不会被别的进程
  按独占打开，而真设备上 `serialport` 的 `TIOCEXCL` 会让**第二个**会话拿到 `EBUSY`。
  ⚠️ 反向的一条：**PTY 不是串口参数的忠实回读装置**，把它当成全部五个参数的证据会得到一份
  看起来完整、实际只覆盖三项的读数。
  ⚠️ **macOS 上连"假设备"都造不出来**：同一个 PTY 从端在那边打开会得到 `Not a typewriter`
  （CI 第三次运行实测），所以那三条 E2E 在 macOS 上按平台**显式跳过**并写明原因
  （`fake_serial_skip_reason`）；Windows 更早一步 —— ConPTY 没有设备节点。
  ⚠️ **Windows 上另有两条 E2E 的路走通了，但设备那一层仍然缺席**（plan 0803）：`windows_ports`
  不造假设备，它验的是**这台机器上真实存在的东西**（注册表里的 PnP 设备项）—— 那是首条在
  Windows 上真的会执行的串口用例。**本机没有串口设备**（`SERIALCOMM` 键不存在），
  所以它执行到的分支是空表；"枚举出**真的**带描述的端口"要一台有设备的 Windows 主机。
  ⚠️ 本会话把原因追到了上游那一行：`serialport` 在 Apple 目标上**用 `IOSSIOSPEED` 设波特率**，
  它对 pty 返回 `ENOTTY`（`serialport-4.10.1/src/posix/termios.rs` 自己写着这一条），
  于是"打开"这一步就失败、断言没有机会执行。**同一条限制现在也门控了单元与集成测试**：
  `akasha-serial` 的 `a_pseudo_terminal_reports_the_parameters_it_can_hold`、
  `an_unplugged_device_says_why_the_stream_ended` 与整份 `tests/pty_roundtrip.rs` 都标了
  `#[cfg(target_os = "linux")]` 并写明理由 —— 参数映射在 macOS 上仍由 `settings.rs` 的单测守着。
  ⚠️ **`AGENTS.md` §7 里 `introspect { action: "processes" }` 那一条对串口没有对象**
  （`session_leader()` 是 `None`，crate 也不 spawn 进程）："真正退出之后零残留"在这条路上只剩
  注册表（`live` / `registered` 回到打开前的读数，实测 `1/1`）—— 它与"没有任何进程/线程留下"
  **不是同一件**：后者由**结构**保证（设备句柄 + 合批线程随流结束而退出），没有单独的读数口
  （`residue` 探针报的是 SSH 连接与隧道看护任务）。
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
  路径，所以"同网段的人经它访问远端网络"这一风险**从未被构造过**
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
- **Windows 目标的类型检查只覆盖到三个成员**（plan 0108）：`akasha-core` / `akasha-pty` /
  `akasha-serial` 在非 Windows 主机上能核对；`akasha-store` / `akasha-ssh` / `akasha-bw` /
  `akasha` 因为 vendored OpenSSL 与 `ring` 的 C 构建脚本在 check 阶段就失败（本机没有 MSVC 工具链），
  这四个成员只能由 CI 的 Windows 格子给出结论 —— 首次运行已由推送触发，结论待读。
- **权限位、单实例、托盘在非 Linux 平台的覆盖已由 CI 的 E2E 补上，但各留一档**（2026-09-27 订正；
  原文写的是"未验证，结论待读"，那两次运行之后 macOS 与 Windows 的 E2E 都已经实际执行并转绿）：
  **权限位**在 macOS 上退化为"直接断言库文件是 0600"（`sqlcipher_contract`，因 SQLite 在那边建库本就 0600，
  没有"前后对比"可用）；**单实例**在 macOS 上第 1–4 层仍然执行，第 5 层（数 `/proc` 里的实例）显式跳过；
  **托盘**在 macOS 的 CI 上**真的建成了**（日志里那条断言执行过、报"托盘菜单项数=1"），
  但"菜单项可点"与"从托盘退出零残留"仍只有 Linux 读数。⚠️ macOS 的**单元测试**不在 CI 里
  （`checks-macos` 只执行 `just check`，E2E 作业只多执行 `serial-unit`）—— 它的单测读数只有本机那一次。
- **macOS 上串口没有任何运行期读数**（2026-09-27 复核）：枚举与波特率全委托上游（IOKit / `IOSSIOSPEED`），
  本仓零 ioctl；三条串口 E2E 在 macOS 上整条跳过，`pty_roundtrip` 与两条 `serial::transport` 用例
  门控在 Linux —— 见问题 #158。
- **macOS 上打包成 dmg / `.app` 之后的一切行为未验证**（2026-09-27 复核）：CI 不出包
  （`AGENTS.md` §12），`tauri.conf.json` 也没有 `bundle.macOS` 段。可搬迁性因此**不适用于 macOS**
  （dmg 安装、数据在 OS 标准目录）—— 口径由 plan 0408 收口。
- **前端类型检查不在任何门禁内**：`just ready` 只覆盖 Rust 与文档，`pnpm build`（tsc）需手动运行。
- **`just dev-web` 的模拟后端未在真实浏览器中操作过**：SSH 两条命令在其中**显式报错**
  （"没有 SSH 客户端"），因此主机选择器在浏览器中只会显示该提示。

- **登录 / 解锁的"成功"路径没有实测**（plan 0902 / 0905）：需要一个真实 vault。本机实测到的是
  `bw login <邮箱> --passwordenv … --raw --nointeraction` 确实把请求发去了服务器（对着本地
  HTTPS 桩发出了 `GET /api/config` 与 `POST /identity/accounts/prelogin/password`），而
  **口令正确之后那一段（解密用户密钥、拿到 session key）一次都没有运行过**。
  桩服务器要造出这一段就得自己实现 Bitwarden 的密钥派生与加密 —— 那正是 `scope.md` §10 的非目标。
- **`--nointeraction` 与 `--passwordenv` 的组合没有在真实 vault 上验过**：两个都取自官方文档与
  `bw --help`，但"两个一起用能不能解锁"只有真 vault 能答（`docs/bitwarden.md` §7）。
- **登录之后的 `bw status` 形状只有文档可依**：文档给了五个键，本机只见过未登录那三个
  （`userEmail` / `userId` 因此按可选解析）。
- **自签证书那条路没有接进界面**：crate 有 `with_extra_ca`（`NODE_EXTRA_CA_CERTS`，实测对本地桩
  有效），但界面上没有"指出 CA 证书路径"这一栏 —— 自托管用自签证书时只能靠系统信任库。
- **本沙箱里驱动不了"真实 app + Victauri MCP"那条路**：每次 bash 调用都是独立的 bwrap
  （问题 #33），app 把发现目录写在它**自己那次调用的私有 `/tmp`** 里，而 MCP bridge 看不到它
  —— 所以 plan 0902 的"真实路径"用一次性探针代替（见「已验证为通过」那一行），
  而 app 那一侧由 `just test-e2e` 里的 `bitwarden_login` 覆盖。两者合起来是"下载走真的、
  接线走真的"，但它们**不是同一次运行**。
- **`config.json` 的写路径是这一条新加的**（plan 0902）：`bitwarden` 那段由本程序写，
  而 `close_behavior` 仍只读；既有文件坏掉时**拒绝写**（免得把用户写的值一起抹掉）——
  这条判据有单测，但"真机上写一次再重启"还没有走过。
- **导入的形状来自上游实现，不是来自一次真实输出**（plan 0903）：`bw list items --raw` 的字段
  形状、`type = 5`、以及"空私钥的三字段缺一即抛"都读自本机那份 `@bitwarden/cli` 2026.2.0 的
  构建产物（`docs/bitwarden.md` §4.1 逐条写了出处），并**没有**在真实 vault 上看到过一次输出。
  两者在真实 vault 上执行一遍要看什么，见 plan 0901；**"实现里这么写"不许说成"实测"**。
- **`Vault is locked.` 只有上游实现可依**（plan 0903）：本机没有可解锁的 vault，
  所以那一档（`BwError::Locked`）的判据是"上游 `errorIfLocked` 会给这句 + 它写 stderr"
  两件事分开记；`bw list items --raw` 的"未登录"那一档则是**实测**（退出码 1、stderr）。
  ⚠️ 本轮**没能**在运行时下载的那一份（`cli-v2026.8.0`）上复核：下载在本沙箱里超时，
  所以这条证据明确限定在本机 npm 那份 2026.2.0 上。
- **导入的钥匙"接到主机上"只有一条规则**（plan 0903）：`IdentityFile` 的 basename 与池里的
  钥匙名**逐字符相同**。因此下面两种情形仍然没有路，如实记着：上游条目名与配置文件里的
  文件名不同名（用户得自己改名）、以及没有 `~/.ssh/config` 条目可依时（参数要一格一格填的
  主机编辑界面还没做，见「主机池的增删改查仍无界面」）。

## 当前基线（2026-09-15 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` / `akasha-store` / `akasha-ssh` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，问题 #29） |
| 后端模块 | **`bitwarden`（两个轴 + **十一条命令** + `bitwarden` probe + **导入**）** / `bindings` / `session` / `tray` / `config` / `lifecycle` / `single_instance` / `vault` / `watchdog` / `ssh`（长住状态 + 那条命令 + 跳板链 + 库内 known_hosts 适配器） / `prompt`（提问往返） / `pools`（池的读取 + **导入** + **转发规则**） / **`tunnel`（隧道实体 + 三条命令 + `tunnel_state` 事件 + `tunnels` probe）** / **`sftp`（SFTP 实体 + 两侧 + 九条命令 + `sftp` 探针）** |
| **命令清单** | `greet` · `vault_status` / `vault_unlock` / `vault_lock` · `vault_hosts` · **`vault_forwards`** · `import_ssh_config` · `open_session` · `open_ssh_session` · `write_session` / `resize_session` / `close_session` · `ssh_prompt_credential` / `ssh_prompt_host_key` / `ssh_prompt_cancel` · **`tunnel_open` / `tunnel_retry` / `tunnel_stop`** · **`sftp_open` / `sftp_connect` / `sftp_list` / `sftp_transfer` / `sftp_transfer_cancel` / `sftp_transfers` / `sftp_sides` / `sftp_sessions` / `sftp_close`** · **`bw_cli_status` / `bw_cli_settings` / `bw_cli_install` / `bw_status` / `bw_server_set` / `bw_login` / `bw_unlock` / `bw_lock` / `bw_logout` / `bw_sync` / **`bw_import_keys`**（plan 0902 / 0905 / 0903，零个事件；只有 `bw_cli_install` 是 async）**（事件：`session_ended` · `ssh_prompt` / `ssh_prompt_dismissed` · **`tunnel_state`**）。**plan 0601 新增四条命令**（三条驱动隧道 + 一条只读规则池）；`tunnel_open` / `tunnel_retry` 是 **async**（命令体里有一次会阻塞几秒的握手）。⚠️ **plan 0602 / 0603 / 0604 / 0605 都没有新增命令与事件**（三条转发走的是同样那三条命令 + 同一份事件 + 同一份 probe），只改了它们的内部与失败分档；plan 0605 新增的是一份配置（`Config::reconnect`）与一条 `tray` probe；**plan 0606 同样没有新增命令与事件**，只加了一条只读探针 `residue`（见下）。⚠️ **plan 0701 新增六条命令、零个事件**（`sftp_*` 四条驱动 + 两条只读）：SFTP 的连接结果由**命令返回**（失败落在那一侧），诊断走 `sftp` 探针 —— 不给它加事件是因为"两侧各自的状态"本来就只在用户按下连接之后才变。⚠️ **plan 0702 又加三条**（一条起传输 + 一条取消 + 一条只读），**仍然零个事件**：传输的进度与结局由 `sftp_transfers` 与探针读（`sftp_transfer` 同步：它排完任务就返回）。`sftp_close` 因此变成 **async**（它要等清理落地）；`sftp_connect` 的第二个参数由 `hostId` 变成 `origin`。⚠️ **plan 0703 一条命令、一个事件都没加**：两档是"同一对端点命令、目标那个端点怎么来的"，所以变的只有三个字段（`through` / `throughFailure` / `via`）与删掉一档错误。⚠️ **plan 0704 同样一条命令、一个事件都没加**：并发上限是引擎内部的策略，契约只多了 `SftpSummary.inFlight.{limit, live, peak}` 这一组读数（`sftp_open` 多收一个 `AppHandle`，签名不变） |
| **probe** | `lifecycle` → `{close_behavior, tray_ready, close_action}`（没登记时 `{"initialized":false}`，问题 #93）；`single_instance` → `{registered, activations}`；`sessions` → `{live, registered}`（SSH 与隧道都没有本地进程，"零残留"只能看注册表）；**`tunnels` → `[{handle, ruleId, name, state, attempt, bind}]`**（与托盘菜单同一份数据；`bind` = 实际监听地址 —— plan 0604 起对 `remote` 规则它说的是**服务端**那一侧的地址，`port = 0` 时是服务端挑的那个）；**`tray` → `{ready, tunnels:[…]}`**（plan 0605：`tunnels` 是**托盘菜单上那几行文字**，与菜单共用 `tunnel_labels`；`ready` = 这台机器上托盘建成没有）；**`residue` → `{sshConnections, watchTasks}`**（plan 0606：`SshConnection` 的存活计数与看护任务的存活计数，判据"两个计数都归零"的读数口 —— 数的是**资源本身**，不是注册表里的实体）；**`bitwarden` → `{cli:{binary, appdata, program, version, variant, licenseNotice, problem}, status, hasSession, problem}`**（plan 0902：两个轴解析出来是什么、CLI 自报的版本与变体、三态、**我们手里有没有 session key** —— 它**不起进程**，报的是上一次动作留下的读数）· **`sftp` → `[{handle, sides:[{side, origin, name, state, failure, path, through, throughFailure}], transfers:[{id, from, fromPath, to, toPath, state, done, total, failure, via}]}]`**[{side, origin, name, state, failure, path, through, throughFailure}], transfers:[{id, from, fromPath, to, toPath, state, done, total, failure, via}]}]`**（plan 0701：一个 SFTP 会话一条记录，两侧的状态、失败原因与当前目录都在里面；plan 0702 起每条还带上**这个会话发起过的传输**与它们的进度 / 结局；plan 0703 起两侧还带上**到达方式**（`through` = 经哪台直通、`throughFailure` = 回退原因），传输带上 `via`；plan 0704 起每个会话还带一组并发读数 `inFlight:{limit, live, peak}` —— 判据"两侧各自列目录成功"、"中断之后没有半成品"、"这次走的是哪一档"与"上限真的在起作用"的读数口）。⚠️ **排队中的传输在传输记录里与"正在搬"长得一样**（状态只有"还没结束"这一档），分辨它们靠 `live` 比"还没结束的条数"少。**库没有 probe**：状态本身就是命令（`vault_status`） |
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
| **前端结构** | `src/ipc/`（`session.ts` / `prompts.ts` / `hosts.ts` / **`tunnels.ts`** / **`sftp.ts`** —— 唯一允许碰后端的目录）、`src/tabs/`、`src/terminal/`、`src/ssh/`（主机选择器 + 导入面板 + 提示面板）、**`src/tunnels/`（隧道面板）**、**`src/sftp/`（SFTP 双栏面板：一栏可以选本机，带传输列表与取消）**、**`src/bitwarden/`（Bitwarden 面板：两个轴 + 服务器 + 登录 / 解锁 / 锁定 + 只读导入）**、`src/App.tsx`。标签页 `kind`：`terminal` / `ssh`（**都有关闭按钮**，规则写成 `CLOSABLE` 清单）—— **隧道与 SFTP 都不是标签页**：它们是应用级浮层，关面板不停任何会话 |
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

- [ ] **下一步 = 阶段 9 只剩最后一条**：[`0901`](./plans/0901-bw-noninteractive-probe.md) 的**真实输出**
  （门槛只有一个：**需要一个真实 vault**）。变体判定与"未登录时的报错"两项已完成
  （`bitwarden.md` §2.2 / §7.2），条目形状读自上游实现（§4.1）—— 差的是"在真 vault 上看一眼"。
  自托管实例的地址由用户提供；主密码不经过本仓库。
- [ ] **自签证书那一栏还没接进界面**：crate 的 `with_extra_ca`（`NODE_EXTRA_CA_CERTS`）已就位、
  也在本地桩上实测有效，但设置里没有"CA 证书路径"这一项，所以自托管的服务器目前只能靠
  系统信任库。这是计划级的活（要连配置文件格式一起定）。
- [ ] **`config.json` 的写路径**（plan 0902 新增）在真机上的"写一次再重启"还没走过：
  单测覆盖了"坏文件拒绝写"与"只换 `bitwarden` 那一段"，而 E2E 每次运行都会重写它并读回
  （配置由 app 启动时读）—— 但"重启之后两个轴仍是上次选的那一个"这条**没有专门的用例**。
- [ ] **Windows 目标的编译被 `akasha-pty` 挡住**（问题 #149）：修它是计划级的活
  （Windows 上没有 POSIX 进程组语义）。它同时压着阶段 8 那半句"Windows / macOS 原生编译"。
- [~] **plan 0803（Windows 上的串口枚举与它的证据）**：`PortKind` 新增 `Windows` 一档，
      取值来自设备项的 `FriendlyName` / `HardwareID`（`SERIALCOMM` 给端口集合、设备项树给描述，
      **两边用 COM 名对齐**）；`SerialPortKind` 与前端同步。本机**第一次**把 Windows 目标构建
      通过（`cargo check --lib --target x86_64-pc-windows-msvc` 退出码 0、无警告；MSVC 与 SDK
      本机都在，只有 vendored OpenSSL 需要一个原生 perl —— 问题 #156）；`serial::` **30 条**、
      workspace `--lib` **218 条**全过，其中两条在 Windows 上真的走了一遍注册表那条路。
      新增 E2E 目标 `windows_ports`：它**不造假设备**，因此不被 `fake_serial_skip_reason` 跳过 ——
      那三条串口 E2E 在 Windows 上仍然全跳（ConPTY 没有设备节点）。新增配方 `just serial-unit`。
      ⚠️ 判据在本机只走通一半：本机**没有任何串口设备**（`SERIALCOMM` 键不存在），执行到的分支
      是**空表**；"枚举出真的带描述的端口"要一台有设备的 Windows 主机。另：负例证明
      "让描述在 Windows 上静默失效"**编译期拦不住**，只有运行能发现 —— 那正是本目标存在的理由。
      `just ready` 的 `lint` 那一关现在已经过了：它此前红在两处 SSH 测试的
      `clippy::result_large_err` 上（与串口改动无关），已按问题 #166 修掉。本机继续往下走曾停在
      `test`，那几处也已修掉（问题 #167 / #168）—— 现在本机
      `cargo nextest run --workspace --no-fail-fast` 是 **423 条全过**。
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
- [~] **plan 0108（Windows 目标的类型检查）**：`akasha-pty` 的 `rustix::process` 已按平台门控，
      三个能本地核对的成员在 Windows 目标上退出码 0；第三次运行又暴露同一类的第二处
      （测试脚手架的 `tty_name`，问题 #160），已修、**判据要等下一次运行**。
      ⚠️ 它只解决**编译**这一面
- [ ] **Windows 上的会话级回收仍是空的**（plan 0108 留下的缺口）：POSIX 的会话 / 进程组在 Windows 上
      不存在，等价物是 Job Object（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`：句柄一关，作业里的进程
      全部结束），它还能替掉伴生看门狗在那条平台上的路径。**本机现在有 Windows 主机与读数**（见
      「已验证为通过」）：注册表那一半是对的 —— 关标签页 / 关窗 / 真退出之后 `live` 与 `registered`
      都回到基线（`ssh_session` / `ssh_jump` / `ssh_config_import` 等的收尾断言都过）；缺的是"会话里
      的其它进程"：`tab_close` / `window_close` 的探针是一个**忽略 SIGHUP 的后台作业**，cmd.exe 写
      不出它、收它也要靠作业对象，于是这两条目标在 Windows 上显式跳过（#176）。
      展开时先写 ADR（进程模型，与 ADR-0005 同源）
- [~] **plan 0102（CI 平台矩阵）**：三次运行把红灯逐层换成了真实缺陷（#154 → #155 / #156 / #157
      → #158 / #159 / #160），每一处都已处置。现状：**Linux 的完整门禁与 Linux 的 E2E 通过**，
      macOS 的类型检查通过；Windows 的类型检查与 macOS 的 E2E 待下一次运行验证，
      Windows 的 E2E 已修掉 #162 的两处配方缺陷（`$!` 的 PID 命名空间、运行中覆盖 exe），
      本机首次完整执行（见「已验证为通过」）
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：CI 上三个平台都真的执行起来了 ——
      ubuntu 格**通过**（28 个目标全部执行完，xvfb 下的原生窗口句柄路径第一次走通），macOS 格**全绿**，
      Windows 格在本机 `just test-e2e` **退出码 0**、28 个目标全部通过（见「已验证为通过」，
      跳过的四类目标与原因见 #176）；**CI 的 Windows runner 上也通过了**（run `36231407751`，见「已验证为通过」）
- [ ] **正式 UI**：等待设计稿（见上文「UI 现状」）—— 没有验收标准，因此**不进入 ROADMAP**

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接运行 `cargo …` 会失败（问题 #8），一律通过 `just` 转发。⚠️ **临时脚本同样适用**。
- **七个 crate 的职责**：`akasha-bw`（Bitwarden CLI 客户端：两个轴、运行时下载、调用与解析、session key 的受保护页；**零 Tauri 依赖**）、`akasha-core`（Session 模型 + 配置模型与判据 + **隧道状态机**，
  **零 Tauri 依赖**）、
  `akasha-pty`（`Transport` + portable-pty + 合批 + `teardown` + `watchdog`）、
  `akasha-serial`（串口：`Transport` 的实现 + `ports()` 枚举 + 参数校验 + **设备消失时的原因**；`serialport` 的 `libudev` 只在 Linux；零 Tauri 依赖）、
  `akasha-store`（库的打开 / 创建 / **格式版本与迁移** / 四类池（含 `jump_chain`）/
  **known_hosts 缓存** / dump / 导出与还原 —— 唯一允许 `unsafe` 的 crate）、
  `akasha-ssh`（连接 + 认证 + known_hosts 三态 + **D9 原语与跳板** + 同步 `Transport` 门面 +
  `testing`（进程内服务端））、
  `akasha`（app 包 = IPC 薄壳 + 托盘 + 配置 + 数据目录 + 窗口关闭语义 + 单实例 + 退出钩子 +
  看门狗接线 + 库的解锁状态 + SSH 的 runtime / 提问往返 / 池读取 / **跳板链** + 代码生成 bin）。
- **前端**：`src/ipc/`（唯一允许调用后端的目录，含 `bitwarden.ts`）、`src/tabs/`、`src/terminal/`、`src/ssh/`、
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
- **改 serial 之前先看**：`akasha-serial/src/enumerate.rs`（⚠️ 空表是正常结果、列出来的端口
  **不保证能打开** —— 问题 #150）→ `transport.rs`（打开、读循环、**设备消失时那句话**）→
  `settings.rs` / `error.rs`（取值域与四组失败）→ **app 侧的接线** `src-tauri/src/serial.rs`
  （三条命令 + 三个 IPC 枚举的映射 + `SerialIpcError` 的四档）→ `src/serial/SerialPicker.tsx`。
  读数口：`just serial-check`（两种 feature 配置各执行一遍）与 `just libudev-check`；
  会话侧只有 `app_state { probe: "sessions" }`（串口没有本地进程）。
  排查"串口标签页为何消失"：日志里 `session retired` 带 `err=串口设备已断开…`（没有退出码的载体
  走这一支；有结局的载体报 `exit_code` / `signal`），界面上那句话在 `[data-session-notice]`。
  ⚠️ 打开是**独占**的（`TIOCEXCL`）：第二个会话会得到 `EBUSY` —— 不得为了绕开它去关独占。
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
14. **`docs-check` 的反向检查**已扩到 `AGENTS.md` / `README.md` / `ROADMAP.md` / `docs/**/*.md`
    （`CLAUDE.md` 除外，理由见问题 #153）。
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
149. **`akasha-pty` 曾用 Windows 上不存在的 `rustix::process`，且没有 `cfg` 守卫**
     （**编译面已处置**，plan 0108，2026-09-15）：上游把 `rustix::process` 限定在
     `#[cfg(not(windows))]`，而 `teardown.rs` / `watchdog.rs` 直接用它的 `Pid` / `Signal` /
     `kill_process` / `setsid` —— 于是 Windows 目标编译不过（`cargo check --target
     x86_64-pc-windows-msvc` 在 `akasha-pty` 就红，3 个 E0432 / E0433）。它长期没暴露的原因是
     CI 的 Windows 那一格（`checks-other`）在此之前没有执行过（首次运行已由推送触发，见 plan 0102）。
     **现在的边界**：编译不再是障碍，但 Windows 上"回收整个会话"**仍然是空的**（`kill_session`
     返回 0，`Child::kill()` 只收得走 shell 自身）—— 等价物是 Job Object，它要一台 Windows 主机
     才能验收，那条缺口记在「进行中 / 下一步」。⚠️ 阶段 8 那条判据（Windows / macOS 的原生编译）
     在本机仍然只有依赖图核对与不带 C 构建脚本的成员，完整证据在 CI 的 Windows 格子。
150. **枚举出来的端口不保证能打开**（plan 0802 实测）：本机（libudev 那套）列出 32 条
     `/dev/ttyS0`…`/dev/ttyS31`，而 `/dev` 下**一个都没有** —— 上游按 udev 设备给 devnode，
     不检查那个节点在 `/dev` 下是否存在；它那句"打不开就跳过"的过滤
     （`serialport` 的 `enumerate.rs`：parent 的驱动是 `serial8250` 且打不开时跳过）本机没有触发。
     同一处还有第二个读数：`--no-default-features`（sysfs 那套）在**同一台机器**上返回 0 条，
     因为它要求 `/dev/<名字>` 存在。教训：**"列出"不等于"能用"** —— 判据写成"枚举到的端口都能
     打开"会得到一条与真机相反的断言（本机恒假、接上设备恒真）；正确的位置是"枚举只负责列，
     打开失败带路径与原因"（`SerialError::Open`），而界面不要把列表当成"可用端口"。
151. **串口接入 app 在 ROADMAP 里没有条目**（plan 0802 收尾时发现，2026-09-15 由**阶段 11** 处置）：
     阶段 8 的两条都在 crate 层，`scope.md` §2 的"三大终端之一（serial）"当时没有用户可见的形态。
     编号不复用；教训：**一个阶段的判据全在 crate 层时，产品形态的那一半要有自己的条目**。
152. **把多类检查写成配方的多行 = 快速失败会掩盖其余结论**（本会话）：`just docs-check` 原先的第一行是
     `@just docs-style`，而 just 在配方里某一行失败时立即终止该配方 —— 于是语体一命中，
     命令未漂移 / ROADMAP 预算 / plan 预算三类检查**一次都没有执行**，输出里也看不出它们没有执行。
     这与 CI 矩阵用 `fail-fast: false` 是同一个理由：**互相独立的信号不串成一条链**。
     正解：四部分放进同一个 shell 块（变量才能跨检查累加），最后一次性汇总退出。
     教训：配方里"再调用另一个配方"的那一行，等于给整条链加了一个**早退点**。
153. **裸词正则会匹配英文散文**：反向命令检查若把 `CLAUDE.md` 算进去，Victauri 自动生成块里的英文句子
     （`just` 后面紧跟 `retry` 一类单词）会被读成配方名。它不在检查范围内不是因为那个文件不重要，
     而是因为裸词正则只对中文文档成立（口径写在 `docs/just.md` §2）。
154. **`rustc-wrapper` 让"没有 sccache 的环境一个文件都编译不了"**（CI 首次运行的唯一红因）：
     `.cargo/config.toml` 里 `rustc-wrapper = "sccache"`，而镜像上没有 sccache —— cargo 在**探测
     rustc** 时就失败，报错读起来像工具链坏了，与真正的编译错误不是同一种。两个可选处置：
     **装 sccache**（`taiki-e/install-action` 的 `TOOLS.md` 三平台都收录）或**去掉那层包装**。
     选后者的两条理由：`Swatinem/rust-cache` v2.7.8 的 README 逐项列出它缓存的目录，只有
     `~/.cargo` 与 `./target`，**不含** sccache 自己的缓存目录 —— 那层包装在 CI 上不可能命中；
     且空值即"没有包装"（本机 cargo 1.98.1 实测：把文件里的包装器换成不存在的那个，只要
     `RUSTC_WRAPPER` 为空就照样通过，证明空值真的覆盖了配置）。
    ⚠️ **2026-09-17 起 CI 那一侧只是兜底**：仓库里的包装器已整份删除（问题 #163），
    因为同一个包装器在 mise 的 cwd 解析下同样让依赖编不过 —— 两处处置至此收敛到同一条口径。
155. **Tauri 的官方依赖列表里没有 `libudev-dev`，而 `serialport` 需要它**（CI 第二次运行）：
     Linux 那一格红在 `just ready` → `lint` → `clippy`，报的是 `libudev-sys v0.1.4` 的构建脚本
     panic —— `pkg-config --libs --cflags libudev` 答 `Package 'libudev' … not found`。
     ⚠️ 缺这个**开发包**与"运行期没有 libudev"是两件事：后者降级成 sysfs 那套枚举（空表，不是
     错误 —— 见阶段 8 那一行），前者是**编译不过**。处置：加进 `env.APT_DEPS`。
156. **Windows 上 `perl` 解析到 Git 自带的 msys 版本，而 vendored OpenSSL 需要 Strawberry Perl**
     （CI 第二次运行）：`openssl-sys` 的构建脚本执行 OpenSSL 的 `Configure`，报
     `Can't locate Locale/Maketext/Simple.pm in @INC`，而 `@INC` 全是 `/usr/share/perl5/core_perl/...`
     —— 那是 Git for Windows 的 perl；镜像另装了 Strawberry Perl（chocolatey 的 `strawberryperl`），
     只是 PATH 里排在后面。处置：`OPENSSL_SRC_PERL` 指向 Strawberry 的 `perl.exe`
     （`.github/actions/windows-perl`，`checks-other` 与 `e2e` 共用）。⚠️ 写进 `GITHUB_ENV` 之前
     先探一次模块：失败因此停在**那一步**，而不是推迟到 cargo 的构建脚本里 —— 后者的报错读起来
     像 OpenSSL 坏了，与真正的原因不是同一种。
157. **`Swatinem/rust-cache` 默认在仓库根执行 `cargo metadata`，而本仓库的 workspace 在 `src-tauri/`**：
     每个执行到它的 job 都会在缓存步骤里打一串 `Error: The process … cargo … failed with exit code 101`
     与 `could not find Cargo.toml in …`（该步骤仍判**成功**，键也照常给出 —— 所以它不会挡住任何
     东西，只会让缓存范围无法从日志判断）。它是 ADR-0004 的直接后果：该 action 的 `workspaces`
     默认值是 `. -> target`。处置：三处都补 `workspaces: src-tauri`。
158. **用例中途失败会把标签页留在 app 上，后面的目标因此必红**（CI 第三次运行，macOS 的 E2E）：
     macOS 上三条串口目标打不开 PTY 从端（问题 #160 那一类平台限制），失败发生在**开标签页之后**
     —— 那三个标签页留在了 app 上，于是下一个目标 `bw_import` 的 `is_connected`（它要求标签页数
     **恰好**等于脚本里写的那个数）永远不成立，报出来的是"提示问答没走完就连不上"，而状态栏明明
     写着"已连接" —— 两句话看起来像产品缺陷，实际是上一个目标留下的污染。教训：**判据里数总量时，
     失败路径的残留就是它的污染源**；平台跳过要发生在**动界面之前**，而不是等失败了再补救。
159. **bash 会把紧跟在 `$变量` 后面的全角标点读进变量名**（CI 第三次运行，macOS 的 E2E 收尾）：
     macOS 的 bash 3.2 在那里把 `$l）` 解析成变量名 `l` + 半个多字节序列，`set -u` 于是报
     `l：unbound variable` 并以 **127** 退出 —— 而这一行只在**已经失败**时才执行，于是它把真正的
     失败换成了"命令找不到"。同一行在 Linux 的 bash 5 上照常工作，所以它只在 macOS 上暴露。
     处置：`$变量` 紧邻非 ASCII 字符时一律写花括号形式（本轮扫了全部 justfile 与测试脚手架，
     7 处一并改掉）。
160. **`portable-pty` 的 `tty_name` 只在 Unix 上有，而测试脚手架直接用了它**（CI 第三次运行，
     Windows 的类型检查）：`error[E0599]: no method named tty_name found for struct Box<(dyn
     MasterPty + Send + 'static)>` —— 测试要的是"从端的设备名"，而 ConPTY 没有设备节点这个概念。
     它与问题 #149 同类（平台专有 API 漏了 `cfg`），只是这一处落在**测试**里，而 `akasha` 带 C 依赖、
     本机无法为 Windows 目标构建 ⇒ 只有 CI 的 Windows 格子能给出读数。处置：`slave_device_name`
     按 `cfg(unix)` 分两条实现，三条串口 E2E 在非 Linux 平台上按 `fake_serial_skip_reason` 显式跳过。
161. **`main` 上的运行是排队等待，看起来像"卡住"**（本会话实测）：`concurrency` 的规则是
     "同一分支只保留最新一次运行，`main` 除外" —— 于是前一次还没结束时，后一次的状态**一直是**
     `pending`；一个分组里最多留一个排队中的运行，**取消正在执行的那次会把排在它后面的那次一起
     结束**（实测两次运行同时变成 `cancelled`）。教训：先分清"等待"与"卡住" —— 判据是
     `pending` 且前一次仍在 `in_progress`、以及各 job 的典型耗时，而不是运行时长本身。
     处置：workflow 补 `workflow_dispatch`（重新验证某次提交不必加空提交）、排队语义与耗时表
     写进 `docs/just.md` §8。
162. **Windows 上 `just test-e2e` 起不来 app，而那份 app 日志是空的**（CI 第三次运行，第一次读数）：
     那格在 **13 分 38 秒**的构建之后执行到"起 app"，120 秒内没有任何 app 登记进 discovery 目录，
     配方于是报 `❌ app 没起来`，而它 `tail` 的那份日志**一个字节都没有**；随后那一次运行被手动
     停止（收尾处的退出码记成 `0xC000013A`，那是控制台被中断的形状，不是 app 自己的退出码）。
     ⚠️ **没有定位**：本机没有 Windows 主机，"进程有没有起来、走到哪里才没有的"都观测不到。
     下一轮要抓的三样证据已经写进配方（日志为空时会明说、cargo 进程还在时会明说）：`cargo run`
     自己的退出码、`tasklist` 里有没有 `akasha.exe`、discovery 目录有没有出现过。
     在那之前 Windows 的 E2E 格子仍然是红的。
 163. **同一个 `rustc-wrapper` 在本机 macOS 上让每一个依赖都编不过**（本会话实测，macOS 26.6.2 /
      arm64）：`.cargo/config.toml` 把 rustc 包成 `sccache`，而本机的 sccache 由 mise 提供 ——
      mise 的 shim 按**当前目录**解析版本，cargo 编译 registry 里的依赖时 cwd 落在
      `~/.cargo/registry/src/<crate>-<版本>/`，那里解析不到版本，于是**每一条** rustc 调用都以
      `mise ERROR No version is set for shim: sccache` 失败，`just dev` / `just check` / `just test`
      一个文件都编不出来（报错读起来像编译器坏了）。**不经 cargo 也能复现**：
      `cd ~/.cargo/registry/src/index.crates.io-*/serde_core-* && sccache --version`。
      它与问题 #154 同因（配置里的包装器在"包装器不可用"的环境里一票否决），触发条件从
      "没装 sccache"变成"装了、但按 cwd 解析不到"。
      处置：**删除 `.cargo/config.toml`** —— 与 #154 里 CI 的选择（清空 `RUSTC_WRAPPER`）收敛到
      同一个口径：包装器不再进仓库配置；需要缓存时另行安装 sccache，再显式 `RUSTC_WRAPPER=sccache just check`。
      ⚠️ 沙箱里直接执行 `just dev` 另有一处环境限制（PTY 报权限不足），与本条无关；用户终端下正常。
 164. **macOS 上被 SIGKILL 的子进程会停在"正在退出"上，直到主端被关闭**（本会话实测，
      macOS 26.6.2 / arm64）：`kill_session` 发出 SIGKILL 之后，`/bin/sh` 在 `ps` 里显示
      `?Es`（`E` = trying to exit、`s` = session leader、命令名已带括号），而 `wait4` 一直
      不返回 —— 同一时刻 `kill(pid, 0)` 仍然成功。**这一步是关键**：`wait4` 阻塞的原因不是
      "信号没送到"（再发一次 SIGKILL 也无效），而是子进程的退出要等终端那一路收干净，而主端
      还开着、又没人读。处置：`PtyTransport::shutdown` 在 `wait` **之前**主动关闭写端 / 读端 /
      主端（`master` 因此变成 `Option`）。⚠️ 这不是清理动作而是**结束条件** ——
      少了它，关标签页会无限阻塞（`just test` 里两条最普通的 shutdown 用例就是这么红的）。
      判别口径：Linux 上同样的顺序不出这个问题，所以它只在 macOS 的读数上现形。
 176. **Windows 上仍有两类 E2E 目标被显式跳过，各自缺的东西不同**（本会话实测）：修掉 #162 / #173 / #174
      与上面那两条平台差异之后，剩下的红灯一律改成显式跳过并在日志里写明原因（`AGENTS.md` §7）——
      - `tab_close` / `window_close`：探针是一个 **POSIX shell 程序**（`sh -c 'trap "" HUP; …' &`），
        Windows 的默认 shell 是 cmd.exe，写不出"忽略 SIGHUP 的后台作业"；而"关标签页 / 关窗把它一起收走"
        在那边要靠**作业对象（Job Object）**，尚未实现（plan 0108 留下的缺口）。
      - `bitwarden_login` / `bw_import`：假 `bw` 是一份 `#!/bin/sh` 脚本，Windows 上 `CreateProcess` 不执行
        脚本 —— 需要一个真的 `.exe`，本仓库还没有（`install_fake_bw` 在那边写出来的文件名已经是 `bw.exe`）。
      ⚠️ 这两类都**不是"执行不了"，是判据本身在那边还不成立**；显式跳过只是把这件事说清楚，缺口仍在。

 181. **macOS 上"真正退出零残留"的判据是空过的，而状态文件把它读成了通过**（2026-09-27 覆盖审计
      发现，**未修**，plan 0112）：`tests/exit_residue.rs:92` 的 `alive()` 读 `/proc/<pid>` 且
      **没有平台门控** —— macOS 上没有 `/proc`，它恒为 `false`，于是"关窗后 app 真的退出"那条断言
      立刻为真；`:189` 又在非 Linux 不启动探针。CI 的 `E2E（macOS）` 日志里这一段
      `1 passed ... finished in 0.22s`（真的走完那条路径要 30 s 级窗口）——**这就是空过的形状**。
      同一类还有 `tests/session_watchdog.rs:170`：跳过理由写"macOS 要 `proc_listpids`"，
      而 `pty/teardown.rs` 的 `ps` + `getsid` 实现早已落地（那条理由已过期，macOS 上因此少一条
      端到端判据）。⚠️ 这类失效**不会变红**，只会让"验过了"变成一句没有证据的话 —— 处置与负例见 plan 0112。

 182. **macOS 的便携数据目录：文档写的是 `.app` 旁边，实现落在 `.app` 内部**（2026-09-27 覆盖审计
      发现，**按口径变更收口**，plan 0408）：`docs/portable.md:32` 与 `docs/scope.md:583` 承诺
      macOS 的数据目录在 `.app` **旁边**，而 `config/ipc.rs` 的 `exe_dir()` 取
      `current_exe().parent()` —— 打包之后那是 `Foo.app/Contents/MacOS`，标记目录放不进去
      （不可写、且破坏签名），放到 `.app` 旁边又推导不到。`tests/portable.rs` 用的是**裸二进制**
      复制进临时目录的布局，所以这条差异在 CI 全绿的情况下也不可见。
      口径裁定（用户）：**macOS 不做便携**（安装形态是 dmg、数据取 OS 标准目录），
      便携只保留 Windows / Linux；Linux 的分发形态**暂定**（可能改用 AppImage，
      其可执行文件位于只读挂载点内，判据的等价物尚未定）。
