# ROADMAP

> **回答"我们去哪"** —— "下一步做什么"只在这一个文件里，不散落在 `AGENTS.md` 或 `docs/STATUS.md`。
> 这里**只勾复选框**，不写细节：具体步骤在 [`docs/plans/`](./docs/plans/)，
> 当前到哪一步在 [`docs/STATUS.md`](./docs/STATUS.md)。
>
> 每个条目必须有**可执行的验收标准**。写不出验收标准的条目不许进入本文件。
>
> **能力清单在 [`docs/scope.md`](./docs/scope.md)** —— 本文件的阶段从它派生。

图例：`[x]` 已完成 · `[ ]` 未开始 · `[~]` 进行中 · `[!]` 被阻塞

---

## 阶段 0 — 地基（可编译、可验证、可交接）

目标：任何时刻 `just ready` 是绿的，规范/现状/进度各有唯一来源。

- [x] 工具链与依赖就位（`just check` / `just lint` 退出码 0）
- [x] 依赖门禁（`just deny-offline` → bans / licenses / sources all ok）
- [x] 规范与文档体系（`AGENTS.md` + `ROADMAP.md` + `docs/`）
- [x] **文档体系随范围扩大同步**（`docs/scope.md` 与 `docs/portable.md` 已登记进
      `AGENTS.md` §8；命名约定已写入 §3.1）
- [x] **ADR 队列收敛为 3 份**（见 [`docs/adr/README.md`](./docs/adr/README.md)）
- [~] **修好 CI** —— 已合并为 `.github/workflows/ci.yml`（`checks` 跑 `just ready`，
      `e2e` 依赖 `checks`）
      验收：CI 上两个 job 变绿 —— **待首次推送确认**
- [ ] **ADR-0001 定案**（决策二已裁定，见其 §0 补记）
      验收：`docs/adr/0001` 状态改为"已接受"，或写明被哪条取代

---

## 阶段 1 — 根 workspace、分层与平台矩阵

目标：多 crate 布局落地，且**跨平台差异从第一天就被验证**，而不是留到收尾。

- [ ] **落地 ADR-0001 决策一**（根 workspace）
      验收：仓库根 `cargo metadata --no-deps` 列出 ≥2 个包，且 `just ready` 仍全绿
      见 [`docs/plans/0001`](./docs/plans/0001-root-workspace.md)
- [ ] **迁移后复测开发循环**：确认改 `crates/` 下的文件**仍触发重编译与重启**
      验收：CLI 打印的监听范围覆盖 `crates/`；改一个 `crates/` 文件后 app 自动重启
      （这是迁移最容易静默破坏的地方，`cargo check` 看不出来）
- [ ] **CI 平台矩阵**（Linux + Windows + macOS）
      验收：三平台各跑一遍 `cargo check`；Linux 跑完整 `just ready`
      —— **不能推到阶段 9**，PTY/serial/ssh 的平台差异是主体工作量
- [ ] `crates/akasha-core` 骨架：**`Session` 模型先立起来**
      验收：单测覆盖 `SessionId` 分配、关闭一个 `Session` 不影响另一个
      —— 必须先于任何后端，否则"终端 = 应用"的假设会被写进架构
- [ ] `crates/akasha-pty`：通用 `Transport` trait + `portable-pty` 实现
      验收：`FakeTransport` 覆盖 spawn / write / shutdown 的单元测试通过

---

## 阶段 2 — 端到端最小终端

目标：能开一个 shell、敲命令、看到输出、关窗口不残留进程。

- [ ] `Transport` 输出合批（≥16ms 或 ≥64KiB）
      验收：单元测试断言 chunk 边界；`criterion` 基准给出吞吐数字
- [ ] IPC 二进制通道（`tauri::ipc::Channel<Vec<u8>>`）
      验收：ast-grep 规则 `no-string-pty-channel` 生效；大输出不掉帧
- [ ] 前端 xterm + WebGL 渲染，字节流不进 React state
      验收：Victauri `dom_snapshot` 确认 canvas 存在；无 per-chunk 组件重渲染
- [ ] **进程生命周期按 `AGENTS.md` §3.3 的三态实现**（窗口关闭 / 配置为直接退出 / 真正退出）
      验收：Victauri `introspect { action: "processes" }` 在**真正退出后**零残留；
      并且**收托盘状态下子进程仍在**（这不是泄漏，见 §3.3）

---

## 阶段 3 — 托盘与应用生命周期

目标：点叉不退出；隧道与终端在窗口隐藏期间存活。

- [ ] 托盘图标 + 菜单（显示/隐藏、退出）；Linux 依赖 `libayatana-appindicator3`
      验收：点叉后进程仍在、托盘可见；从托盘退出后零残留
- [ ] **隐藏而非销毁**窗口
      验收：隐藏再显示后，终端回滚缓冲仍在（证明窗口没被销毁）
- [ ] 关闭行为可配置（收托盘 / 直接退出）
      验收：切成"直接退出"后点叉即退出，且零残留
- [ ] 单实例（第二个实例唤起已有窗口，而不是各跑一套）
      验收：连续启动两次只有一个进程、一条隧道

---

## 阶段 4 — 存储与凭据池

目标：四套池可增删改查；库加密；可导出。

- [ ] `rusqlite` + **SQLCipher**（`bundled-sqlcipher-vendored-openssl`）
      验收：用错误口令打不开库；`.db` 文件里搜不到明文密钥
