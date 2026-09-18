# plan 索引与约定

> **一个工作项一个 plan 文件**，本文件是**全部 plan 的单一入口**（含已归档的）。
> plan 回答"怎么做"（步骤 + 可粘贴的验收命令）；**判据在 [`../../ROADMAP.md`](../../ROADMAP.md)**，
> 现状在 [`../STATUS.md`](../STATUS.md)。三级粒度见 [`../README.md`](../README.md)。

## 编号：阶段块号 `TTxx`

- `TT` = ROADMAP 阶段号（2 位），`xx` = 该阶段内序号（2 位）；阶段 0 用 `00xx`。
- **阶段内条目与 plan 一一对应**：ROADMAP 每个条目末尾挂一个指针，指回这里。
- 编号**不复用**：作废的 plan 保留编号，在索引里标「放弃」。
- **已完成的条目不回填 plan**：阶段 0 的记录在提交历史与 `STATUS.md`，补写只增噪音。
- 阶段号排序 = 文件名字典序，所以 `ls docs/plans` 天然按阶段排列。

## 归档（硬规则：防止"一个文件太大"）

1. **完成即归档**：plan 状态改「已完成」后 `git mv` 进 `docs/plans/archive/`；
   索引**保留一行**，文件链接改指 `archive/` 路径。
2. **归档只移动整份文件，永不拼接。** 不许把多份 plan 合成一份「阶段汇总」，
   也不许往已完成的 plan 里追加后续工作 —— 那正是"文件越滚越大"的来源。
   需要新工作就开新编号。
3. `archive/` 只放已完成的 plan；目录本身的存在由其中的 `README.md` 说明。

## 单文件预算（硬规则，由 `just docs-check` 强制）

- **每份 plan ≤ 200 行**（目标 60–120 行）。超了**拆成两份**，在前者头部写「后继：NNNN」——
  不得将细节继续堆进同一个文件。
- **骨架 plan 不许标「进行中」**：没有「## 验收命令」节就不许开工
  （`docs-check` 会拦下来）。判据来自 ROADMAP，但**手段必须能粘贴执行**。

## 状态取值

`未规划（骨架）` · `未开始` · `进行中` · `已完成` · `放弃`

- **未规划（骨架）**：已定位、已声明依赖，但**还没到能写步骤的时候**（通常是前置 ADR/plan 未定）。
  骨架只写目标 / 非目标 / 前置 / 判据，**不写推测性步骤**。
- 从「未规划」转为「未开始」的唯一门槛：**补齐可粘贴的验收命令与分步步骤**。

## 骨架

```markdown
# Plan NNNN: <标题>

- **关联**：ROADMAP 阶段 N ·「<条目原文>」
- **前置**：plan NNNN / ADR-XXXX
- **状态**：未规划（骨架）| 未开始 | 进行中 | 已完成 | 放弃

## 目标 / 非目标
## 前置检查
## 步骤（每步都能独立验证）
## 验收命令（可直接粘贴执行，并写出预期输出）
## 回滚
## 实施记录（边做边追加，记录验收命令的实际输出）
```

能把验收变成 `just` 配方的，就不应写成散文；新增/改名配方要同步 [`../just.md`](../just.md) §2。

---

## 索引

### 阶段 1 — 根 workspace、分层与平台矩阵

