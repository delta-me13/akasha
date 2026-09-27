# 已完成的 ROADMAP 条目（归档）

> 由 `just docs-archive` 从 `ROADMAP.md` 移来（规则见 `AGENTS.md` §8.3），**只移动、不改写**。
> 未完成的条目仍在 [`ROADMAP.md`](../../ROADMAP.md)；plan 索引见 [`../plans/README.md`](../plans/README.md)。

## 阶段 0 — 地基（可编译、可验证、可交接）
- [x] 工具链与依赖就位（`just check` / `just lint` 退出码 0）
- [x] 依赖门禁（`just deny-offline` → bans / licenses / sources all ok）
- [x] 规范与文档体系（`AGENTS.md` + `ROADMAP.md` + `docs/`）
- [x] 文档体系随范围扩大同步（`docs/scope.md` 与 `docs/portable.md` 登记进
      `AGENTS.md` §8；命名约定写在 §3.1）
- [x] ADR 队列收敛为 3 份（见 [`docs/adr/README.md`](./docs/adr/README.md)）
- [x] ADR-0001 定案，决策二裁定见其 §0.3
      验收：`docs/adr/0001` 的状态为"已定案"

## 阶段 1 — 分层与平台矩阵
- [x] 落地 ADR-0001 决策一：根 workspace
      验收：仓库根成为 workspace、`src-tauri` 降为成员之一，且门禁仍全部通过
      → [plan 0101](./docs/plans/archive/0101-root-workspace.md)
- [x] crates 改为 `src-tauri/src` 下的模块（ADR-0008）
      验收：仓库内不再有成员 manifest 与 `mod.rs`，纯逻辑不依赖 Tauri 由路径规则接手
      → [plan 0109](./docs/plans/archive/0109-crates-to-modules.md)
- [x] Rust 成员收进 `src-tauri/`（取代上面 0101 的根 workspace 布局，见 ADR-0004）
      验收：根目录无 manifest / 成员 / target，门禁全部通过，改成员仍触发重编译
      → [plan 0106](./docs/plans/archive/0106-workspace-under-src-tauri.md)
- [x] Windows 上受保护页与 SQLCipher 不再争抢同一份额度（问题 #167；ADR-0009）
      验收：Windows 上连续建 32 个 16 KiB 受保护页全部成功，且导出 / 还原那两条用例不再竞态失败
      → [plan 0110](./docs/plans/0110-windows-locked-page-budget.md)
- [x] Windows 上原生执行 `just test` 不再失败（问题 #168：SOCKS5 拒绝后的 RST、路径的 verbatim 前缀）
      验收：Windows 上原生执行完整一遍 workspace 测试，全部通过
      → [plan 0111](./docs/plans/0111-windows-native-test-failures.md)
- [x] `src-tauri/crates/akasha-core` 骨架：`Session` 模型（**必须先于任何后端**）
      验收：单测覆盖 `SessionId` 分配、关闭一个 `Session` 不影响另一个
      → [plan 0103](./docs/plans/archive/0103-core-session-model.md)
- [x] 迁移后复测开发循环：改 `src-tauri/crates/` 下的文件仍触发重编译与重启
      验收：改一个 `src-tauri/crates/` 文件后 app 自动重启
      → [plan 0104](./docs/plans/archive/0104-dev-loop-retest.md)
- [x] `src-tauri/crates/akasha-pty`：通用 `Transport` trait + `portable-pty` 实现
      验收：用假实现覆盖 spawn / write / shutdown 的单测通过
      → [plan 0105](./docs/plans/archive/0105-pty-transport-trait.md)
- [x] macOS 上的进程级判据真实化（`exit_residue` / `session_watchdog` 不再空过）
      验收：macOS 上那两条用例有非空读数，撤掉会话级回收后必红
      → [plan 0112](./docs/plans/0112-macos-liveness-criteria.md)

## 阶段 2 — 端到端最小终端
- [x] `Transport` 输出合批（≥16ms 或 ≥64KiB）
      验收：合批边界有单测断言；有吞吐基线数字
      → [plan 0201](./docs/plans/archive/0201-output-batching.md)
- [x] IPC 二进制通道（输出走 raw 字节，不是 JSON 数组）
      验收：10 MB 输出完整到达前端且无错误；该路径被 `no-string-pty-channel` 守住
      → [plan 0202](./docs/plans/archive/0202-ipc-binary-channel.md)
