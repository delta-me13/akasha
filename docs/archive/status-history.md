# STATUS 的历史（归档）

> 由 `just docs-archive` 从 [`STATUS.md`](../STATUS.md) 移来（规则见 `AGENTS.md` §8.3），**只移动、不改写**。
> `问题 #N` 的编号永不复用 —— 条目移走之后编号仍然有效，原文就在这里。

最近一次归档：2026-09-27。

## 摘要（历史）

**2026-09-26：`fix/win-e2e-162` 合入 `main`（合并提交 `bc1fda3`）** —— 分支的 7 个提交与本地
`main` 上那两份文档整理提交都在；三处冲突（`docs/STATUS.md` / `agent-runner.md` / `just.md`）
按"整理后的措辞 + 分支的新事实"逐条合并。合并之后又处置了三处：**#165**（Windows 上假上游接受到的
连接继承了非阻塞模式，连续执行 10 次里 7 次红 → **已修**，复测 10 次全过）、**#179**（前端把同一条
隧道事件记两遍，macOS 的 CI 上因此偶发红 → **已修**）、**#180**（Linux 的 `vault_unlock` 三次读数
被折成一个只填首槽的元组，Linux 上必红 → **已修**）。本机 `just ready` **6/6**、
`just test-e2e` **退出码 0**（见「已验证为通过」）。

**2026-09-19：布局重排（ADR-0008）** —— 6 个 workspace 成员（`akasha-core` / `akasha-pty` /
`akasha-ssh` / `akasha-serial` / `akasha-store` / `akasha-bw`）全部并入 `src-tauri/src/`
的域模块：`session/` `config/` `tunnel/` `pty/` `ssh/` `serial/` `store/` `bw/`。
每个域里纯逻辑与 app 侧的 `ipc.rs`（或 `ipc/`）分开；`crates/` 与 6 份成员 manifest 已消失，
`[workspace]` 只留 `[workspace.lints]` 供继承。护栏随之改到新路径：
`no-tauri-in-core-crates` → **`no-tauri-in-pure-modules`**（`files:` 覆盖全部
`src-tauri/src/**`，`ignores:` 列出允许碰 Tauri 的 app 侧文件），`no-unsafe-outside-store`
的放行点改为 `src-tauri/src/store/`，`no-ui-vocab-in-types` 收敛到 `src-tauri/src/**`。
`src-tauri/Cargo.toml` 的注释按所有者要求删除。

⚠️ **那次迁移漏看了两处**（2026-09-20 已修）：
1. Linux 的 `just ready` 编译 `tests/unlock_lifecycle.rs` 时报 `E0432`（`use akasha_lib::common;`
   —— `common` 是测试目标自己的模块，lib 里没有它）→ 问题 #169；
2. plan 0109 第 6 步写着“`just test-e2e` 的目标清单一并改”，但 31 个迁进 `tests/` 的域集成测试
   没有登记，`just test-e2e` 开头的 guard 对每一个都判红并 `exit 1`，E2E 用例一条都没执行
   → 问题 #170。
两处都在 `just ready` 的覆盖之外（前者被目标平台的 `cfg` 挡住，后者根本不在 `ready` 里），
所以只有把两条路都真的执行一次才看得见。修后读数：Linux 侧 `just ready` **6/6**、
`just test-e2e` 本机 **29 个目标 / 37 个用例全过**（见「已验证为通过」）。

**读数**：`just check` / `just clippy`（`-D warnings`）/ `just lint`（clippy + ast-grep scan +
ast-grep test，6 条规则）绿；成员集成测试 SSH **51/51**、store **114/114**、serial 两次配置通过。
`just test` 在本沙箱里 4 个 `pty::local` 用例失败于 `openpty: PermissionDenied`
（`AGENTS.md` §1 记的环境权限）；**带完整权限重新执行 `just ready`**，那 4 条通过，
剩下的一条是**已知问题 #165**（`bw::acquire` 的 install 用例偶发 `Peer disconnected`，未定位）。
`libudev-check` 需要 Linux 宿主。三条都不是这次改动引入的。

⚠️ **本文件下方的历史读数写的仍是旧 crate 路径**（`akasha_ssh::…` / `src-tauri/crates/…`）：
它们记录的是当时的事实，按 ADR-0008 §4 不追改。当前路径与规则以 `AGENTS.md` §3.1 / §6 为准。

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

**阶段 9 的四条已完成**（plan 0902 获取与前置检查、plan 0905 登录 / 解锁 / 锁定接进前端、
plan 0903 只读导入、plan 0904 离线缓存，四条都已归档）：口径由 [ADR-0007](./adr/0007-bitwarden-cli-acquisition.md) 定
—— 两个轴默认都取宿主机那一档、运行时下载只取 OSS 变体、session key 只在内存；
**导入**这一条把 SSH key 条目只读搬进密钥池，并额外记一行**来历**（上游条目 id / `revisionDate`
/ `fingerprint`）—— 那行来历就是 plan 0904 要用的缓存。库格式因此到 **v3**（`bw_items`，
v2 → v3 只加表）。**离线缓存**那一条把 plan 0903 记下的 `fingerprint` / `revisionDate` 用起来：
自检**不联网、不起 `bw`**（断网也能用），与上游比对只报不改。剩下的一条是
**0901 那三项真实输出**（需要一个真实 vault）；形状与判据见 [`adr/0007`](./adr/0007-bitwarden-cli-acquisition.md)。

**阶段 8 的两条都完成（plan 0801 / 0802）、阶段 11 的三条也已完成（plan 1101 / 1102 / 1103）**：
`akasha-serial` 落地（`Transport` 的串口实现 + `ports()` 枚举 + 参数校验，零 Tauri 依赖、
`libudev` 只在 Linux），一条串口 `Session` 接着接进 app —— 从界面打开、字节双向流动、关标签页即回收、
取值越界当场看到字段与取值、设备被拔掉时以可读原因结束并关闭标签页。判据与读数见下文「已验证为通过」。

**阶段 1 补了一条：Windows 目标的类型检查**（plan 0108）。`akasha-pty` 里的 `rustix::process`
（收会话用的 SIGKILL 封装）原本没有 `cfg` 守卫，于是 Windows 目标编译不过（问题 #149）——
CI 的两个原生平台检查 job（`checks-macos` / `checks-windows`）执行的就是
`cargo check --workspace --all-targets`，那两个平台因此至今
不可能通过。已按平台门控（`rustix` 变成 unix 专属依赖），能本地核对的三个成员现在都是退出码 0。
⚠️ **可编译不等于有实现**：Windows 上「回收整个会话」仍然是空的 —— 那条缺口见「进行中 / 下一步」。

**CI 的第三次运行把门禁推到两格绿**（plan 0102）：**Linux 的完整 `just ready` 与 Linux 的 E2E
都通过**（xvfb 下第一次把 E2E 走完），macOS 的类型检查也通过；Windows 的类型检查红在一处**真实的
`cfg` 错误**（测试脚手架用了 `portable-pty` 的 Unix 专有 `tty_name`，问题 #160），macOS 的 E2E 红在
三条串口目标（PTY 从端在 macOS 上打不开）以及它的两处连带效应（问题 #158 / #159）。前两轮的红因
（#154 sccache、#155 `libudev-dev`、#156 msys perl、#157 缓存工作区）都已处置并在这次运行里验证。