| 编号 | 标题 | 状态 | 前置 | 文件 |
|---|---|---|---|---|
| 0101 | 落地根 workspace | 已完成 | ADR-0001 已定案 | [0101](./archive/0101-root-workspace.md) |
| 0106 | Rust 成员收进 `src-tauri/`（**取代 0101 的布局**） | 已完成 | plan 0101 / 0104 | [0106](./archive/0106-workspace-under-src-tauri.md) |
| 0109 | crates 改为 `src-tauri/src` 下的模块（**取代 0106 的成员布局**，ADR-0008） | 已完成（2026-09-19：6 个成员全部并入 `src/`，护栏、配方与文档同步） | ADR-0008（已定案） | [0109](./archive/0109-crates-to-modules.md) |
| 0102 | CI 平台矩阵（Linux + Windows + macOS，GitHub Actions 一份） | 进行中（本地已完成，待 CI 实际运行） | plan 0101 | [0102](./0102-ci-platform-matrix.md) |
| 0103 | `src-tauri/crates/akasha-core`：`Session` 模型骨架 | 已完成 | plan 0101 | [0103](./archive/0103-core-session-model.md) |
| 0104 | 迁移后复测开发循环 | 已完成（监听范围曾失效，已修） | plan 0103（要有 `src-tauri/crates/` 成员才测得了） | [0104](./archive/0104-dev-loop-retest.md) |
| 0105 | `src-tauri/crates/akasha-pty`：通用 `Transport` trait | 已完成 | plan 0103（命名与规则先立） | [0105](./archive/0105-pty-transport-trait.md) |
| 0107 | E2E 入口（`just test-e2e` 自包含） | 进行中（本地已实测；CI 三平台待实际运行） | plan 0202 / 0203 | [0107](./0107-e2e-entry.md) |
| 0108 | Windows 目标的类型检查真的通过（问题 #149 的编译面） | 进行中 | plan 0801（平台边界）/ plan 0102（CI 矩阵要它才绿） | [0108](./0108-windows-type-check.md) |

### 阶段 2 — 端到端最小终端

| 编号 | 标题 | 状态 | 前置 | 文件 |
|---|---|---|---|---|
| 0201 | `Transport` 输出合批 | 已完成 | plan 0105 | [0201](./archive/0201-output-batching.md) |
| 0202 | IPC 二进制通道（输出走 raw 字节） | 已完成 | plan 0201 | [0202](./archive/0202-ipc-binary-channel.md) |
| 0203 | 前端 xterm + WebGL 渲染 | 已完成 | plan 0202 | [0203](./archive/0203-xterm-webgl-render.md) |
| 0204 | 真正退出零残留（能执行代码的两条路径） | 已完成 | plan 0203 | [0204](./archive/0204-exit-zero-residue.md) |
| 0205 | 被 SIGKILL 的退出路径也零残留（看门狗进程 + 管道 EOF） | 已完成 | plan 0204 / ADR-0005 | [0205](./archive/0205-sigkill-exit-residue.md) |

### 阶段 3 — 托盘与应用生命周期

| 编号 | 标题 | 状态 | 前置 | 文件 |
|---|---|---|---|---|
| 0301 | 托盘图标 + 菜单 | 已完成（Linux 实测：注册 / 菜单 / 退出零残留） | plan 0204 | [0301](./archive/0301-tray-icon-menu.md) |
| 0302 | 隐藏而非销毁窗口 | 已完成（隐藏期间进程 / 会话 / 终端缓冲都在，E2E `window_close`） | plan 0301 | [0302](./archive/0302-hide-not-destroy.md) |
| 0303 | 关闭行为可配置 | 已完成（数据目录里的 `config.json`；非法值回默认 + warn） | plan 0302 | [0303](./archive/0303-close-behavior-config.md) |
| 0304 | 单实例 | 已完成（第二个实例 150 ms 内退出并唤起**藏着的**窗口，E2E `single_instance`） | plan 0302 | [0304](./archive/0304-single-instance.md) |
| 0305 | 关闭终端标签页 = 立刻丢弃该 Session | 已完成 | plan 0204 | [0305](./archive/0305-tab-close-discards-session.md) |
| 0306 | 会话自己结束 = 回收它 + 关闭那个标签页 | 已完成 | plan 0305 | [0306](./archive/0306-session-ended-closes-tab.md) |