- [x] 前端 xterm + WebGL 渲染，字节流不进 React state
      验收：终端由 canvas 渲染；无 per-chunk 组件重渲染
      → [plan 0203](./docs/plans/archive/0203-xterm-webgl-render.md)
- [x] 真正退出零残留（能执行代码的两条路径）：关窗口退出 / panic 都显式回收会话
      验收：两条路径退出后（含忽略 SIGHUP 的子进程）零残留
      → [plan 0204](./docs/plans/archive/0204-exit-zero-residue.md)
- [x] 被 SIGKILL 的退出路径也零残留（`tauri dev` 重编译重启 / `kill -9` / `kill -TERM`）
      验收：三条路径之后，上一轮会话里忽略 SIGHUP 的子进程也一个不剩
      → [plan 0205](./docs/plans/archive/0205-sigkill-exit-residue.md)

## 阶段 3 — 托盘与应用生命周期
- [x] 托盘图标 + 菜单（显示/隐藏、隧道列表与状态、退出）
      验收：托盘注册进宿主托盘、菜单项可点、从托盘退出后零残留
      → [plan 0301](./docs/plans/archive/0301-tray-icon-menu.md)
- [x] 隐藏而非销毁窗口（关闭窗口后进程仍在，会话与终端原样存活）
      验收：隐藏再显示后终端回滚缓冲仍在，且子进程仍在（**这是预期**，不是泄漏）
      → [plan 0302](./docs/plans/archive/0302-hide-not-destroy.md)
- [x] 关闭行为可配置（收进托盘 / 直接退出）
      验收：切成"直接退出"后关闭窗口即退出且零残留；切回"收进托盘"后关闭窗口不退出
      → [plan 0303](./docs/plans/archive/0303-close-behavior-config.md)
- [x] 单实例（第二个实例唤起已有窗口，而不是各自运行一套）
      验收：连续启动两次只有一个进程、一条隧道
      → [plan 0304](./docs/plans/archive/0304-single-instance.md)
- [x] 关闭终端标签页 = 立刻丢弃该 Session（只有三大终端 local / ssh / serial 有 ×）
      验收：点 × 后该会话的进程消失、无需二次确认、其它标签页不受影响；转发 / 密码库 /
      文件传输是仅渲染的视图（无 ×），关前端不影响后端 → [plan 0305](./docs/plans/archive/0305-tab-close-discards-session.md)
- [x] 会话自己结束（终端里输入 exit）= 回收它 + 关闭那个标签页（与 0305 反方向）
      验收：输入 exit 后标签页自己消失、进程零残留、app 不退出（标签页 ⇔ 会话同生命期）
      → [plan 0306](./docs/plans/archive/0306-session-ended-closes-tab.md)

## 阶段 4 — 存储与凭据池
- [x] ADR-0002 进入实现中（改动存储代码之前；同时纳入 Bitwarden 的机密来源）
      验收：状态为「实现中」，且补齐了 `scope.md` 里"还没有值"的那几项
      → [plan 0400](./docs/plans/archive/0400-adr-0002-secret-storage.md)
- [x] `rusqlite` + SQLCipher（`bundled-sqlcipher-vendored-openssl`）
      验收：用错误口令打不开库；`.db` 文件里搜不到明文密钥
      → [plan 0401](./docs/plans/archive/0401-sqlcipher-open.md)
- [x] 口令 → KDF → 库密钥（**不依赖 OS keychain**，见 `scope.md` §1）
      验收：无任何 `keyring` 类依赖
      → [plan 0402](./docs/plans/archive/0402-passphrase-kdf.md)
- [x] 口令的内存防护：受保护页（锁定 / 静止不可读 / 不进 core dump / fork 清零）
      验收：进程 VmLck 涨；那页在 smaps 里没有任何权限；VmFlags 含 dd 与 wf
      → [plan 0406](./docs/plans/archive/0406-memsafe-passphrase-page.md)
- [x] 四类池的 CRUD：密钥 / ssh 配置 / serial 配置 / 端口转发规则
      验收：各自的 round-trip 单测通过；**库里不存绝对路径**（P2）
      → [plan 0403](./docs/plans/archive/0403-pools-crud.md)