**"看起来像卡住"是两件事，都不是缺陷**（本会话实测）：第三次运行里 Windows 的 E2E 格子执行了
**32 分钟**（前 14 分钟是全量构建）且 app 没能起来 —— 它是**配方自己的缺陷**（#162：`$!` 在
Windows（MSYS）上是本 shell 的 PID，配方却拿它问 `tasklist` / `taskkill`，于是启动后 0.3 秒
就判定"app 没起来"，收尾里 `wait` 又等一个没被杀掉的 `cargo run`），已修复；
而紧随其后的那次运行一直停在 `pending`，那是 `main` 上的**排队**语义（#161），不是卡住。
两条的判据与处置（`workflow_dispatch`、配方里"日志是空的"会明说、典型耗时表）分别是 #161 / #162。
⚠️ 推送之后本机已能读 Actions（`git credential fill` 可用），结论不再只能从网页看。

**macOS 的会话回收从"只 killpg"补成与会话等价的实现**（本会话，macOS 26.6.2 / arm64）：
`teardown::kill_session` 在非 Linux 的 unix 上原先是"没有可移植的会话枚举"而退化成进程组，
于是 `nohup` / `trap "" HUP` 这类**自己一个进程组**的作业收不回来（三条看门狗用例因此在 macOS 上
必红）。现在它由两件事拼成：`ps -Ao pid=` **只当 pid 列表**用，会话归属交给内核的 `getsid` ——
⚠️ `ps` 的 `sess` / `tsess` 两列在 macOS 上打印的是会话**指针**，对任何进程都输出 0
（连会话首进程自己也是 0，本机实测），拿它当会话 id 会一个进程都匹配不到。
不引入 `unsafe` 也不加依赖：不用 `libproc` 的 `proc_listpids`（`AGENTS.md` §3.4 只允许
`akasha-store` 出现 `unsafe`），而 `getsid` 本来就在已依赖的 `rustix` 里。
⚠️ **ADR-0005 §3.2** 的括号里写的是 Linux 的机制（扫 `/proc`）—— 那份 ADR 已定案、不可改，
机制的平台差异以 `akasha-pty/src/teardown.rs` 的平台差异表为准（同 ADR-0006 那处过期句子的处理口径）。
同时补上一处**必须关闭主端**的顺序：macOS 上被 SIGKILL 的子进程可能停在 `ps` 的 `?E`
（正在退出）状态里，主端还开着又没人读时那个退出不结束、`wait4` 跟着一起不返回 ——
症状是关标签页时 `shutdown` 无限阻塞（本机实测：`shutdown` 5 秒内不返回，子进程 state `?Es`）。
判据：**本机 macOS 上 `just test` 从 4 条红变成 458/458 全绿**（Linux 侧同一套用例不变）。
⚠️ 串口那三条走"PTY 从端当串口"的脚手架在 macOS 上仍然不成立（问题 #158 的同一处平台限制，
上游对 pty 设波特率返回 `ENOTTY`），本轮把它们与整份 `pty_roundtrip` 显式门控到 Linux 并写明原因。

**文档门禁改为非快速失败**（本会话）：`just docs-check` 原先把 `just docs-style` 作为独立一行调用，
第一次失败即终止整个配方 —— 语体命中会把"命令未漂移 / ROADMAP 预算 / plan 预算"三类检查**全部掩盖**。
现在四部分在同一个 shell 块里执行完再汇总退出（负例实测：一份语体命中的文档与一份命令漂移的文档
同时在场时，两项在同一轮里都报出），单份文档命中超过 20 条时列出前 20 条并写明剩余条数。
`README.md` 同时按软件工程标准语体重写，并纳入"命令未漂移"的反向检查（`CLAUDE.md` 除外，见问题 #153）。

阶段 4 的八项（每项一句）：**SQLCipher 加密库可打开**（0401）、**口令只从一条路径进入且可真正验证**
（0402 —— 拆分"打开"与"新建"；此前在文件不存在的路径上**任何口令都能打开**）、
**口令在内存中同样受保护**（0406）、**四类池可增删改查**（0403）、**库中数据可导出也可还原**
（0404）、**解锁与锁定构成完整生命周期**（0407 —— 真实 app 上 `VmLck` **0 → 176～192 → 0 kB**）、
**目录迁移后数据仍在且可用**（0405）、**便携目录不可写时拒绝启动**（退出码 2）。
**阶段 5 的四块**依次是**连接 + 认证**（0502）、**主机密钥的信任策略 + 本仓库首次库格式迁移**
（0503）、**接入 IPC 与前端**（0504）、**`direct-tcpip` 原语 + 跳板（ProxyJump）**（0505），
最后是 `~/.ssh/config` 导入（0506）—— 逐条落点见对应 plan（索引在 `docs/plans/README.md`）。

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

