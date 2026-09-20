# ROADMAP

> **回答"我们去哪"** —— "下一步做什么"只在这一个文件里，不散落在 `AGENTS.md` 或 `docs/STATUS.md`。
> 这里**只勾复选框**，不写细节：具体步骤在 [`docs/plans/`](./docs/plans/)，
> 当前到哪一步在 [`docs/STATUS.md`](./docs/STATUS.md)。
>
> 每个条目必须有**可执行的验收标准**。写不出验收标准的条目不许进入本文件。
>
> **能力清单在 [`docs/scope.md`](./docs/scope.md)** —— 本文件的阶段从它派生。
> **每个条目末尾挂一个 plan 指针**：怎么做、可粘贴的验收命令都在那里；
> plan 用阶段块号（`TTxx`），完整索引见 [`docs/plans/README.md`](./docs/plans/README.md)。

图例：`[x]` 已完成 · `[ ]` 未开始 · `[~]` 进行中 · `[!]` 被阻塞

---

## 阶段 0 — 地基（可编译、可验证、可交接）

目标：任何时刻 `just ready` 通过，规范/现状/进度各有唯一来源。

**已完成 —— 按约定不回填 plan**（记录在提交历史与 `STATUS.md`）。

- [x] 工具链与依赖就位（`just check` / `just lint` 退出码 0）
- [x] 依赖门禁（`just deny-offline` → bans / licenses / sources all ok）
- [x] 规范与文档体系（`AGENTS.md` + `ROADMAP.md` + `docs/`）
- [x] **文档体系随范围扩大同步**（`docs/scope.md` 与 `docs/portable.md` 已登记进
      `AGENTS.md` §8；命名约定已写入 §3.1）
- [x] **ADR 队列收敛为 3 份**（见 [`docs/adr/README.md`](./docs/adr/README.md)）
- [~] **CI 通过** —— 工作流已重写（每平台一条流水线：检查 → E2E 两段串行，六个 job）
      验收：CI 上六个 job 通过 —— Linux（含 E2E）与 macOS 的类型检查已通过；Windows 的类型检查与两处
      平台问题已修，结论待下一次运行
- [x] **ADR-0001 定案** —— 已定案（2026-09-11），决策二裁定见其 §0.3
      验收：`docs/adr/0001` 状态已改为"已定案" ✓

---

## 阶段 1 — 分层与平台矩阵

目标：后端分层落地，且跨平台差异从一开始就被验证。

- [x] **落地 ADR-0001 决策一：根 workspace**
      验收：仓库根成为 workspace、`src-tauri` 降为成员之一，且门禁仍全部通过
      → [plan 0101](./docs/plans/archive/0101-root-workspace.md)
- [x] **crates 改为 `src-tauri/src` 下的模块**（ADR-0008；2026-09-19 完成，plan 0109 已归档）
      验收：仓库内不再有成员 manifest 与 `mod.rs`，纯逻辑不依赖 Tauri 由路径规则接手
      → [plan 0109](./docs/plans/archive/0109-crates-to-modules.md)
- [x] **Rust 成员收进 `src-tauri/`**（取代上面 0101 的根 workspace 布局，见 ADR-0004）
      验收：根目录无 manifest / 成员 / target，门禁全部通过，改成员仍触发重编译
      → [plan 0106](./docs/plans/archive/0106-workspace-under-src-tauri.md)
- [~] **CI 平台矩阵**（Linux + Windows + macOS，GitHub Actions 一份）
      验收：三平台都能通过类型检查；Linux 另执行完整门禁与 E2E
      → [plan 0102](./docs/plans/0102-ci-platform-matrix.md)
- [~] **E2E 入口可实际运行**：`just test-e2e` 自包含（启动 app → 运行全部 E2E 目标 → 收尾）
      验收：没有 app 在运行时它也退出码 0；新增的 E2E 目标不接入即失败；CI 上三平台已实际执行（Linux 通过，另两格见 plan 0102）
      → [plan 0107](./docs/plans/0107-e2e-entry.md)
- [~] **Windows 目标的类型检查真的通过**（问题 #149 的编译面已修；#160 是同类的第二处）
      验收：Windows 目标上的类型检查退出码 0（本地只能核对不依赖 C 工具链的三个成员，其余交给 CI）
      → [plan 0108](./docs/plans/0108-windows-type-check.md)