- [x] 解锁与锁定的生命周期：谁持有解好的库、口令从哪来、锁定时抹掉什么
      验收：解锁 → 读一次池 → 锁定之后进程里不留机密（`VmLck` 回落到解锁前）
      → [plan 0407](./docs/plans/archive/0407-unlock-lifecycle.md)
- [x] dump 与导出（可选加密；明文导出必须二次确认）
      验收：加密导出可在另一目录导入还原；明文导出路径有显式确认门槛
      → [plan 0404](./docs/plans/archive/0404-dump-export.md)
- [x] 可搬迁性验证（见 [`docs/portable.md`](./docs/portable.md)）
      验收：移动整个文件夹后重启，原有主机/密钥/规则都在（只验证"能开"不算过）
      → [plan 0405](./docs/plans/archive/0405-portability-verify.md)
- [x] ADR-0002 转「已定案」（阶段 4 落地完成之后）
      验收：状态为「已定案」，且 §10 修订记录里每次改动都有理由
- [x] 可搬迁性的平台口径分档（macOS 不做便携；Linux 的分发形态待定）
      验收：文档与实现口径一致，macOS 上不再承诺 `.app` 旁的便携目录
      → [plan 0408](./docs/plans/0408-portable-platform-scope.md)

## 阶段 5 — SSH 栈（`russh`）
- [x] ADR-0003（SSH 栈与资源模型）定稿，状态置「实现中」
      验收：状态为「实现中」，且有可核对的 `russh` 版本结论
      → [plan 0501](./docs/plans/archive/0501-adr-0003-ssh-stack.md)
- [x] `src-tauri/crates/akasha-ssh`：连接 + 认证（密钥池 / agent / 内存凭据缓存）
      验收：同主机开三个 Session 只发起一次凭据询问（`scope.md` §2.2）
      → [plan 0502](./docs/plans/archive/0502-ssh-connect-auth.md)
- [x] known_hosts 校验与缓存
      验收：主机密钥变化时拒绝连接并提示（不静默接受）；未知主机密钥经用户确认后写入缓存
      → [plan 0503](./docs/plans/archive/0503-known-hosts.md)
- [x] SSH 接进 IPC / 前端（界面选主机 → 连接 → 双向流；凭据与未知主机密钥的往返）
      验收：真实 app 上开一个 SSH 会话：字节双向流动、凭据只询问一次、关闭标签页零残留
      → [plan 0504](./docs/plans/archive/0504-ssh-into-ipc-frontend.md)
- [x] `direct-tcpip` 原语（本阶段先用于跳板，之后三处复用）
      验收：ProxyJump 可连通仅对跳板机可见的目标
      → [plan 0505](./docs/plans/archive/0505-direct-tcpip-primitive.md)
- [x] `~/.ssh/config` 受限子集导入（`Match` / `Include` 显式报错）
      验收：含 `Match` 的配置产生明确报错，而非静默误解析
      → [plan 0506](./docs/plans/archive/0506-ssh-config-subset-import.md)
- [x] 主机指纹在前端可视与可删除（添加与修改只在连接过程中触发）
      验收：面板里看得到、删得掉，删除之后下一次连接重新询问
      → [plan 0507](./docs/plans/0507-known-hosts-panel.md)

## 阶段 6 — SSH 端口转发
- [x] 隧道实体（独立于终端 `Session`）+ 状态机
      验收：五态可观测；状态变化发事件
      → [plan 0601](./docs/plans/archive/0601-tunnel-entity-state-machine.md)
- [x] 本地转发 `-L`（复用阶段 5 的 `direct-tcpip`）
      验收：转发端口可访问远端服务
      → [plan 0602](./docs/plans/archive/0602-local-forward.md)
- [x] 动态转发 `-D`（本地 SOCKS5 服务端）
      验收：配置 SOCKS5 代理后能访问远端网络
      → [plan 0603](./docs/plans/archive/0603-dynamic-forward-socks5.md)
- [x] 远程转发 `-R`（`tcpip-forward` + `forwarded-tcpip`，另一套机制）
      验收：远端监听端口可回连到本机服务
      → [plan 0604](./docs/plans/archive/0604-remote-forward.md)
- [x] 断线重连：3 次 + 指数退避，然后标记失败
      验收：拔网线后进入"重连中"，耗尽次数后变"失败"且托盘可见；可手动重试
      → [plan 0605](./docs/plans/archive/0605-reconnect-backoff.md)