- [ ] 口令 → KDF → 库密钥（**不依赖 OS keychain**，见 `scope.md` §1）
      验收：无任何 `keyring` 类依赖（`cargo tree` 可证）
- [ ] 四套池的 CRUD：密钥 / ssh 配置 / serial 配置 / 端口转发规则
      验收：各自的 round-trip 单测通过；**库里不存绝对路径**（P2）
- [ ] dump 与导出（可选加密；明文导出必须二次确认）
      验收：加密导出可在另一目录导入还原；明文导出路径有显式确认门槛
- [ ] **可搬迁性验证**（见 [`docs/portable.md`](./portable.md)）
      验收：移动整个文件夹后重启，**原有主机/密钥/规则都在**（只验证"能开"不算过）

---

## 阶段 5 — SSH 栈（`russh`）

目标：纯 Rust SSH，不调系统 `ssh`。

- [ ] `crates/akasha-ssh`：连接 + 认证（密钥池 / agent / 内存凭据缓存）
      验收：同主机开三个 Session **只问一次**凭据（`scope.md` §2.2）
- [ ] **`direct-tcpip` 原语**（先做跳板，等于拿到另外两处的地基）
      验收：ProxyJump 可连通只对跳板机可见的目标
- [ ] `~/.ssh/config` **受限子集**导入（`Match`/`Include` 显式报错）
      验收：含 `Match` 的配置产生**明确报错**，不是静默误解析
- [ ] known_hosts 校验与缓存
      验收：host key 变化时拒绝连接并提示（不静默接受）
- [ ] ADR-0002 / ADR-0003 落地（写于本阶段开工前，见 `docs/adr/README.md`）

---

## 阶段 6 — SSH 端口转发

目标：本地/远程/动态三类转发，独立 Session，失败可见。

- [ ] 隧道实体（独立于终端 `Session`）+ 状态机
      验收：`连接中/已连接/重连中/失败/已停止` 五态可观测；状态变化发事件
- [ ] 本地转发 `-L`（复用阶段 5 的 `direct-tcpip`）
      验收：转发端口可访问远端服务
- [ ] 动态转发 `-D`（本地 SOCKS5 服务端）
      验收：配置 SOCKS5 代理后能访问远端网络
- [ ] 远程转发 `-R`（`tcpip-forward` + `forwarded-tcpip`，**另一套机制**）
      验收：远端监听端口可回连到本机服务
- [ ] 断线重连：**3 次 + 指数退避**，然后标记失败
      验收：拔网线后进入"重连中"，耗尽次数后变"失败"**且托盘可见**；可手动重试
- [ ] 关闭 `Session` 立刻断连，并中止该 `Session` 的重连循环
      验收：关闭转发 Session 后连接数与重连任务数都归零

---

## 阶段 7 — SFTP

- [ ] 双栏界面骨架 + 两侧独立选主机（**不需要先开终端 Session**）
      验收：直接打开 SFTP 即可用，无终端依赖
- [ ] local ↔ host 双向；**临时名 + 原子重命名**落盘
      验收：中断传输后目标目录里**没有**冒充完整的文件
- [ ] host ↔ host：**优先 B 档（`direct-tcpip`），失败回退 A 档（内存 relay）**
      验收：A 无法直连 B 时自动走 A 档；两档均不落盘
- [ ] 并发 in-flight 请求（pipelining）
      验收：大量小文件的吞吐显著优于串行（给出基准数字）

---

## 阶段 8 — serial

- [ ] `crates/akasha-serial`，`libudev` 走 **Linux-only cargo feature**
      验收：Windows / macOS 构建不链接 libudev（`cargo tree` 可证）
- [ ] 端口枚举与连接参数（波特率/数据位/停止位/校验/流控）
      验收：枚举在本机列出真实端口；参数错误时给出可读报错

---

## 阶段 9 — Bitwarden 导入

- [ ] 前置检查：探测 `bw` **及其变体**
      验收：未安装 → 明确报"需安装 Bitwarden CLI **及该装哪个变体**"；
      专有变体 → 给出提示（见 `scope.md` §7.0 的许可证约束）
- [ ] 只读导入 SSH key 条目（`sshKey.privateKey`）
      验收：导入后可用该密钥建立 SSH 连接
- [ ] 离线缓存：**私钥离线自检用 `fingerprint`**，联网刷新用 `revisionDate`
      验收：断网时能校验缓存完整性；联网且 `revisionDate` 变化时提示刷新
- [ ] **实现前先实测** `bw` 对 `sshKey` 条目的非交互行为（`STATUS.md` 待办）

---

## 阶段 10 — Android（延迟）

- [ ] 评估 Android 上的托盘与端口转发形态
- [ ] 只做 ssh/sftp，还是接 Termux（local）/ USB Host（serial）
      验收：这个决定必须**显式**做出并记录，不能是意外结果

---

## 明确的非目标

完整清单与理由见 [`docs/scope.md`](./docs/scope.md) §10。当前要点：

- 不在 Rust 侧实现屏幕模型（除非 `docs/adr/0001` §3 的触发条件满足）
- 不做插件系统
- 不自实现 Bitwarden vault 密码学；**不打包 `bw`**（许可证，见 §7.0）
- 不调用系统 `ssh` 二进制
- 不为 webview 依赖栈做环境重定向
- 不做 host↔host"真不中转"
