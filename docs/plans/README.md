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
  不要把细节继续堆进同一个文件。
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

能把验收变成 `just` 配方的，就不要写成散文；新增/改名配方要同步 [`../just.md`](../just.md) §2。

---

## 索引

### 阶段 1 — 根 workspace、分层与平台矩阵

| 编号 | 标题 | 状态 | 前置 | 文件 |
|---|---|---|---|---|
| 0101 | 落地根 workspace | 已完成 | ADR-0001 已接受 | [0101](./archive/0101-root-workspace.md) |
| 0106 | Rust 成员收进 `src-tauri/`（**取代 0101 的布局**） | 已完成 | plan 0101 / 0104 | [0106](./archive/0106-workspace-under-src-tauri.md) |
| 0102 | CI 平台矩阵（Linux + Windows + macOS，GitHub Actions 一份） | 进行中（本地已完成，待 CI 实跑） | plan 0101 | [0102](./0102-ci-platform-matrix.md) |
| 0103 | `src-tauri/crates/akasha-core`：`Session` 模型骨架 | 已完成 | plan 0101 | [0103](./archive/0103-core-session-model.md) |
| 0104 | 迁移后复测开发循环 | 已完成（监听范围曾失效，已修） | plan 0103（要有 `src-tauri/crates/` 成员才测得了） | [0104](./archive/0104-dev-loop-retest.md) |
| 0105 | `src-tauri/crates/akasha-pty`：通用 `Transport` trait | 已完成 | plan 0103（命名与规则先立） | [0105](./archive/0105-pty-transport-trait.md) |
| 0107 | E2E 入口（`just test-e2e` 自包含） | 进行中（本地已实测；CI 三平台待实跑） | plan 0202 / 0203 | [0107](./0107-e2e-entry.md) |

### 阶段 2 — 端到端最小终端

| 编号 | 标题 | 状态 | 前置 | 文件 |
|---|---|---|---|---|
| 0201 | `Transport` 输出合批 | 已完成 | plan 0105 | [0201](./archive/0201-output-batching.md) |
| 0202 | IPC 二进制通道（输出走 raw 字节） | 已完成 | plan 0201 | [0202](./archive/0202-ipc-binary-channel.md) |
| 0203 | 前端 xterm + WebGL 渲染 | 已完成 | plan 0202 | [0203](./archive/0203-xterm-webgl-render.md) |
| 0204 | 真正退出零残留 | 未开始 | plan 0203 | [0204](./0204-exit-zero-residue.md) |

### 阶段 3 — 托盘与应用生命周期

| 编号 | 标题 | 状态 | 前置 | 文件 |
|---|---|---|---|---|
| 0301 | 托盘图标 + 菜单 | 未开始 | plan 0204 | [0301](./0301-tray-icon-menu.md) |
| 0302 | 隐藏而非销毁窗口 | 未开始 | plan 0301 | [0302](./0302-hide-not-destroy.md) |
| 0303 | 关闭行为可配置 | 未开始 | plan 0302 | [0303](./0303-close-behavior-config.md) |
| 0304 | 单实例 | 未开始 | plan 0302 | [0304](./0304-single-instance.md) |

### 阶段 4 — 存储与凭据池

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0400 | 写 ADR-0002（机密存储与可搬迁） | 未开始 | 阶段 4 开工前 | [0400](./0400-adr-0002-secret-storage.md) |
| 0401 | `rusqlite` + SQLCipher 打开加密库 | 未规划（骨架） | 展开时机：plan 0400 定案后 | [0401](./0401-sqlcipher-open.md) |
| 0402 | 口令 → KDF → 库密钥 | 未规划（骨架） | 展开时机：plan 0400 定案后 | [0402](./0402-passphrase-kdf.md) |
| 0403 | 四套池的 CRUD | 未规划（骨架） | 展开时机：plan 0401 / 0402 之后 | [0403](./0403-pools-crud.md) |
| 0404 | dump 与导出 | 未规划（骨架） | 展开时机：plan 0403 之后 | [0404](./0404-dump-export.md) |
| 0405 | 可搬迁性验证 | 未规划（骨架） | 展开时机：plan 0403 之后 | [0405](./0405-portability-verify.md) |

