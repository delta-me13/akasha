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

**2026-09-27：CI run #18（`83cf941`）—— 五绿一红，红因已修** —— 把 `staging` 合入 `main` 推上去之后，
六个 job 里 `检查（Linux / Windows / macOS）`、`E2E（Linux）`、`E2E（Windows）` 全过，只有
**`E2E（macOS）`** 红：失败落在第三段 `portable` 的 `without_a_portable_dir_it_starts_anyway`，
报 `initialize returned 401 Unauthorized`。根因是**测试脚手架的就绪判据**：`tests/portable.rs` 的
`discovery()` 只要 `port` 文件出现就返回，而 Victauri 的 `token` 是后落盘的 —— 客户端在两者之间
连上去就没有令牌（本机三次运行都没撞上，CI 上撞上了）。处置：**两个文件都读出来才算就绪**，
并给 `App::client()` 加有界重试（发现目录出现之后、MCP 真正 accept 之前还有一个很短的窗口）。
本机重新执行第三段：`4 passed; 0 failed`（6.00 s）。

**2026-09-27：macOS 上从 Dock 唤回窗口（plan 0307）** —— 窗口收进托盘（隐藏）之后，点 Dock 图标
此前**没有任何行为**（全仓只处理 `RunEvent::Exit` / `Ready`，没有 `Reopen` 的处理点）。本轮在
`app.run` 的闭包里加 macOS 分支：记一条 `window reopen requested`（带 `has_visible_windows`）后调
`single_instance::activate` —— 与"第二个实例敲门"共用**同一份**唤回实现（还原 / 显示 / 置前），
不新写一条路径。实测（dev 构建，托盘在这台机器上真的建成）：隐藏 → 点 Dock → `visible=true` 且
`focused=true`，屏幕上的锚点与隐藏前那条提示行的时间戳都还在（同一个终端缓冲），
`live=registered=1`（没有重启、没有新实例）；日志给出两种读数（隐藏时
`has_visible_windows=false`，已显示时 `true`）。⚠️ 点 Dock 图标**没有可编程的等价物**：
这一条验收里有一次人工动作。

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
| **CI run #18（`83cf941`，GitHub `delta-me13/akasha`）** | 六格：`检查（Linux / Windows / macOS）` + `E2E（Linux）` + `E2E（Windows）` **全过**；`E2E（macOS）` **红** —— 失败在第三段 `portable` 的 `without_a_portable_dir_it_starts_anyway`（`initialize returned 401 Unauthorized`），根因与修复见本轮摘要 |
| **plan 0307 的 Dock 唤回（macOS，dev 构建 + Victauri 无会话 REST 接口）** | `lifecycle` = `{close_action:"hide", tray_ready:true}`；隐藏后 `visible=false` → 点 Dock → `visible=true` / `focused=true`；`screenText` 里隐藏前的提示行时间戳与锚点都在（同一个终端缓冲）；`sessions` = `live=1 registered=1`；日志 `window reopen requested has_visible_windows=false` 与 `…=true`。`just ready` 6/6 |
| **`just test-e2e`（macOS，经 runner）—— plan 0507 的新目标** | `known_hosts_manage` **1 passed; 0 failed**（2.37 s）：第一次连接 `["hostKey:…", "credential:e2e@127.0.0.1:57633"]` → 面板 `行数=1`（名字来自主机池、指纹逐字等于服务端）→ 删除后 `行数=0` 且库侧无那条记录 → 第二次连接 `["hostKey:…"]`（**重新询问**成立，口令走内存缓存）。⚠️ 该次运行整体退出码 1，原因见问题 #183 |
| **`just portable`（macOS，`just test-e2e` 第三段）—— plan 0408** | `4 passed; 0 failed`（6.09 s）：`a_portable_dir_next_to_the_binary_is_adopted` 与 `without_a_portable_dir_it_starts_anyway` 通过；`data_survives_the_move` / `an_unwritable_portable_dir_refuses_to_start` 各打印一行「跳过: macOS 不做便携（安装形态是 dmg / .app，数据取 OS 标准目录，见 docs/portable.md §3）」。⚠️ 该次运行整体退出码 1，原因见问题 #183 |
| **`just test`（macOS 26.6.2 / arm64，经 `just runner-run test` 在沙箱外执行）** | 退出码 **0**：**463 tests run: 463 passed, 0 skipped**（333.4 s）；`akasha::session_watchdog a_sigkill_of_the_app_leaves_no_child_behind` **执行并通过**（3.18 s）—— plan 0112 之前它在 macOS 上直接 `Skipping:`。同一轮的 `just ready` **6/6**（fmt 1s · lint 21s · test 141s · deny 1s · gen-types 33s · docs 3s） |
| **`just test-e2e`（macOS，经 runner）—— plan 0112 的判据部分** | 第二段 `exit_residue` 真的走完（0.28 s：探针 `Some(pid)` 真的启动、判活真的为真、关窗之后真的消失）；第一段 `tab_close` / `window_close` 的进程级断言同样是真的。⚠️ 该次运行**整体退出码 1**：`ssh_session` / `ssh_config_import` 的回声判据超时（问题 #183，原始工作区同样复现），其余目标与第三段 `portable` 3/3 照常通过 |
| **负例（plan 0112 步骤 5：`pty/teardown.rs` 的非 Linux 分支临时改成 `let _ = leader; 0`）** | `just test`（临时给配方加 `--no-fail-fast`）：`463 tests run: 457 passed, 6 failed`，含 `akasha::session_watchdog a_sigkill_of_the_app_leaves_no_child_behind` **FAIL**（20.1 s）与 5 条 `pty` 用例；`just test-e2e`：`exit_residue` 报「退出后仍有残留：忽略 SIGHUP 的 74434 还活着」（15.3 s），`tab_close` 报「关闭标签页之后探针 A(72845) 还活着」。还原之后三条都转绿 |
| **`just ready`（合并后，本机 Windows 11 / MSVC）** | **6/6**：`fmt-check` 1s · `lint` 3s · `test` **18s**（前一轮同配方 139s，差别只在缓存）· `deny-offline` 3s · `gen-types-check` 28s · `docs-check` 40s。⚠️ 在此之前同一个配方在本机红了三轮，三次都停在那条已知偶发（`bw::acquire` 的 install 用例，连续执行 10 次里 7 次红）—— 定位与处置见问题 #165，修后复测 10 次全过 |
| **`just test-e2e`（合并后，本机）** | 退出码 **0**：第一段 **28 个目标 / 33 条用例**、第二段 `exit_residue` **1/1**、第三段 `portable` **3/3**；跳过的目标与原因不变（见 #176）。`tunnel_reconnect` **27.69 s** 通过 —— #179 是在 macOS 的 CI 上红的，本机一直过，处置是否消掉那次红要等 CI |
| **CI 运行 `36249980440`（@ `bda9324`，把分支合入 `main` 之后）** | **六格全绿**：`检查（Linux / Windows / macOS）` 与三个平台的 E2E 全部通过。上一轮的两处红都消掉了：**Linux 的 `vault_unlock`** 报出三次真实读数（`VmLck_解锁前=0 VmLck_解锁中=160 VmLck_锁定后=0`，问题 #180）、**macOS 的 `tunnel_reconnect`** 通过（问题 #179）。Windows 的 E2E 日志 **180,895 字节 / 2,230 行**、`LNK4099` **0 行**（#178 的处置在这条链上仍然成立）|
| **`just test-e2e`（Windows 11 / MSVC，本机实测）** | 退出码 **0**，**全绿**：第一段 **28 个目标全部通过**（33 条用例，0 失败）、第二段 `exit_residue` **1/1**、第三段 `portable` **3/3**。整体跳过 4 个目标（`tab_close` / `window_close` / `bitwarden_login` / `bw_import`），另有 5 个目标里各 1 条用例按平台跳过（`vault_unlock` 的 `VmLck`、三条串口的设备节点、`single_instance` 的 `/proc` 实例计数），原因都在日志里逐条写明（见 #176）。走到这一步修掉的是四类**与平台绑定**的问题：`$!` 的 PID 命名空间与运行中覆盖 exe（#162）、ConPTY 启动时要求先答 `ESC[6n` 且行尾必须是 CR（#175）、tokio 的连接在 Windows 上读不出"连接被拒"（#173）、隧道用例清场时先删主机行后删规则行（#174）。⚠️ 仍被跳过的那两类就是**平台缺口的现状**：会话级的进程回收（Job Object，plan 0108 的遗留）与假 `bw` 的可执行形态 |
| **`just test-e2e`（Windows runner，CI）** | 六个 job 里 `E2E（Windows）` **通过**（run `36231407751` @ `afc3d11`）—— 本轮三轮读数：第一轮 4 个目标红（`terminal_render` ×2、`single_instance`、`windows_ports`、`portable`）、第二轮只剩 `portable`、第三轮 **0 个**。⚠️ 与 Windows 无关的两处仍在：Linux 的 E2E 红在 `vault_unlock`（`读不到 app 的 /proc/<pid>/status`，三轮都在、与本次改动无关），第三轮另有一次 `sftp_host_to_host` 的凭据超时（该目标前两轮都过、文件未被本次改动触碰）|
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
  但无法修改。⚠️ plan 0507 补上的是**另一块**：主机**指纹**（库里的 `known_hosts` 缓存）看得到、
  删得掉；主机池本身的增删改仍然没有界面。
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
  （dmg 安装、数据在 OS 标准目录）—— 口径已由 plan 0408 收口（`portable.md` §3 / `scope.md` §9）。
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