## 被取代的读数
| ↑ **同一份代码的下一次运行**（`36251661992` @ `08fffb2`，只改了文档） | ⚠️ 有两处红：Windows 的 `ssh_config_import`（"选择器里有导入面板"超时，底层的 eval 9 s 未返回）与 Linux 的 `ssh_session`（服务端回声 30 s 没出现在终端上）—— 两处都停在 `tests/support/mod.rs:244` 那个等待辅助上。**重新执行这两个 job 即绿**（六格全绿），因此记为**偶发**，与本次改动无关（那两条路径不碰隧道、也不碰 `vault_unlock`）|
| ↑ **同一配方在修复前的读数**（本机首次完整执行） | 退出码 **1**：第一段 28 个目标里 **13 个通过**（其中 5 个按平台显式跳过）、**15 个红**；第二段 `exit_residue` **1/1**；第三段 `portable` **3/3**。修复前停在第一段的 `app 未登记到 discovery 目录` 并挂到取消。那 15 个红灯当时被归成三类"平台缺口"（会话 / 隧道回收为空、ConPTY 下本地终端输出到不了 raw 通道、`bw` 的假 CLI 是 `#!/bin/sh`）；后续定位表明其中**大部分是判据自己与平台绑定**，真正剩下的只有后两类里的形态问题 |
| ↑ **CI 的第一、二轮 Windows 读数**（`36226121050` / `36228577977`） | 第一轮 4 个目标红、第二轮 1 个（`portable`）；性质与逐条处置见 #177 |
| ↑ **CI 第三次运行的实际读数**（run `34989700283` @ `b7f6a62`） | **Linux 的 `just ready` 通过**（门禁六步全绿）、**E2E（ubuntu-latest）通过**、**检查（macos-latest）通过**；`检查（windows-latest）` 红在类型检查：`error[E0599]: no method named tty_name`（`tests/support/mod.rs`，问题 #160）；`E2E（macos-latest）` 红在三条串口目标（`串口打不开：/dev/ttys000（Not a typewriter）`）与连带的 `bw_import`，收尾时 bash 报 `unbound variable` 把退出码换成 **127**（问题 #158 / #159）；第五个 job（`E2E（windows-latest）`）在本机读到时仍在运行。⚠️ 这次运行里 `RUSTC_WRAPPER` 为空、编译真的开始 —— #154 的处置得到验证 |
| ↑ **第三次运行验证了其中三处处置** | `RUSTC_WRAPPER` 为空且编译真的开始（#154）；Windows 走到了编译测试目标这一步、`openssl-sys` 的 vendored OpenSSL 已构建完（#156）；Linux 日志里 `could not find Cargo.toml` 的计数为 **0**、缓存键由 workspace 元数据给出（#157）。**E2E（ubuntu-latest）**：28 个目标全部执行完，`test result: ok` **29 条**、失败 **0 条**（唯一一处跳过是运行器不提供 `WEBGL_lose_context`，用例自己写明了原因）—— CI 上的 E2E 第一次完整执行 |
| ↑ **CI 第二次运行的实际读数**（run `34987984477` @ `9081ac9`） | **macOS 那一格通过**（全部步骤绿，平台类型检查第一次有结论）；Linux 红在 `just ready` → `lint` → `clippy`、Windows 红在 `just check`：报错分别是 `libudev-sys` 的构建脚本找不到 `libudev.pc`（问题 #155）与 `openssl-sys` 的 vendored OpenSSL 配置失败（问题 #156）；`e2e` 仍为 `skipped`。日志里 `env:` 段落显示 `RUSTC_WRAPPER` 为空，编译确实开始 —— 空值这条处置有效。三处修正见 plan 0102 的「两次运行」 |
| ↑ **CI 首次运行的实际读数**（run `34972060468` @ `24d4f72`；清理后的 workflow 再次运行 `34986599520` @ `31a4d7c` 同因） | 三个 job **全部失败于同一步**：Linux 的 `just ready`（红在 `lint` → `clippy`）、Windows 与 macOS 的 `just check`。三处报错逐字相同：`could not execute process` + `sccache <rustc> -vV` + `(never executed)`，以及 `No such file or directory (os error 2)`。**其余步骤全部成功**：checkout、系统依赖、`rust-toolchain`、`rust-cache`（首次 `No cache found`）、`install-action`（`just 1.58.0` 校验通过）、`npm install -g @ast-grep/cli@0.45.3`。`e2e` 状态为 **`skipped`**（`needs: checks-linux`），三平台 E2E 至今没有读数 |
| ↑ **本会话再核对一次**（改过会话回收之后） | 同三条仍**退出码 0** 且无警告；换 `cargo clippy -p akasha-pty --target x86_64-pc-windows-msvc -- -D warnings` 也**干净** —— 三平台的会话回收确实由条件编译分开：Linux 走 `/proc`、其他 unix 走 `ps` + `getsid`、非 unix 是那个"什么都不做"的分支，各自的辅助函数在别的平台上不编译。⚠️ 交叉目标的**测试**目标本机仍编不过（`criterion` → `alloca` 的 C 构建要 MSVC 的 `malloc.h`），所以"Windows 测试目标上没有 dead_code"只能靠 `cfg` 推理 + 那个测试模块的门控选择来保证 |
| ↑ **同一命令带 `--all-targets` 在本机过不去** | `criterion`（dev-dependency，只有 bench 用它）拉进 `alloca v0.4.0`，它的 C 构建脚本要 MSVC 的 `lib.exe` —— 本机没有 MSVC 工具链。**与本次改动无关**；CI 的 Windows 格子上有那个工具链 |
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
| ↑ **`连接中 → 失败` 可见，且能手动重试**（plan 0601） | ✅ 连不上的那条（规则指向一台**不可达主机**）：`tunnel_open` 回 `{handle, failure:{kind:"failed",…}}`、probe `state = failed`、事件里有 `failed`；`tunnel_retry` 再走一遍 `connecting → failed`；**全程没有 `reconnecting`** —— 首次连接失败不自动重试（plan 0605 的口径） |
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
| ↑ **`just test-e2e` 全部通过**（2026-09-20 本机重新执行） | 退出码 **0**：第一段（默认收托盘）**28 个目标 / 33 个用例** + 第二段 `exit_residue` **1 个用例** + 第三段（可搬迁性）`portable` **3 个用例** = **29 个目标 / 37 个用例**、0 失败。三处跳过都写明了原因：`smoke` 的截图（Wayland 会话拿不到 Victauri 认的原生句柄）、`tab_close` 的丢上下文（当前不是 WebGL 渲染器）、`tunnel_reconnect` 的托盘断言（本机建不起托盘，probe 报 `ready = false` —— 走的正是 §3.3 那条“托盘起不来就降级”的路径）。⚠️ `E2E_NO_APP` 的成员不在第一段的目标清单里（`just test` 是它们的入口），`E2E_SELF_APP` 的 `portable` 由第三段执行 |
| ↑ **判据：对着真实上游下载一次**（一次性探针，plan 0902；读完数即删） | ✅ `cargo test -p akasha-bw --test probe_real_upstream -- --nocapture`（**24.63 s**）：解析到上游最新 `cli-v2026.8.0` → 真的从 GitHub 拉下资产 → 落在 `<数据目录>/bitwarden/bw-2026.8.0/bw`、**141819984 字节** → crate 报的 SHA-256 是 `d8bbc213…b1b1704`，与 `sha256sum` 直接算那个 zip 的**逐字符相同** → `Cli::version()` 报 `2026.8.0`、`Cli::variant()` 报 `Oss`、`Cli::status()` 报 `Unauthenticated`。⚠️ 探针里那次**直接执行**（不设 `BITWARDENCLI_APPDATA_DIR`）输出为空：这个沙箱的家目录只读，`bw` 建不出它自己的 `data.json` —— 这正是"`managed` 那一轴有必要"的一个旁证 |
| ↑ **一个把 IPC 占住 27 秒的问题（plan 0902 的实现期发现）** | ⚠️ 本机 `host` 轴上**确实有一个 `bw`**（发行版的 `bitwarden-cli` 把 `/usr/bin/bw` 指向 npm 包），而它在这个只读家目录里要 **13 秒**才报错。第一版每个快照都起三次进程（版本 / 帮助 / 状态），于是 `bw_cli_settings` 撞上 Victauri 的 30 秒 eval 上限。处置两条：**探测结果按"解析出来的程序路径"缓存**（版本与变体对一个给定的程序文件是不变的），并且**连 `--version` 都答不出来的那一份不再往下问状态**（那不是"状态读不出来"，是"这一份 `bw` 用不了"）。⇒ 正常机器上每个快照不再起进程；本机这个坏 `bw` 上每次切到它也只要一次 13 秒 |
| ↑ **E2E 目标之间会互相影响**（plan 0903 的实现期发现） | 全部目标在**同一个 app 进程**里执行，所以上一个目标留下的界面状态也留着：`bitwarden_login` 结束时 Bitwarden 面板是**开着**的，而 `bw_import` 原先直接点 `.tab-new-bw`（那是个开关）→ 面板被关闭 → 下一步的判据等到超时。单独执行那一条时全绿，**全量执行才红**。处置：用 `support::open_bitwarden_panel`（它先关再开）。⚠️ 与 `ssh_config_import` 头部记的"名字撞上内存凭据缓存"是同一类问题：新加目标时要问一句"上一个目标留下的是什么状态" |
| ↑ **判据：断网能校验缓存、联网能看出上游变过**（E2E `bw_import` 第 8 / 9 段，plan 0904） | ✅ 同一个目标里接着验：导入后自检 = "完好"且带上算出来的指纹 → **把库里那把私钥换成另一把** → 自检报"对不上"（诱饵：少了它，"自检"与"永远说好"分不开；两次算出的指纹确实不同）→ 上游比对 = "没变" → **只改假 `bw` 的 `revisionDate`** → 比对报"上游变过"。自检那条**不起进程、不联网**，所以断网时它照样成立 |
| ↑ **判据：导入后可用该密钥建立 SSH 连接**（E2E `bw_import`，plan 0903） | ✅ 假 `bw` 的 `list items --raw` 交出一份**真的** ed25519 私钥 + 一条登录条目 → 面板导入报告说"上游给了 2 条，其中 SSH 密钥 1 条；新增 1 条"并带名字与上游给的指纹（登录条目一个字都没进）→ `~/.ssh/config` 里 `IdentityFile` 的 basename 与钥匙名相同 → `vault_hosts` 那一行 `keyId` 指向它 → 开会话连上，**服务端 `offered_keys` 里的指纹与导入时存下的逐字符相同**（"连上了"与"用的是这把钥匙"是两件事）→ 字节能双向流。另一步：没登录时导入只报一句话、池里一行都没多 |
| ↑ **判据：登录 / 解锁 / 锁定在界面上跟着 CLI 走**（E2E `bitwarden_login`） | ✅ 假 `bw`（脚本，状态放在隔离目录里）：`host` 轴报"PATH 里找不到 bw"、`managed` 轴报"还没有下载过"（**两句话分得开**）→ 落点里放一份可执行文件之后报出它的版本与 `oss` 变体 → 面板显示同一串 → 填自托管地址 → 显示的**是读回来的那一串** → 口令错时面板上的话是 `bw` 自己的原话 → 口令对则"已解锁 + session key 在内存里"（且输入框里那份主密码被清掉）→ 点锁定变"已登录，未解锁 + 没有 session key" → 解锁回到已解锁 → **外部**删掉假 CLI 的解锁标记（等价于在别处执行了一次 `bw lock`）再点刷新 → 界面跟到 `locked` 且 `hasSession = false`（ADR-0007 D10）→ 登出回到未登录 |
| ↑ **判据：session key 那一页真的被护住**（ADR-0002 D13 的判据表，按新用途重验） | ✅ `session_protection`（Linux，`--nocapture`）：`VmLck` **0 → 4 kB**；多出来的那一段是 `---p` 且 `VmFlags` 含 `dd` 与 `wf`（不进 core dump、不落 swap）；取值仍逐字节相同（读它要走一次提权窗口）；丢掉之后 `VmLck` **回到 0**。⚠️ D13 表里两条不成立的边界照旧：Windows 没有静止只读那一档，`/proc/self/mem` 仍读得到（问题 #79） |
| ↑ **判据：两个轴各自可解析，且"没有 `bw`"与"还没下载"分得开** | ✅ `host` 轴上找不到 → `MissingBinary`；`managed` 轴上没有版本目录 → `NotInstalled`；解析时**不会**从一条轴静默滑到另一条（ADR-0007 D5，两种错误各有单测） |
| ↑ **判据：变体判定** | ✅ 两份 `cli-v2026.8.0` 实测输出的命令表各判一次：有 `device-approval` → 专有、没有 → OSS。诱饵（`device-approval` 只出现在 `Examples:` 段）与"命令表为空"都判为 `unknown`（**不默认成 OSS**） |
| ↑ **判据：主密码与 session key 都不进 argv**（ADR-0007 D7 / D8） | ✅ 假 `bw` 把收到的 argv 与 `$BW_PASSWORD` 各写一份：口令确实到了子进程（会话 key 就是它拼出来的），而 **argv 里搜不到它**，且带着 `--passwordenv` 与 `--nointeraction`；带 session 的命令只多一个环境变量、不多参数 |
| ↑ **判据：下载 → 解包 → 落盘 → 清旧版本**（进程内假上游） | ✅ 列表里取**最新**的那条 `cli-v*`（draft 与 prerelease 跳过）、报出的 SHA-256 等于这次下载的字节、包里那个可执行文件落到 `bw-<版本>/bw` 且带可执行位、装第二个版本之后旧目录被清掉（只剩一个版本） |
| ↑ **判据：压缩包里没有可执行文件时不许猜** | ✅ 报错里列出包内实际条目（`README.md`），且**不留下一个看似装好的版本目录** |
| ↑ **失败分档** | ✅ 未登录（`You are not logged in.` + 退出码 1）/ 明文 HTTP / 自签证书 / 网络（`FetchError` + `ETIMEDOUT`，只留第一行、不把堆栈带进消息）/ 认不出的原话（`CommandFailed`，**原样**交给用户，不猜类别） |
| ↑ **超时杀 + 收** | ✅ 假 `bw` 睡 5 秒、期限 400 ms → `Timeout`（`kill` 之后 `wait`，不留僵尸） |
| ↑ **判据：串口会话能打开并双向传字节、关标签页即回收**（plan 1101） | ✅ `serial_session` E2E（真实 app + 测试进程自己造的一对 PTY，从端的路径当设备，**1.00 s**）：界面点"串口" → 选池里那一行 → 标签页出现且"已连接" → **设备 → 界面**（往主端写 `from-device-1101`，屏幕文本里出现它）→ **界面 → 设备**（在标签页里敲 `to-device-1101`，主端读到同一串）→ 关标签页后 `sessions` probe 回到打开前的 `{"live":1,"registered":1}`（**两数相等**） |
| ↑ **判据：界面上看到本机枚举结果并据此（或手输路径）打开；取值越界时显示字段与取值**（plan 1102） | ✅ `serial_ports_ui` E2E **1.46 s**（设备同上）：后端枚举 **32 条** → 面板上那个计数与它**相等**（库**故意不解锁**：池那一块显示"先解锁"、端口照常列出 → 三条输入并列）→ 点第一条枚举结果 `/dev/ttyS0` → **"设备路径"那一栏变成它** → 手输 PTY 从端的路径 → 「打开」→ 标签页"已连接"、`from-device-1102` 到了界面、`to-device-1102` 回到设备 → 关标签页后 `sessions` 回到 `{"live":1,"registered":1}`；数据位填 **9** → 「打开」→ 那个面的报错行里是 **`data_bits = 9`**、关闭之后 probe 同样回到打开前（**没有登记**）；数据位填 `abc` → **不开面**，`[data-picker-problem]` 说清是哪一栏 |
| ↑ **池行 → 参数这条搬运，以及端口映射与"契约加一档"的编译期代价**（plan 1101 / 1102，crate 层） | ✅ `akasha --lib serial` **7 passed**（1101 的 5 条 + 1102 的 2 条）：两个 IPC 枚举的全量映射 / 六个字段的搬运 / **越界取值报字段与取值**（`data_bits` 与 `stop_bits` 各三个取值）/ **空路径报出用户给的那一串** / 不存在的设备报出它试过的路径 / 四档 `PortKind` 各有去处（`Usb` 五项一个不丢）/ 五项都缺时保持 `None`。⚠️ `SerialIpcError` 加 `enumerate` 之后，前端两处 `switch`（`SerialInvokeError` / `SerialPortsUnavailable`）当场**编译不过**（TS2366） |
| ↑ **判据：设备消失时会话结束、没有残留注册（且原因可读）**（plan 1103） | ✅ `serial_device_gone` E2E **0.36 s**（设备同上）：手输从端路径打开 → 标签页"已连接" →（**关闭唯一一份主端 = 拔掉设备**）→ 标签页自己关闭、`[data-session-notice]` 里是 **`「/dev/pts/0」已结束：串口设备已断开：/dev/pts/0（Broken pipe）`**、`sessions` probe 回到打开前的 `{"live":1,"registered":1}`。⚠️ 这条用例**先断言那句话此刻不在界面上**（正反例成对：否则"拔掉之后它出现了"会被上一次留下的通知顶过去） |
| ↑ **读端失败的形态、原因出得了载体、"我们自己收尾"不留原因**（plan 1103，crate + app 层） | ✅ `akasha-serial` **25 passed**：关闭 PTY 主端 → 读端报错（**不是** `Ok(0)`），且 `stream_error` 说得清是哪台设备（含路径与`已断开`，问几次都是同一句）；设备安静（超时 / 零字节）与**我们自己收尾**都不留原因。`akasha-pty` **40 passed**（`stream_error` 的默认值是 `None`）。`akasha --lib session` **18 passed**：假载体报一句原因 → `retire` 把它**原样**交出（会话层不拼那句话）；**反例**：不报原因的载体拿到 `None` |
| ↑ **判据：主机密钥变化即拒绝，未见过的询问一次**（plan 0503） | ✅ `akasha-ssh` 的 7 条：未知且无人可问 → `HostKeyUnknown`（携带用于核对的指纹，且**认证尚未开始**）；确认 → 写入缓存，**第二个连接 0 次询问**；记录不匹配 → `HostKeyChanged`（**两个指纹都在**）且**不发起询问**；用户拒绝 → 拒绝连接且**不记录**；用户文件中已认可 → 连通且文件**逐字节未变** |
| ↑ **本仓库首次格式迁移**（plan 0503） | ✅ `akasha-store` 的 6 条：`DDL_V1` 构造出**真实 v1 库** → `open` 之后 `user_version = 2`、五张表存在、**该 host 行仍在**；再次打开当前格式的库**不写入任何字节**；缺表的 v1 **不迁移**；加密导出与明文导出两条还原路径均**升级副本、来源逐字节不变** |
| ↑ **判据：同主机三个连接仅询问一次凭据**（plan 0502） | ✅ `three_sessions_ask_for_one_credential`：`provider.calls() == 1`、缓存 `len() == 1`、服务端三次均收到**同一口令** |
| ↑ **认证顺序由协议交互验证**（plan 0502） | ✅ 服务端记录的序列：`publickey → password`、`publickey → keyboard-interactive`；agent 不可用时序列中**没有** `publickey` |
| ↑ **`nodelay` 实际生效**（plan 0505 修正） | ✅ `tcp_stream` 自建 TCP 时显式 `set_nodelay(true)`（问题 #120：上游仅在 `client::connect` 中读取 `Config::nodelay`，而两条路都使用 `connect_stream`） |
| ↑ **`Cargo.lock` 增量仅一行**（plan 0504 / 0505） | ✅ 新增 `akasha → akasha-ssh` 这条边**只增加一行**；0505 **未增加任何行**（无新依赖，`rand` 早已是 `akasha-ssh` 的真依赖） |
| ↑ **解锁 / 锁定 / 内存回收**（0407） | ✅ 真实 app 上 `VmLck` **0 → 176～192 → 0 kB**；进程内存扫描（带正对照）：口令 `1 → 2 → 2 → 1` 处、派生密钥 `3 → 1` 处 |
| ↑ **导出与还原**（0404） · **目录迁移**（0405） · **退出零残留**（0204/0205） | ✅ 三项判据仍然全部通过（`just portable` **3 passed**；`app 已退出` + `零残留`） |
| ↑ **终端 / 会话判据（未退化）** | `renderer = webgl`；写入 8 MB 数据后仍可交互；raw 通道 10.73 MB / 172 批；收尾帧 1 个、console 零异常 |
| ↑ **§7 的 registry 一条：当前不可满足**（问题 #82） | `get_registry` 返回 **`[]`**（命令均未标 `#[inspectable]`）。**替代证据**为真实路径上的 `invoke_command` 成功 —— plan 0704 与 1101–1103 的新命令均以此验证 |