### 阶段 5 — SSH 栈（`russh`）

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0501 | 写 ADR-0003（SSH 栈与资源模型） | 未开始 | 阶段 5 开工前 | [0501](./0501-adr-0003-ssh-stack.md) |
| 0502 | `src-tauri/crates/akasha-ssh`：连接 + 认证 | 未规划（骨架） | 展开时机：plan 0501 定案后 | [0502](./0502-ssh-connect-auth.md) |
| 0503 | `direct-tcpip` 原语 | 未规划（骨架） | 展开时机：plan 0502 之后 | [0503](./0503-direct-tcpip-primitive.md) |
| 0504 | `~/.ssh/config` 受限子集导入 | 未规划（骨架） | 展开时机：plan 0502 之后 | [0504](./0504-ssh-config-subset-import.md) |
| 0505 | known_hosts 校验与缓存 | 未规划（骨架） | 展开时机：plan 0502 之后 | [0505](./0505-known-hosts.md) |

### 阶段 6 — SSH 端口转发

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0601 | 隧道实体 + 状态机 | 未规划（骨架） | 展开时机：plan 0503 之后 | [0601](./0601-tunnel-entity-state-machine.md) |
| 0602 | 本地转发 `-L` | 未规划（骨架） | 展开时机：plan 0601 之后 | [0602](./0602-local-forward.md) |
| 0603 | 动态转发 `-D`（SOCKS5） | 未规划（骨架） | 展开时机：plan 0601 之后 | [0603](./0603-dynamic-forward-socks5.md) |
| 0604 | 远程转发 `-R` | 未规划（骨架） | 展开时机：plan 0601 之后 | [0604](./0604-remote-forward.md) |
| 0605 | 断线重连（3 次 + 指数退避） | 未规划（骨架） | 展开时机：plan 0601 之后 | [0605](./0605-reconnect-backoff.md) |
| 0606 | 关闭 Session 断连 + 中止重连 | 未规划（骨架） | 展开时机：plan 0605 之后 | [0606](./0606-close-session-teardown.md) |

### 阶段 7 — SFTP

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0701 | 双栏界面骨架 + 两侧独立选主机 | 未规划（骨架） | 展开时机：阶段 7 开工时 | [0701](./0701-sftp-dual-pane.md) |
| 0702 | local ↔ host：临时名 + 原子重命名 | 未规划（骨架） | 展开时机：plan 0701 之后 | [0702](./0702-transfer-atomic-rename.md) |
| 0703 | host ↔ host：B 档优先，回退 A 档 | 未规划（骨架） | 展开时机：plan 0503 / 0702 之后 | [0703](./0703-host-to-host-topology.md) |
| 0704 | 并发 in-flight 请求 | 未规划（骨架） | 展开时机：plan 0702 之后 | [0704](./0704-pipelining.md) |

### 阶段 8 — serial

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0801 | `src-tauri/crates/akasha-serial`：`libudev` 走 Linux-only feature | 未规划（骨架） | 展开时机：阶段 8 开工时 | [0801](./0801-serial-crate-libudev.md) |
| 0802 | 端口枚举与连接参数 | 未规划（骨架） | 展开时机：plan 0801 之后 | [0802](./0802-serial-enumeration-params.md) |

### 阶段 9 — Bitwarden 导入

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 0901 | 实测 `bw` 对 `sshKey` 条目的非交互行为 | 未规划（骨架） | 展开时机：需要真实 vault（可提前） | [0901](./0901-bw-noninteractive-probe.md) |
| 0902 | 前置检查：探测 `bw` 及其变体 | 未规划（骨架） | 展开时机：plan 0901 之后 | [0902](./0902-bw-preflight-check.md) |
| 0903 | 只读导入 SSH key 条目 | 未规划（骨架） | 展开时机：plan 0902 之后 | [0903](./0903-bw-readonly-import.md) |
| 0904 | 离线缓存（fingerprint / revisionDate） | 未规划（骨架） | 展开时机：plan 0903 之后 | [0904](./0904-bw-offline-cache.md) |

### 阶段 10 — Android（延迟）

| 编号 | 标题 | 状态 | 前置 / 展开时机 | 文件 |
|---|---|---|---|---|
| 1001 | 评估 Android 上的托盘与端口转发形态 | 未规划（骨架） | 展开时机：Android 立项时 | [1001](./1001-android-form-factor.md) |
| 1002 | 只做 ssh/sftp 还是接 Termux / USB Host | 未规划（骨架） | 展开时机：plan 1001 之后 | [1002](./1002-android-scope-decision.md) |