## 已知问题（开放的）

> **完整原文在 [`issues.md`](./issues.md)**（编号永不复用；条目只增不改写），已处置的条目在
> [`archive/status-history.md`](./archive/status-history.md)。下表只列**仍开放、且写判据或写代码时
> 会直接用到的**那些 —— 工具链怪癖、上游行为与方法论教训留在 `issues.md`，它们多数已经被
> `AGENTS.md` 的某条规则或某个 ADR 吸收。

| 编号 | 一句话 | 现状 / 出处 |
|---|---|---|
| 45 | `SIGKILL` 的投递是异步的 | 等「消失」，不是等「信号发过了」 |
| 46 | `portable-pty` 的 `Child::kill()` 不是纯 `SIGKILL` | 收整个会话要另外点名（`pty/teardown.rs`） |
| 48 | `/proc/<pid>` 存在 ≠ 进程还活着（僵尸也有目录项） | 判活要看状态位；macOS 没有 `/proc`，见 #181 |
| 60 | 托盘在 Linux 上要写 `$XDG_RUNTIME_DIR`，只读环境里建不起来 | 托盘是可选能力，降级口径见 `AGENTS.md` §3.3 |
| 68 | `/proc/<pid>/exe` 可能带 ` (deleted)` 后缀 | 按可执行文件数实例时要先规范化 |
| 79 | `/proc/<pid>/mem` 的读走 `FOLL_FORCE`，绕过页保护 | 受保护页的已知边界（ADR-0002 D13） |
| 80 | `/proc/self/smaps` 的字段不处处都有，且属性行带缩进 | 解析它时按实际格式写 |
| 82 | `get_registry` 在本仓库是空的（命令未标 `#[inspectable]`） | `AGENTS.md` §7 那一条只能用替代证据 |
| 92 | 解锁期间 `VmLck` 涨的大头是 `cipher_memory_security`，不是我们那一页 | 不得据 `VmLck` 反推我们那一页的读数 |
| 124 | 全部 E2E 目标共用一个 app，而内存凭据缓存的键是 `(host, port, user, 认证方式)` | 新增 E2E 目标要算上缓存命中 |
| 130 | E2E 里的面板是有状态浮层，按钮是切换而不是打开 | 新增面板目标要写明这一点 |
| 141 | 上游只给同步的 `is_closed()`：半死连接要等保活耗尽（约 90 s） | 发现延迟未缩短（D15 的三个数仍是默认值） |
| 150 | 枚举出来的端口不保证能打开 | 本机 `/dev/ttyS*` 全部打不开 |
| 164 | macOS 上被 `SIGKILL` 的子进程停在「正在退出」，直到主端关闭 | 关标签页必须先关主端，否则 `shutdown` 阻塞 |
| 176 | Windows 上仍有两类 E2E 目标被显式跳过（缺 Job Object / 假 `bw` 不是 `.exe`） | 缺口仍在 |
| 183 | macOS 上 SSH 目标的回声判据稳定超时（30 s；`ssh_session` / `ssh_jump` / `ssh_config_import` 里每次变红的不全相同） | 未定位；把 plan 0112 的全部改动 `git stash` 之后重新执行 `just test-e2e` 同样复现。它拦住了 `just test-e2e` 的退出码 0 |