- [x] **Windows 上受保护页与 SQLCipher 不再争抢同一份额度**（问题 #167；ADR-0009）
      验收：Windows 上连续建 32 个 16 KiB 受保护页全部成功，且导出 / 还原那两条用例不再竞态失败
      → [plan 0110](./docs/plans/0110-windows-locked-page-budget.md)
- [x] **Windows 上原生执行 `just test` 不再有红**（问题 #168：SOCKS5 拒绝后的 RST、路径的 verbatim 前缀）
      验收：Windows 上原生执行完整一遍 workspace 测试，全部通过
      → [plan 0111](./docs/plans/0111-windows-native-test-failures.md)
- [x] `src-tauri/crates/akasha-core` 骨架：**`Session` 模型**（**必须先于任何后端**）
      验收：单测覆盖 `SessionId` 分配、关闭一个 `Session` 不影响另一个
      → [plan 0103](./docs/plans/archive/0103-core-session-model.md)
- [x] **迁移后复测开发循环**：改 `src-tauri/crates/` 下的文件仍触发重编译与重启
      验收：改一个 `src-tauri/crates/` 文件后 app 自动重启（否则开发循环静默失效）
      → [plan 0104](./docs/plans/archive/0104-dev-loop-retest.md)
- [x] `src-tauri/crates/akasha-pty`：通用 `Transport` trait + `portable-pty` 实现
      验收：用假实现覆盖 spawn / write / shutdown 的单测通过
      → [plan 0105](./docs/plans/archive/0105-pty-transport-trait.md)

---

## 阶段 2 — 端到端最小终端

目标：可打开一个 shell、执行命令、看到输出，且**真正退出后**不残留进程。

- [x] `Transport` 输出合批（≥16ms 或 ≥64KiB）
      验收：合批边界有单测断言；有吞吐基线数字
      → [plan 0201](./docs/plans/archive/0201-output-batching.md)
- [x] IPC 二进制通道（输出走 **raw 字节**，不是 JSON 数组）
      验收：10 MB 输出完整到达前端且无错误；该路径被 `no-string-pty-channel` 守住
      → [plan 0202](./docs/plans/archive/0202-ipc-binary-channel.md)
- [x] 前端 xterm + WebGL 渲染，字节流不进 React state
      验收：终端由 canvas 渲染；无 per-chunk 组件重渲染
      → [plan 0203](./docs/plans/archive/0203-xterm-webgl-render.md)
- [x] **真正退出零残留（能执行代码的两条路径）**：关窗口退出 / panic 都显式回收会话
      验收：两条路径退出后（含**忽略 SIGHUP** 的子进程）零残留 —— E2E 实测
      → [plan 0204](./docs/plans/archive/0204-exit-zero-residue.md)
- [x] **被 SIGKILL 的退出路径也零残留**（`tauri dev` 重编译重启 / `kill -9` / `kill -TERM`）
      验收：三条路径之后，上一轮会话里**忽略 SIGHUP 的子进程**也一个不剩 —— 实测 + 集成用例
      → [plan 0205](./docs/plans/archive/0205-sigkill-exit-residue.md)

---

## 阶段 3 — 托盘与应用生命周期

目标：关闭窗口不退出；隧道与终端在窗口隐藏期间存活。

- [x] 托盘图标 + 菜单（显示/隐藏、隧道列表与状态、退出）
      验收：托盘注册进宿主托盘、菜单项可点、从托盘退出后零残留 —— Linux 实测
      → [plan 0301](./docs/plans/archive/0301-tray-icon-menu.md)
- [x] **隐藏而非销毁**窗口（关闭窗口后进程仍在，会话与终端原样存活）
      验收：隐藏再显示后终端回滚缓冲仍在，且子进程仍在（**这是预期**，不是泄漏）
      → [plan 0302](./docs/plans/archive/0302-hide-not-destroy.md)
- [x] 关闭行为可配置（收进托盘 / 直接退出）
      验收：切成"直接退出"后关闭窗口即退出且零残留；切回"收进托盘"后关闭窗口不退出
      → [plan 0303](./docs/plans/archive/0303-close-behavior-config.md)