## 已处置的问题
 165. **`akasha-bw` 的 `install_takes_the_newest_release…` 偶发失败：Windows 上接受到的那条连接
      继承了非阻塞模式**（**已修**，本会话定位）：`Stub` 的接受循环把**监听套接字**设成非阻塞（它要能
      轮询停止标志），而 **Windows 的 `accept` 返回的连接会继承这个非阻塞模式**（Linux 上按 POSIX
      返回阻塞套接字）—— 于是 `serve` 里第一句 `read_line` 在客户端的请求还没到时返回 `WouldBlock`
      （os error 10035），这条连接被丢掉，客户端读到的是"连接被中止"（os error 10053；macOS 上曾记成
      `Peer disconnected`，同一条用例、同一处时序，未单独定位）。**本机基线：同一个用例连续执行 10 次，
      7 次红**；把 `max_idle_connections(0)` 当变量试过（排除连接复用），仍 9/10 红。定位靠给假上游加
      一次性探针（已完成即删）：失败那次服务端只记到"接入一条连接 → 这条连接结束"，中间没有任何请求行，
      错误正是 `WouldBlock`。处置：接受到之后 `stream.set_nonblocking(false)` 再交给 `serve`
      （**复测：10 次全过**）。⚠️ 它只在 Windows 上现形，Linux 的 CI 一直是绿的；同形状的问题要一起问：
      "把监听套接字设成非阻塞之后，接受到的那条连接在这个平台上是什么模式"。

 166. **`clippy::result_large_err` 只在"别的 crate"看那个错误类型时才报**（本会话实测，**已修**）：
      `tests/known_hosts.rs:134` 与 `tests/pipelining.rs:771` 返回 `Result<_, SshError>`，曾被它命中
      （`SshError` 正好 128 字节 = 阈值）。⚠️ **同一个 crate 内部怎么改都不出声**：在
      `src/ssh/error.rs` 里加 `pub` / `pub(crate)` / 私有三种可见性的同签名函数，一个都不报 ——
      用一个 20 行的同构 crate 双向复核过（本 crate 的错误类型不报，外来的报）。所以本仓库里能
      触发它的只有 `tests/`，而那几条私有辅助函数一改形状，这条约束就静默消失。处置：把
      `HostKeyChanged` 的 `recorded_in` 装箱（`RecordedIn` 48 字节，是那个变体最大的字段），
      `SshError` 128 → 88 字节；并在 `src/ssh/error.rs` 加一条**直接量 `size_of`** 的用例
      （阈值同样复核过：`size_of` 正好 128 即触发），使它不再依赖测试辅助函数的形状。

 167. **Windows 上 `VirtualLock` 的额度归整个进程共用，而 SQLCipher 的
      `cipher_memory_security` 也在用它**（本会话实测，**已修** —— ADR-0009 / plan 0110）：
      修掉 #166 之后 `just ready` 在本机继续往下走，停在 `test` —— `tests/export_contract.rs` 的
      `an_encrypted_export_restores_into_another_directory` 在 `keys::private_key` 上报
      `MemoryProtection(MemoryError(Os { code: 1453 }))`（`ERROR_WORKING_SET_QUOTA`）。
      根因用可复现的读数钉住：
      * **Windows 把“一个进程能锁多少页”限定为它的最小工作集**。一个 20 行的探针进程实测：
        4 KiB 的页能锁 44 次、16 KiB 的 11 次、32 KiB 的 5 次 —— 三者都恰好 176 KiB；
        调 `SetProcessWorkingSetSize(…, 64 MiB, …)` 之后同一份代码能锁 16334 页 ≈ 64 MiB。
      * SQLCipher 的 `cipher_memory_security = ON`（D1）会给**每一次**分配调 `VirtualLock`
        （`libsqlite3-sys` 的 `sqlcipher/sqlite3.c`：`sqlcipher_malloc → sqlcipher_mlock →
        VirtualLock`，失败**只记日志**），于是它与我们自己的 `memsafe` 受保护页争用同一个 176 KiB 额度。
      * 用例内插桩量到的余量（单位 = 一个 16 KiB 的私钥页）：进程起来 **11** →
        `populated()` 之后 **3** → `restore` 之后 **4** → `open(restored)` 之后 **0**。
        也就是说失败取决于那一刻 SQLCipher 手里握着多少 —— 同一次完整运行里
        `the_export_file_is_itself_a_vault` 也红、单独执行又绿，`passphrase_contract` 那条同样
        时红时绿：这是**竞态**，不是某一条用例写错了。
      * ⚠️ 它同时是**产品**问题，不只是测试问题：App 里“库解锁着 + 读一把私钥”走的是同一条路。
      处置取了两条候选里更保守的那一条，并按本仓库的规矩写成 **ADR-0009**（ADR-0002 已定案、
      一字未改）：在 `src-tauri/src/store/protected.rs` 里，第一次构造受保护页之前把本进程的
      最小 / 最大工作集抬到 16 MiB / 256 MiB —— 一处 Windows-only 的 `unsafe`（调
      `SetProcessWorkingSetSize`），`Once` 保护，失败只记一条 `warn`。落地与读数在 plan 0110：
      新用例 `windows_holds_many_protected_pages_at_once` 先红（`Os { code: 1453 }`）、
      加实现之后转绿；`export_contract` / `passphrase_contract` = 18/18；
      修好之后在同一条路径上量到 SQLCipher 与我们的页合起来只占约 288 KiB。
      ⚠️ **CI 抓不到它**：完整门禁只在 Linux 执行，Windows 那一格只做类型检查 + E2E +
      `just serial-unit`（`--lib`）—— 集成测试在 Windows 上一次都没执行过。

 168. **本机 Windows 上 `just test` 另有三条红**（本会话实测，**已修** —— plan 0111）：完整一遍
      `cargo nextest run --workspace --no-fail-fast` = **422 条全部执行完、418 过、4 红**，
      其中一条是 #167，另三条是 —— `socks5_forward::a_socks5_port_reaches_whatever_the_client_names`
      （读 `REP` 时 `Os { code: 10054 }` = `ConnectionReset`：服务端本该先回那个字节，
      而 Windows 上 RST 会丢掉已排队的字节）；`transfer_atomic::the_local_endpoint_lists_a_directory`
      （判据拿 `std::fs::canonicalize` 的结果对照，而 Windows 的 `canonicalize` 给的是 `\\?\` verbatim 形态）；
      `passphrase_contract::a_fresh_vault_only_accepts_the_passphrase_it_was_created_with`
      （时红时绿，与 #167 同一个额度问题 —— 随 #167 一起转了绿）。后两条的修法分别是：
      拒绝之后**把对端剩下的字节读干净再关**（接收缓冲非空的连接在 Windows 上关闭发的是 RST，
      而 RST 会丢掉已经排队的 `REP`），以及把对照物改成“先规范化、再去前缀”（去掉 verbatim
      前缀是 `ssh/local.rs` 的 `tidy` 刻意要做的，红的是对照物）。⚠️ 这三条都要一台 Windows
      主机才能定位与验证：完整一遍 `cargo nextest run --workspace --no-fail-fast` 现在
      **423 条全过**，而 Linux 门禁看不到它们。

 169. **按 crate 名的重写会连带改掉测试目标自己的本地模块**（本会话实测，**已修**）：
      plan 0109 把引用从 `akasha_store::…` 改成 `akasha_lib::store::…` 时，
      `tests/unlock_lifecycle.rs` 里的 `use crate::common;` 被一并扫成 `use akasha_lib::common;`
      —— 而 `common` 是该测试目标自己的模块（`#[path = "store_common/mod.rs"] mod common;`），
      lib 里没有它。⚠️ **它整段在 `#[cfg(target_os = "linux")]` 内**：macOS 与 Windows 的
      `just check` 连这一段都不编译，所以那两个平台的作业全绿，红的只有 Linux，失败点是
      `test` 之前的编译（`E0432`：no `common` in the root），并且 `e2e-linux`
      因 `needs: checks-linux` 整格跳过 —— 于是链路上真正执行过那条分支的作业一个都没有。
      由此的教训：**平台门控的代码只有那一个平台的作业能证伪**；按旧名重写之后，要在会编译该
      分支的目标上执行一次 `cargo check --workspace --all-targets`。处置：改回 `crate::common`，
      Linux 侧 `just ready` 6/6 通过（修复前同一条命令报 `E0432`）。

 170. **归属清单不跟着文件迁移走，E2E 会在第一步判红**（本会话实测，**已修**）：
      plan 0109 第 6 步要求迁移成员的 `tests/*.rs` 时"`just test-e2e` 的目标清单一并改"，
      实际只改了 `just test` 那一侧 —— 31 个迁进来的域集成测试（store / ssh / sftp / pty / bw）
      既不在 `E2E_TARGETS`，也不在 `E2E_NO_APP`，于是 `just test-e2e` 开头的 guard
      （问题 #36：生成物无人执行）对每一个都报错并 `exit 1`：**一条 E2E 用例都没执行**。
      ⚠️ 那个 guard 只看 `tests/*.rs`，与平台无关 —— 三个平台的 E2E 作业都会停在这里，
      而它不在 `just ready` 里，所以门禁全绿也发现不了：判据只能来自真的执行一次
      `just test-e2e`。处置：把 31 个目标按字典序登记进 `E2E_NO_APP`（它们的入口本来就是
      `just test` 的 nextest 全量运行），并把"迁移这类文件必须同步登记"写进那段注释。

 171. **发现目录由 `std::env::temp_dir()` 推导，而它在 `TMPDIR` 缺席时不取 `/tmp`**（本会话实测，
      **已修**）：victauri 的发现目录是 `<temp>/victauri/<pid>/`，两侧各自算一遍 —— app 用
      `std::env::temp_dir()`，`just test-e2e` 用 `${TMPDIR:-/tmp}`。执行器只给动作
      `BASE_ENV` 的最小环境（不含 `TMPDIR`），于是本机（macOS）上两侧分叉：app 落到
      `confstr` 给的那个私有目录（`/var/folders/…/T/victauri/`）、配方去找 `/tmp/victauri`，
      症状是 **app 已启动、进程确实活着，配方却报"app 没起来（日志：…）"**，E2E 在这里停 120 秒后
      `exit 1`，一条用例都没执行。定位读数：执行器里打印 `TMPDIR=[unset] tmp=[/tmp]`，
      而 app 的日志与 `<系统临时目录>/victauri/<pid>/metadata.json` 都在。
      处置两条 —— **① 样板给一个确定的 `TMPDIR`**（`policy.env`，见 `agent-runner.md` §3 / §6，
      这是让两侧取同一个值的那条）；**② 配方按候选目录逐个找**（`/tmp` 与 `$TMPDIR` 各算一个），
      对"app 由别的进程启动、两边 temp 目录不同"也成立（`test-e2e` / `portable` 都是）。
      ⚠️ 教训：**`rm -rf` 式的"临时目录"不能在两侧各推一遍** —— 要推就取同一个来源。

 172. **"本机用户名"取自环境变量，而执行器的最小环境里没有它**（本会话实测，**已修**）：
      `~/.ssh/config` 导入（`import_ssh_config`）在没写 `User` 的条目上要补本机用户名，
      `local_user()` 读的是 `USER` / `USERNAME`；而执行器的 `BASE_ENV` 不含 `USER`
      （`PATH` 与 `HOME` 另接），于是 `ssh_config_import` 与 `bw_import` 两条 E2E 目标
      **在点完导入之后一起等满超时**：界面上那句话是"取不到本机用户名（`USER` / `USERNAME`
      都没有）"，而断言只报"导入报告出现了 超时" —— 那句话是逐步缩小范围之后才看见的
      （`support::wait_js` 现在会把界面上"已经在说"的那句话一起打进 panic 消息）。
      处置同 #171：样板 `policy.env` 里补 `USER`（值由 `pwd.getpwuid(getuid())` 取，
      **不读环境变量** —— 环境变量的取值可以被任意改写，而它要写进样板）。
      ⚠️ 教训：**"进程外面长什么样"是 E2E 的一条隐式前提**，最小环境把前提抽走时，
      症状出现在最远处（界面等超时），而不是在环境那一层。
 173. **Windows 上 tokio 的连接对"连接被拒"只报超时，于是"端口已释放"那条判据永不成立**（本会话实测，
      **已修**）：`tunnel_local_forward` / `tunnel_dynamic_forward` 在停止隧道之后要等"端口不再接受连接"，
      判据是 `timeout(2s, TcpStream::connect(…))` 落在 `Err` 一侧。本机实测：**同一个已经关闭的回环端口**，
      `std::net::TcpStream::connect` 立刻给 `ConnectionRefused`（os error 10061），而 tokio 的连接三次全部
      **超时** —— 于是"端口已经还给系统"被读成"还在接受连接"，用例等满 20 秒判红。
      app 侧的证据与它相反：`netstat` 里那个监听只在停止后的**前 3 个采样**里出现过（实时采样 20 余次），
      `app_state` 的 `tunnels` 也早已没有那条规则。处置：新增 `support::port_released(port)`，改用**阻塞**的
      `std::net::TcpStream::connect_timeout` 探（判据取"连不上"，不取"恰好是 ConnectionRefused"——回环上没有
      防火墙，握手成功就等于有一方在监听）。⚠️ 教训：**"连不上"这件事在 Windows 上不能用 tokio 的连接来问**。
      同一个形状还在 `tests/local_forward.rs` 里（它把超时也算进"连不上"，因此在 Windows 上是过的 —— 但过的
      理由不是它想验的那条）。

 174. **隧道用例的 `seed` 先删主机行、后删规则行，同一个数据目录上再执行一次必撞外键**（本会话实测，**已修**）：
      六条隧道目标（`tunnel_state` / `tunnel_local_forward` / `tunnel_dynamic_forward` / `tunnel_remote_forward` /
      `tunnel_reconnect` / `tunnel_teardown`）的 `seed` 都按"先 `forget(主机)`、再删转发规则"清场，而规则的
      外键指着主机行：上一次留下的规则还在时，删主机行当场报
      `Conflict { pool: "hosts", detail: "FOREIGN KEY constraint failed" }`。CI 每个作业只执行一遍、碰不到它，
      本机反复执行才暴露 —— 而仓库明确要求"这些用例不依赖上一次执行干净了"
      （`support::forget` 的注释里写着这条）。处置：六个 `seed` 一律改成**先删规则、再删主机行**，顺序的理由
      写进注释。

 175. **ConPTY 的启动握手与"Enter 是 CR"**（本会话实测，**已修**）：Windows 上本地终端在最初 20 秒里
      **一个字节都读不到**，两条原因都不在 app 的业务逻辑里 ——
      ① **ConPTY 启动时先问一次光标位置**（`ESC[6n`），**没等到回答之前不出任何输出**（cmd.exe 的版本横幅
      与提示符都压着）。真实界面里回答它的是 xterm（xterm.js 的 `deviceStatus` 对 `CSI 6 n` 回
      `ESC[<row>;<col>R`），所以"界面上看得到"这条路本来就是好的；而 `session_channel` 的 raw 通道探针是
      **哑的**（只统计字节），于是它什么都等不到。处置：只在 Windows 上补一条记录（`write_session` 发
      `ESC[1;1R`），并在那里写明"POSIX 的 PTY 没有这个握手，往那边写就是往 shell 的输入里塞转义序列"。
      ② **行尾必须是 CR**：POSIX 的行规程用 `ICRNL` 把 CR 折成 NL，所以送 LF 也能提交；**ConPTY 只认 CR**
      —— 送 LF 时命令行停在屏幕上不动（实测：`echo X` + LF 只回显、不执行，改 CR 立刻执行）。处置：E2E 的
      "敲一行"辅助函数改成**由函数补 CR**（`type_line` 的 `line` 参数因此不带行尾），15 处调用点去掉自己写的
      `\n`。⚠️ 教训：**"终端线上的一组字节"在三个平台上不是同一件事**，这类差异不会在类型检查或
      `just ready` 里露头，只在真的驱动一次终端时出现。

 177. **CI 的 Windows runner 上暴露出来的一类差异**（本会话实测，**已修**）：一次真的 CI 运行
      （`36226121050`）把本机的全绿换成了四处红；第二次运行（`36228577977`）只剩 `portable` 一处。
      逐条如下 ——
      - **WebView2 的用户数据目录被上一份实例握着**（与本次改动无关，**两次运行都红**）：结束这一份 app 之后
        它的 WebView2 还握着 `%LOCALAPPDATA%\fans.cyrene.akasha-terminal` 一小会儿，于是**下一份** app
        起不来 webview：`failed to create webview: WebView2 error ... "The requested resource is in use."`
        （第二段的 `exit_residue` 不看界面所以照过；第三段的 `portable::data_survives_the_move` 等到
        60 秒超时）。处置两条：收 app 时带 `/T`（连同子进程一起收），**并在收完之后等那个目录放开** ——
        判据取"它能改名"（有活进程握着时改名失败），本机上通常立刻就能改，代价接近零。
        ⚠️ 只带 `/T` 不够（第二次运行仍红在同一处）：握着它的不一定还挂在 app 的进程树上。
      - **`windows_ports` 在一台"有端口、注册表里却没有描述"的机器上必红**（与本次改动无关）：
        runner 上枚举出 1 条 `{"kind":"unknown","path":"COM2"}` —— 固件留下的端口在 `SERIALCOMM` 里、
        `Enum` 下没有对应的 PnP 项，app 无从描述它（`unknown` 是**如实**的）。原判据把"这台机器本来
        就没有描述"与"注册表里有、而枚举没带出来"当成同一件事，只有后者才是链断。处置：这一档改成
        显式跳过并把枚举结果原样输出；真正的负对照留给有真实串口设备的主机（plan 0803 的那一层）。
      - **`terminal_render` / `single_instance` 的终端驱动**：两条用例都只做一件事 —— 把一行敲进**界面
        上的**终端等结果。它们红在"屏幕上始终没有那串字"。两条线索合起来指向**输入路径**：句柄到手
        之前，xterm 生成的数据（最要紧的是它给 ConPTY 的 DSR 回答）被 `onInput` 直接丢弃，于是那条
        会话永远不出提示符。处置两条：**①** 会话开起来之前把 xterm 生成的数据按序攒下、开了再补发
        （`src/terminal/attach.ts`，上限 64 KiB）；**②** 同时修掉 #175 留下的那个自相矛盾 —— `echo_marker` 把
        求值结果写进了**文件名**，而命令行会被回显，光靠回显就能让断言命中（本机当时就是这样"过"的）；
        现在只让**文件内容**带结果，文件名只用不构成结果的那一部分。失败信息也跟着补上屏幕原文，
        下一次红能直接看出"没送到 / 没提示符 / 没执行"。⚠️ **第二次运行（`36228577977`）里这两条都过了**
        —— 处置有效（同一轮里 `windows_ports` 按新口径显式跳过）。**第三次运行（`36231407751`）Windows 的 E2E 全绿**，
        `portable` 那一处也随"等目录放开"这一步消失。
      - Linux 的 `vault_unlock` 与上一次运行**同一处、同一句话**（`读不到 app 的 /proc/<pid>/status`），
        当时记在「进行中」，现已定位并修掉 —— 那是这条判据自己的缺陷（问题 #180）；
         第三次运行另有一次 `sftp_host_to_host` 的凭据超时
        （`right 这一侧连接失败：认证失败：… 上没有可用的方式`）—— 该目标前两轮都过、文件未被本次
        改动触碰，记为偶发。

 178. **vendored OpenSSL 的 `LNK4099` 把 Windows 的构建日志冲散**（本会话实测，**已修**）：
      `openssl-src` 在 MSVC 下用 `/Zi /Fdossl_static.pdb` 编译，而那份 PDB 不随
      `libopenssl_sys-*.rlib` 发到 `deps/` 下 —— 链接器于是对**每一个** .obj 各输出一行
      `warning LNK4099: PDB 'ossl_static.pdb' was not found ...`。本机实测：链接一个测试目标
      **1618 行**，全量 `cargo test --no-run`（65 个目标）上万行，把真正的报错冲到看不见。
      处置：`src-tauri/build.rs` 在 Windows + MSVC 上补一条链接器开关 `/IGNORE:4099`（复测：
      65 个目标、**0 行**）。另一条路是 `.cargo/config.toml` 里的 `rustflags`，**没有**选它：
      那会让整棵依赖树重编（含从源码构建的 OpenSSL，而本机只有 MSYS 的 perl，重建当场失败在
      `perl reported failure with exit code: 2`），而 `rustc-link-arg` 只影响本包自己的
      bin / test / example 链接。⚠️ 仍有的那一行 `linker stdout: 正在创建库 ...` 是 cdylib 链接的
      正常输出（rustc 的 `linker_messages` lint），一行而已，不关。顺带：`no-println` 规则给
      `**/build.rs` 开了豁免 —— 构建脚本的 stdout 就是它的 API（cargo 从那里读指令）。

 179. **前端把每一次隧道状态变化记了两次，`tunnel_reconnect` 因此偶发红**（CI 运行 `36234483861` 的
      macOS 格实测，**已修**）：`subscribeTunnelStates` 原来**每订阅一次就 `listen` 一次**，并把事件
      写进 `window.__akashaTunnels.events`（测试接口），而退订是**异步**的（`listen` 返回 Promise，
      `void stop.then((unlisten) => unlisten())`）。面板被卸下再挂上时（`open_tunnel_panel` 正是
      "先关再开"），或 StrictMode 把 effect 走两遍时，上一份订阅可能还没摘掉 —— 同一条事件于是被记两遍。
      ⚠️ 那次红的形状是**成对**而不是"事件变多"：序列
      `[reconnecting, reconnecting, connecting, connecting, …, failed, failed]`、`attempts` 拿到
      `[1,1,2,2,3,3]`（期望 `[1,2,3]`）。同一份 app 里前一条隧道目标（`tunnel_state`）读到的
      `状态数` 仍是 **2**（不是 4）—— 所以是**订阅漏摘、监听器多了一个**，不是后端重复发事件。
      哪一次挂载漏掉了退订没有定位（本机 Windows 上三轮都过，复现不出来），因此处置不追那次时序：
      改成 `session.ts` 的 `ensureListening` 那个形状 —— **一条常驻监听**先记一次日志、再分发给当前
      订阅者（`Set`），退订只把自己从名单里摘掉。事件日志因此与"订阅过几次"无关。

 180. **Linux 的 `vault_unlock` 必红：三次读数被折成一个只填了第一个槽位的元组**（CI 运行
      `36234483861` 的 Linux 格实测，**已修**）：`3d6ebd7` 把"解锁前 / 解锁中 / 锁定后"三次
      `locked_kb(pid)` 读数折成 `locked_marks(port) -> (Option<u64>, Option<u64>, Option<u64>)`，
      而它只填第一个槽位（`(Some(before), None, None)`），调用点却分别取 `.0` / `.1` / `.2`
      —— 于是 `.1` / `.2` 恒为 `None`，那句 `expect("读不到 app 的 /proc/<pid>/status")` 在
      Linux 上必然 panic。⚠️ 它**只**在 Linux 上红：那三条断言整段在 `#[cfg(target_os = "linux")]`
      里，macOS / Windows 的格子连这段都不编译，而本机没有 Linux 主机 —— 三轮本机 E2E 全绿也看不见它。
      判据是 CI 日志的逐字读数：`VmLck` 打印了 **0 / 160 / 0** 之后紧跟 panic —— 读数本身是好的，
      丢掉后两次的是那个包装函数。处置：恢复成"一次调用取一次读数"的 `locked_now(port)`，三个调用点
      各取一次，并让每次读数各输出一行（`进程: pid=… VmLck=…`）。
      验证：本机把该文件里的 `target_os = "linux"` 临时翻成 `"windows"` 执行一次
      `cargo check --test vault_unlock`（退出码 0，那段 cfg 分支编译得过）后还原；
      **它是否真的在 Linux 上转绿由 CI 的 Linux 格给出**（本机没有 Linux 主机）。