### 阶段 4 — 存储与凭据池

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0400 | 写 ADR-0002（机密存储与可搬迁） | 已完成 | 阶段 4 开工前 | [0400](./archive/0400-adr-0002-secret-storage.md) |
| 0401 | `rusqlite` + SQLCipher 打开加密库 | 已完成（2026-09-12） | plan 0400（ADR-0002 实现中） | [0401](./archive/0401-sqlcipher-open.md) |
| 0402 | 口令 → KDF → 库密钥 | 已完成（2026-09-12） | plan 0401（库能开） | [0402](./archive/0402-passphrase-kdf.md) |
| 0403 | 四类池的 CRUD（含库内的不变量与私钥的受保护页） | 已完成（2026-09-12） | plan 0401 / 0402 / 0406 | [0403](./archive/0403-pools-crud.md) |
| 0407 | 解锁与锁定的生命周期（谁持有连接、口令从哪来、锁定时抹什么） | 已完成（2026-09-12） | plan 0403 / 0404 / 0406 | [0407](./archive/0407-unlock-lifecycle.md) |
| 0404 | dump 与导出（加密 / 明文两条路 + 还原） | 已完成（2026-09-12） | plan 0403（四类池能读写） | [0404](./archive/0404-dump-export.md) |
| 0406 | 口令的内存防护（`memsafe`） | 已完成（2026-09-12） | plan 0402（`Passphrase` 已就位） | [0406](./archive/0406-memsafe-passphrase-page.md) |
| 0405 | 可搬迁性验证 | 已完成（2026-09-12：配方 `portable`；迁移后四类池 1/1/1/1；便携目录不可写 → 退出码 2） | plan 0403 / 0407（有数据、能读） | [0405](./archive/0405-portability-verify.md) |

### 阶段 5 — SSH 栈（`russh`）

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0501 | 写 ADR-0003（SSH 栈与资源模型） | 已完成（2026-09-12：ADR-0003 置为「实现中」，调研结论带版本号与出处） | 阶段 5 开工前 | [0501](./archive/0501-adr-0003-ssh-stack.md) |
| 0502 | `src-tauri/crates/akasha-ssh`：连接 + 认证（密钥池 / agent / 内存凭据缓存） | 已完成（2026-09-13：三个连接共享一次凭据询问；`just ready` 6/6） | plan 0501（ADR-0003 实现中） | [0502](./archive/0502-ssh-connect-auth.md) |
| 0503 | known_hosts 校验与缓存（含库格式 v2 迁移） | 已完成（2026-09-13：三态判定 + 自动迁移；`just ready` 6/6） | plan 0502（ADR-0003 D11 已定策略） | [0503](./archive/0503-known-hosts.md) |
| 0504 | SSH 接进 IPC / 前端（带目标的命令 + 凭据往返 + 提示界面） | 已完成（2026-09-13：真实 app 上界面选主机 → 提示 → 双向流；凭据只询问一次；关闭标签页零残留） | plan 0503（信任策略已定） | [0504](./archive/0504-ssh-into-ipc-frontend.md) |
| 0505 | `direct-tcpip` 原语 | 已完成（2026-09-13：跳板链连通；库内 4 用例 + 真实 app E2E；`just ready` 6/6） | plan 0504（已完成）· 形状见 ADR-0003 D9 | [0505](./archive/0505-direct-tcpip-primitive.md) |
| 0506 | `~/.ssh/config` 受限子集导入 | 已完成（2026-09-13：三档边界落地；导入的行经跳板真实连通；含 `Match` 的整份报错且一行不写；`just ready` 6/6） | plan 0505（已完成）· 边界见 ADR-0003 D14 | [0506](./archive/0506-ssh-config-subset-import.md) |

> 本阶段的执行顺序由**依赖**决定，不按原立项编号（2026-09-13 重排）：信任策略 → 接入 app →
> 原语 → 配置导入。重排后编号与执行顺序一致；`direct-tcpip` 由 0503 移到 0505，
> 因为它的三处消费者（跳板 / 端口转发 / SFTP）都需要先有一条从 app 建立起来的 SSH 会话才能验证。