- [x] 单实例（第二个实例唤起已有窗口，而不是各自运行一套）
      验收：连续启动两次只有一个进程、一条隧道
      → [plan 0304](./docs/plans/archive/0304-single-instance.md)
- [x] **关闭终端标签页 = 立刻丢弃该 Session**（只有三大终端 local / ssh / serial 有 ×）
      验收：点 × 后该会话的进程消失、无需二次确认、其它标签页不受影响；转发 / 密码库 /
      文件传输是**仅渲染**的视图（无 ×），关前端不影响后端 → [plan 0305](./docs/plans/archive/0305-tab-close-discards-session.md)
- [x] **会话自己结束（终端里输入 exit）= 回收它 + 关闭那个标签页**（与 0305 反方向）
      验收：输入 exit 后标签页自己消失、进程零残留、app 不退出（标签页 ⇔ 会话同生命期）
      → [plan 0306](./docs/plans/archive/0306-session-ended-closes-tab.md)

---

## 阶段 4 — 存储与凭据池

目标：四类池可增删改查；库加密；可导出。

- [x] **ADR-0002 进入实现中**（改动存储代码之前；同时纳入 Bitwarden 的机密来源）
      验收：状态为「实现中」，且补齐了 `scope.md` 里"还没有值"的那几项
      → [plan 0400](./docs/plans/archive/0400-adr-0002-secret-storage.md)
- [x] `rusqlite` + **SQLCipher**（`bundled-sqlcipher-vendored-openssl`）
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
- [x] **解锁与锁定的生命周期**：谁持有解好的库、口令从哪来、锁定时抹掉什么
      验收：解锁 → 读一次池 → 锁定之后进程里不留机密（`VmLck` 回落到解锁前）
      → [plan 0407](./docs/plans/archive/0407-unlock-lifecycle.md)
- [x] dump 与导出（可选加密；明文导出必须二次确认）
      验收：加密导出可在另一目录导入还原；明文导出路径有显式确认门槛
      → [plan 0404](./docs/plans/archive/0404-dump-export.md)
- [x] **可搬迁性验证**（见 [`docs/portable.md`](./docs/portable.md)）
      验收：移动整个文件夹后重启，**原有主机/密钥/规则都在**（只验证"能开"不算过）
      → [plan 0405](./docs/plans/archive/0405-portability-verify.md)
- [x] **ADR-0002 转「已定案」**（阶段 4 落地完成之后）
      验收：状态为「已定案」，且 §10 修订记录里每次改动都有理由

---

## 阶段 5 — SSH 栈（`russh`）

目标：纯 Rust SSH 实现，不调用系统 `ssh`。

- [x] **ADR-0003（SSH 栈与资源模型）定稿，状态置「实现中」**
      验收：状态为「实现中」，且有可核对的 `russh` 版本结论
      → [plan 0501](./docs/plans/archive/0501-adr-0003-ssh-stack.md)
- [x] `src-tauri/crates/akasha-ssh`：连接 + 认证（密钥池 / agent / 内存凭据缓存）
      验收：同主机开三个 Session 只发起一次凭据询问（`scope.md` §2.2）
      → [plan 0502](./docs/plans/archive/0502-ssh-connect-auth.md)
- [x] known_hosts 校验与缓存
      验收：主机密钥变化时拒绝连接并提示（不静默接受）；未知主机密钥经用户确认后写入缓存
      → [plan 0503](./docs/plans/archive/0503-known-hosts.md)
- [x] **SSH 接进 IPC / 前端**（界面选主机 → 连接 → 双向流；凭据与未知主机密钥的往返）
      验收：真实 app 上开一个 SSH 会话 —— 字节双向流动、凭据只询问一次、关闭标签页零残留
      → [plan 0504](./docs/plans/archive/0504-ssh-into-ipc-frontend.md)
- [x] **`direct-tcpip` 原语**（本阶段先用于跳板，之后三处复用）
      验收：ProxyJump 可连通仅对跳板机可见的目标
      → [plan 0505](./docs/plans/archive/0505-direct-tcpip-primitive.md)