- [x] 关闭 `Session` 立刻断连，并中止该 `Session` 的重连循环
      验收：关闭转发 Session 后连接数与重连任务数都归零
      → [plan 0606](./docs/plans/archive/0606-close-session-teardown.md)
- [x] ADR-0003 转「已定案」（阶段 6 落地完成之后）
      验收：状态为「已定案」，且 §14 修订记录里每次改动都有理由

## 阶段 7 — SFTP
- [x] 双栏界面骨架 + 两侧独立选主机（**不需要先开终端 Session**）
      验收：直接打开 SFTP 即可用，无终端依赖
      → [plan 0701](./docs/plans/archive/0701-sftp-dual-pane.md)
- [x] local ↔ host 双向；临时名 + 原子重命名落盘
      验收：中断传输后目标目录里**没有**看似完整的文件
      → [plan 0702](./docs/plans/archive/0702-transfer-atomic-rename.md)
- [x] host ↔ host：优先 B 档（`direct-tcpip`），失败回退 A 档（内存 relay）
      验收：A 无法直连 B 时自动走 A 档；两档均不落盘
      → [plan 0703](./docs/plans/archive/0703-host-to-host-topology.md)
- [x] 并发 in-flight 请求（pipelining）
      验收：大量小文件的吞吐显著优于串行请求
      → [plan 0704](./docs/plans/archive/0704-pipelining.md)

## 阶段 8 — serial
- [x] `src-tauri/crates/akasha-serial`，`libudev` 走 Linux-only cargo feature
      验收：Windows / macOS 构建不链接 libudev
      → [plan 0801](./docs/plans/archive/0801-serial-crate-libudev.md)
- [x] 端口枚举与连接参数（波特率/数据位/停止位/校验/流控）
      验收：枚举在本机列出真实端口；参数错误时给出可读报错
      → [plan 0802](./docs/plans/archive/0802-serial-enumeration-params.md)

## 阶段 9 — Bitwarden 导入
- [x] `bw` 的获取与前置检查：二进制（宿主机的 `bw` / 运行时下载的 OSS 变体）与 CLI 状态目录两轴独立可切，默认都取宿主机那一档
      验收：宿主机没有 `bw` 时报出这件事并给出下载动作；下载之后能读到它的版本与变体
      → [plan 0902](./docs/plans/archive/0902-bw-acquire-and-preflight.md)
- [x] 登录 / 解锁 / 锁定接进前端（含自托管）：服务器地址可设，会话状态取自 CLI 自己的状态，session token 只在内存
      验收：设自托管地址后登录到未解锁态、解锁到已解锁态、锁定后内存里不再有 token；三态与 CLI 自报一致
      → [plan 0905](./docs/plans/archive/0905-bw-login-session-ui.md)
- [x] 只读导入 SSH key 条目（`sshKey.privateKey`）：导入落进密钥池并记一行来历（上游 id / `revisionDate` / `fingerprint`）
      验收：导入后可用该密钥建立 SSH 连接（`IdentityFile` 的文件名与钥匙名相同时接上那一行）
      → [plan 0903](./docs/plans/archive/0903-bw-readonly-import.md)
- [x] 离线缓存：私钥离线自检用 `fingerprint`（不起进程、不联网），联网刷新用 `revisionDate`
      验收：断网时能校验缓存完整性；联网且 `revisionDate` 变化时提示刷新
      → [plan 0904](./docs/plans/archive/0904-bw-offline-cache.md)

## 阶段 11 — 串口接入 app
- [x] 串口 `Session` 接入 app（命令 + 注册表 + 标签页 + 关闭与回收）
      验收：真实 app 上打开一个串口会话并双向传字节；关闭标签页后 `live` / `registered` 归零
      → [plan 1101](./docs/plans/archive/1101-serial-session-ipc.md)
- [x] 端口枚举与参数接进界面（列端口 + 手动路径兜底 + 池行取值 + 可读报错）
      验收：界面上看到本机枚举结果并据此（或手输路径）打开；取值越界时显示字段与取值
      → [plan 1102](./docs/plans/archive/1102-serial-ports-ui.md)
- [x] 设备消失时串口会话以可读原因结束，标签页随之关闭
      验收：把测试用的 PTY 主端关闭（等价于拔掉设备）时会话结束、没有残留注册
      → [plan 1103](./docs/plans/archive/1103-serial-device-gone.md)