### 阶段 6 — SSH 端口转发

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0601 | 隧道实体 + 状态机（五态 · 事件 · 手动重试） | 已完成（2026-09-13：真实 app 上 `连接中 → 已连接 → 已停止`，失败可见且可重试；`just ready` 6/6） | plan 0505（已完成） | [0601](./archive/0601-tunnel-entity-state-machine.md) |
| 0602 | 本地转发 `-L` | 已完成（2026-09-13：真实 app 上「转发端口 → 对端 → 进程内回声服务」通，端口被占用可读） | plan 0601（已完成） | [0602](./archive/0602-local-forward.md) |
| 0603 | 动态转发 `-D`（SOCKS5） | 已完成（2026-09-13：SOCKS5 服务端落地且只绑回环；真实 app 上 `curl --socks5-hostname` 取回远端服务；`just ready` 6/6） | plan 0602（已完成） | [0603](./archive/0603-dynamic-forward-socks5.md) |
| 0604 | 远程转发 `-R` | 已完成（2026-09-14：服务端监听 + `forwarded-tcpip` 回连本机服务；本机服务不可达时**拒绝**通道；停止即撤销；`just ready` 6/6） | plan 0603（已完成）· 机制见 ADR-0003 D10 | [0604](./archive/0604-remote-forward.md) |
| 0605 | 断线重连（3 次 + 指数退避） | 已完成（2026-09-15：服务端断开 → `重连中(1)` → 自行回到 `已连接`；服务端消失 → 三次退避后 `失败`，7.55 s；`just ready` 6/6） | plan 0602 / 0603 / 0604（已完成）· 判据见 ADR-0003 D12 / D13 | [0605](./archive/0605-reconnect-backoff.md) |
| 0606 | 关闭 Session 断连 + 中止重连 | 已完成（2026-09-15：关闭之后连接数与看护任务数都归零，对端也看不到那条连接；在途的尝试 7.5 ms 内被中止；`just ready` 6/6） | plan 0605（已完成）· 连接所有权见 ADR-0003 D5 | [0606](./archive/0606-close-session-teardown.md) |

> ADR-0003（SSH 栈与资源模型）的落地计划到 0606 为止 —— 本阶段收尾时它转为**已定案**。

### 阶段 7 — SFTP

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0701 | 双栏界面骨架 + 两侧独立选主机 | 已完成（2026-09-15：真实 app 上两侧各连一台服务端、各自列目录；SFTP 之前之后终端标签页数不变；`just ready` 6/6） | plan 0505（已完成）· 形状见 ADR-0006 | [0701](./archive/0701-sftp-dual-pane.md) |
| 0702 | local ↔ host：临时名 + 原子重命名 | 已完成（2026-09-15：库内 6 条用两个可控假端点把"传到哪一步"变成可等待的事件；E2E 取消后目标目录 0 个条目、上传字节与源相同；`just ready` 6/6） | plan 0701（已完成）· 形状见 ADR-0006 | [0702](./archive/0702-transfer-atomic-rename.md) |
| 0703 | host ↔ host：B 档优先，回退 A 档 | 已完成（2026-09-15：B 档经跳板直通——跳板记到恰好 1 条 `direct-tcpip`、中继搬过字节、目标真盘字节相同；直通被拒时自动回退本机直连且留下原因；库内 3 条含两半负控；`just ready` 6/6） | plan 0702（已完成）· 形状见 ADR-0006 D5 | [0703](./archive/0703-host-to-host-topology.md) |
| 0704 | 并发 in-flight 请求 | 已完成（2026-09-15：上限归 `InFlight`（会话持一个、等空位可取消）；临时名改成一次原子占用；库内 6 条 + 端到端 `sftp_pipelining`；12 个 1 KiB 文件在带时延链路上 1.17 s → 0.21 s；`just ready` 6/6） | plan 0703（已完成）· 上限见 ADR-0006 D6 | [0704](./archive/0704-pipelining.md) |

> 本阶段的形状固化在 [ADR-0006](../adr/0006-sftp-stack-and-transfer-engine.md)：SFTP 库选型、
> 会话承载在**哪条流**上、一个 `Session` 拥有**两侧独立连接**、以及传输引擎的端点职责边界
> （0702 / 0703 / 0704 的地基）。0704 归档时它转为**已定案**。