- [x] `~/.ssh/config` **受限子集**导入（`Match` / `Include` 显式报错）
      验收：含 `Match` 的配置产生明确报错，而非静默误解析
      → [plan 0506](./docs/plans/archive/0506-ssh-config-subset-import.md)

---

## 阶段 6 — SSH 端口转发

目标：本地/远程/动态三类转发，独立 Session，失败可见。

- [x] 隧道实体（独立于终端 `Session`）+ 状态机
      验收：五态可观测；状态变化发事件
      → [plan 0601](./docs/plans/archive/0601-tunnel-entity-state-machine.md)
- [x] 本地转发 `-L`（复用阶段 5 的 `direct-tcpip`）
      验收：转发端口可访问远端服务
      → [plan 0602](./docs/plans/archive/0602-local-forward.md)
- [x] 动态转发 `-D`（本地 SOCKS5 服务端）
      验收：配置 SOCKS5 代理后能访问远端网络
      → [plan 0603](./docs/plans/archive/0603-dynamic-forward-socks5.md)
- [x] 远程转发 `-R`（`tcpip-forward` + `forwarded-tcpip`，**另一套机制**）
      验收：远端监听端口可回连到本机服务
      → [plan 0604](./docs/plans/archive/0604-remote-forward.md)
- [x] 断线重连：**3 次 + 指数退避**，然后标记失败
      验收：拔网线后进入"重连中"，耗尽次数后变"失败"**且托盘可见**；可手动重试
      → [plan 0605](./docs/plans/archive/0605-reconnect-backoff.md)
- [x] 关闭 `Session` 立刻断连，并中止该 `Session` 的重连循环
      验收：关闭转发 Session 后连接数与重连任务数都归零
      → [plan 0606](./docs/plans/archive/0606-close-session-teardown.md)
- [x] **ADR-0003 转「已定案」**（阶段 6 落地完成之后）
      验收：状态为「已定案」，且 §14 修订记录里每次改动都有理由

---

## 阶段 7 — SFTP

目标：双栏传输可用，且**任何时刻都不留下看似完整、实为不完整的文件**。

- [x] 双栏界面骨架 + 两侧独立选主机（**不需要先开终端 Session**）
      验收：直接打开 SFTP 即可用，无终端依赖
      → [plan 0701](./docs/plans/archive/0701-sftp-dual-pane.md)
- [x] local ↔ host 双向；**临时名 + 原子重命名**落盘
      验收：中断传输后目标目录里**没有**看似完整的文件
      → [plan 0702](./docs/plans/archive/0702-transfer-atomic-rename.md)
- [x] host ↔ host：**优先 B 档（`direct-tcpip`），失败回退 A 档（内存 relay）**
      验收：A 无法直连 B 时自动走 A 档；两档均不落盘
      → [plan 0703](./docs/plans/archive/0703-host-to-host-topology.md)
- [x] 并发 in-flight 请求（pipelining）
      验收：大量小文件的吞吐显著优于串行请求
      → [plan 0704](./docs/plans/archive/0704-pipelining.md)

> ADR-0006（SFTP 栈与传输引擎）的落地计划到 0704 为止 —— 本阶段收尾时它转为**已定案**。

---

## 阶段 8 — serial

目标：串口能枚举、能按参数打开，且不把 libudev 带进别的平台。

- [x] `src-tauri/crates/akasha-serial`，`libudev` 走 **Linux-only cargo feature**
      验收：Windows / macOS 构建不链接 libudev
      → [plan 0801](./docs/plans/archive/0801-serial-crate-libudev.md)
- [x] 端口枚举与连接参数（波特率/数据位/停止位/校验/流控）
      验收：枚举在本机列出真实端口；参数错误时给出可读报错
      → [plan 0802](./docs/plans/archive/0802-serial-enumeration-params.md)
- [ ] **Windows 上的枚举带得出描述**（条件编译；上游在那边把一切都报成"未知"）
      验收：Windows 上枚举出来的端口各自带得出描述；没有端口时给出空表而不是错误
      → [plan 0803](./docs/plans/0803-windows-serial-tests.md)

---

## 阶段 9 — Bitwarden 导入