### 阶段 8 — serial

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0801 | `src-tauri/crates/akasha-serial`：`libudev` 走 Linux-only feature | 已完成（2026-09-15：串口 `Transport` 落地；`just libudev-check` 按目标核对依赖图并用两条负例验过；13 条用例全过；`just ready` 6/6。⚠️ Windows 目标的原生编译待问题 #149） | plan 0105（`Transport` 已定形态） | [0801](./archive/0801-serial-crate-libudev.md) |
| 0802 | 端口枚举与连接参数 | 已完成（2026-09-15：`ports()` 列本机 32 条且顺序稳定 / 无重复；参数在真 tty 上回读、越界取值报字段与取值；两条枚举实现各执行一遍 —— `just serial-check`；`just ready` 6/6。⚠️ 数据位 / 校验位在 PTY 上被归一化，真机证据见「待验证」） | plan 0801（已完成） | [0802](./archive/0802-serial-enumeration-params.md) |

### 阶段 9 — Bitwarden 导入

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0901 | 实测 `bw` 对 `sshKey` 条目的非交互行为 | 未规划（骨架；四项里变体判定已实测，其余三项需要真实 vault） | 展开时机：需要真实 vault（可提前） | [0901](./0901-bw-noninteractive-probe.md) |
| 0902 | `bw` 的获取与前置检查（两个轴 + 运行时下载 + 变体判定） | 已完成（2026-09-15：`akasha-bw` 46 条用例；真实上游拉过一次（SHA-256 与 zip 逐字符相同）；app 那一侧由 `bitwarden_login` 覆盖） | ADR-0007（实现中） | [0902](./archive/0902-bw-acquire-and-preflight.md) |
| 0905 | 登录 / 解锁 / 锁定接进前端（含自托管） | 已完成（2026-09-15：E2E `bitwarden_login` 十步全过（5.84 s）；真实 vault 的成功登录路径仍未实测） | plan 0902（已完成） | [0905](./archive/0905-bw-login-session-ui.md) |
| 0903 | 只读导入 SSH key 条目（含"导入的钥匙接到主机上"那条规则） | 已完成（2026-09-15：E2E `bw_import` —— 服务端看到的指纹与导入时存下的逐字符相同；库格式 v3） | plan 0902 / 0905（已完成） | [0903](./archive/0903-bw-readonly-import.md) |
| 0904 | 离线缓存（fingerprint / revisionDate） | 已完成（2026-09-15：E2E 里自检从"完好"翻成"对不上"、比对从"没变"翻成"上游变过"；无新的库格式迁移） | plan 0903（已完成） | [0904](./archive/0904-bw-offline-cache.md) |

### 阶段 10 — Android（延迟）

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 1001 | 评估 Android 上的托盘与端口转发形态 | 未规划（骨架） | 展开时机：Android 立项时 | [1001](./1001-android-form-factor.md) |
| 1002 | 只做 ssh/sftp 还是接 Termux / USB Host | 未规划（骨架） | 展开时机：plan 1001 之后 | [1002](./1002-android-scope-decision.md) |

### 阶段 11 — 串口接入 app

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 1101 | 串口 `Session` 接入 app（命令 + 注册表 + 标签页 + 关闭与回收） | 已完成（2026-09-15：设备 → 界面 → 设备两个方向都在真实 app 上走通；关标签页后注册表回到打开前的读数（"归零"的口径同 `ssh_session`）；`just ready` 6/6） | 前置：plan 0801 / 0802（已完成） | [1101](./archive/1101-serial-session-ipc.md) |
| 1102 | 端口枚举与参数接进界面 | 已完成（2026-09-15：三条并列的输入（枚举到的端口 / 手输路径 / 池行取值）互不遮挡；取值越界时那个面的报错行里是 `data_bits = 9`，非数字由面板拦下、不开面；`just ready` 6/6） | 前置：plan 1101（已完成） | [1102](./archive/1102-serial-ports-ui.md) |
| 1103 | 设备消失时串口会话以可读原因结束 | 已完成（2026-09-15：关闭 PTY 主端（等价于拔掉设备）→ 会话自己结束、标签页随之关闭，通知行里那句话带设备路径与 OS 原因；注册表回到打开前的读数；`just ready` 6/6） | 前置：plan 1101 / 1102（已完成） | [1103](./archive/1103-serial-device-gone.md) |