目标：CLI 的获取与前置条件明确，登录（含自托管）在前端可用，只读导入可用，离线缓存强度与本地池同级。

- [x] **`bw` 的获取与前置检查**：二进制（宿主机的 `bw` / 运行时下载的 OSS 变体）与 CLI 状态目录两轴独立可切，默认都取宿主机那一档
      验收：宿主机没有 `bw` 时报出这件事并给出下载动作；下载之后能读到它的版本与变体
      → [plan 0902](./docs/plans/archive/0902-bw-acquire-and-preflight.md)
- [x] **登录 / 解锁 / 锁定接进前端（含自托管）**：服务器地址可设，会话状态取自 CLI 自己的状态，session token 只在内存
      验收：设自托管地址后登录到未解锁态、解锁到已解锁态、锁定后内存里不再有 token；三态与 CLI 自报一致
      → [plan 0905](./docs/plans/archive/0905-bw-login-session-ui.md)
- [ ] **实测 `bw` 对 `sshKey` 条目的非交互行为**（需要真实 vault，可提前做）
      验收：登录之后的 `bw status` 形状、未解锁时的报错、条目的 JSON 形状与离线可见性各有结论，写回 [`docs/bitwarden.md`](./docs/bitwarden.md)
      → [plan 0901](./docs/plans/0901-bw-noninteractive-probe.md)
- [x] 只读导入 SSH key 条目（`sshKey.privateKey`）：导入落进密钥池并记一行来历（上游 id / `revisionDate` / `fingerprint`）
      验收：导入后可用该密钥建立 SSH 连接（`IdentityFile` 的文件名与钥匙名相同时接上那一行）
      → [plan 0903](./docs/plans/archive/0903-bw-readonly-import.md)
- [x] 离线缓存：**私钥离线自检用 `fingerprint`**（不起进程、不联网），联网刷新用 `revisionDate`
      验收：断网时能校验缓存完整性；联网且 `revisionDate` 变化时提示刷新
      → [plan 0904](./docs/plans/archive/0904-bw-offline-cache.md)

---

## 阶段 10 — Android（延迟）

目标：**显式**决定 Android 的形态，并把它写进 `scope.md`。

- [ ] 评估 Android 上的托盘与端口转发形态
      验收：结论写进 `scope.md` §9 / §10，且有可核对的出处
      → [plan 1001](./docs/plans/1001-android-form-factor.md)
- [ ] 只做 ssh/sftp，还是接 Termux（local）/ USB Host（serial）
      验收：这个决定必须**显式**做出并记录，不能是意外结果
      → [plan 1002](./docs/plans/1002-android-scope-decision.md)

---

## 阶段 11 — 串口接入 app

目标：`scope.md` §2 的第三个后端在 app 上可用（阶段 8 的续作，crate 已就位）。

- [x] 串口 `Session` 接入 app（命令 + 注册表 + 标签页 + 关闭与回收）
      验收：真实 app 上打开一个串口会话并双向传字节；关闭标签页后 `live` / `registered` 归零
      → [plan 1101](./docs/plans/archive/1101-serial-session-ipc.md)
- [x] 端口枚举与参数接进界面（列端口 + 手动路径兜底 + 池行取值 + 可读报错）
      验收：界面上看到本机枚举结果并据此（或手输路径）打开；取值越界时显示字段与取值
      → [plan 1102](./docs/plans/archive/1102-serial-ports-ui.md)
- [x] 设备消失时串口会话以可读原因结束，标签页随之关闭
      验收：把测试用的 PTY 主端关闭（等价于拔掉设备）时会话结束、没有残留注册
      → [plan 1103](./docs/plans/archive/1103-serial-device-gone.md)

---

## 明确的非目标

**理由不在这里**（按 §8.1，理由属于 `scope.md`）—— 只列条目：

- 不在 Rust 侧实现屏幕模型
- 不做插件系统
- 不自实现 Bitwarden vault 密码学；**不打包 `bw`**
- 不调用系统 `ssh` 二进制
- 不为 webview 依赖栈做环境重定向
- 不做 host↔host"真不中转"
- 不做串口热插拔事件驱动的自动重连

**每条的理由与完整清单见 [`docs/scope.md`](./docs/scope.md) §10。**
