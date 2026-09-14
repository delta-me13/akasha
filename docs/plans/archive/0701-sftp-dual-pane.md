# Plan 0701: SFTP 双栏界面骨架 + 两侧独立选主机

- **关联**：ROADMAP 阶段 7 ·「双栏界面骨架 + 两侧独立选主机（**不需要先开终端 Session**）」·
  ADR-0006（D2 / D3 / D7）· `scope.md` §4 / §5.1
- **前置**：阶段 5（能建立 SSH 连接、`SshConnection` 已就位）· 阶段 4（主机从池中选择）·
  ADR-0006（形状已定）
- **状态**：已完成（2026-09-15）

## 目标

双栏 SFTP：左右两侧各自从池里选主机（`scope.md` §4）。两侧**独立**，且**不依赖终端 `Session`**
（`scope.md` §5.1 第 3 条：SFTP 不需要先开一个终端）。

本 plan 交付到"**两侧各自列目录成功**"为止 —— 那是 ROADMAP 的判据，也是传输（0702）能用眼睛
看见的前提：目录列不出来，就没有东西可传。

## 非目标

- 传输本身（0702 / 0703 / 0704）：本 plan **只建立两侧连接并列出目录**
- 本地文件系统那一栏（`local ↔ host` 的 local 侧，0702）：本 plan 两侧都是主机
- 拖拽 / 快捷键 / 多选 / 路径补全等交互打磨（UI 阶段）
- 断点续传（`later`，`scope.md` §10）
- 上传 / 下载 / 删除 / 重命名等**写**操作：它们没有判据要求，且写路径的不变量属于 0702

## 先定死的三件事

1. **"两侧"是后端的两个格子，不是前端的两个面板。** 一个 SFTP `Session` 拥有两侧，每侧一条
   **独立**的 `SshConnection` 与一个 `SftpClient`（ADR-0006 D3）。判据里的"独立"因此有可断言的
   形式：两侧可以各连**不同的**服务端，一侧断开不影响另一侧。
2. **`side` 是唯一进入契约的呈现概念**（ADR-0006 D7）：过 IPC 的取值是稳定短名 `left` /
   `right`。后端不给它别的含义 —— 资源归那个 `Session`，不归某一侧。
3. **列目录要能分辨"两侧各自"**。E2E 因此起**两台**进程内 SFTP 服务端（各自的目录内容不同），
   而不是对同一台服务端连两次 —— 后者在"两次调用都返回同一个列表"时也能通过，
   那正是这条判据最该分得开的情形。

## 步骤（每步都能独立验证）

1. **依赖**：`akasha-ssh/Cargo.toml` 加 `russh-sftp = "=3.0.0"`（ADR-0006 D1）。
   ⚠️ 沙箱里 `cargo add` 会去写 `~/.cargo` 的索引缓存（问题 #105），所以手工编辑 manifest，
   再由 `cargo check` 更新 `Cargo.lock`。验证：`just check`；`cargo deny` 的许可证一档仍通过。
2. **`akasha-ssh/src/sftp.rs`**（新）：`SftpClient`（封装上游会话，**不外漏上游类型**，ADR-0006 D2）
   与打开入口 —— 开 session 通道 → `request_subsystem("sftp")` → 上游 `SftpSession::new(流)`。
   操作只做本阶段要用的：`list(path)`（先 `canonicalize` 再 `read_dir`，返回规范化路径 + 条目）。
   失败档新增一档（ADR-0006 §5）。验证：`just test`（crate 层用例：连上测试服务端后列目录）。
3. **`akasha-ssh/src/testing.rs`**：进程内服务端支持 `sftp` 子系统。
   `ServerOptions` 多一档"是否提供 sftp + 这个根目录下有哪些条目"；实现 `subsystem_request`
   （认下 → 把通道交给上游服务端）；观察点记"收到过几次 sftp 子系统请求"。
   验证：`just test`（crate 层的正例 + 负例：没开 sftp 的服务端必须让 `SftpClient` 建不起来）。
4. **`akasha` 侧 `src-tauri/src/sftp.rs`**（新）：实体 `Sftp { id, sides: [Side; 2] }`。
   每侧持 `{主机行 id, 名字, 状态, 当前路径, 连接与客户端}`。四条命令（ADR-0006 D7）：
   `sftp_open`（同步，只登记）/ `sftp_connect`（async：照池里的行建连接再开会话）/
   `sftp_list`（async）/ `sftp_close`（同步改状态，收尾在 runtime 上排队）。
   只读探针 `sftp` 报两侧状态与路径。验证：`just test`（app 层：状态转移与"两侧互不影响"）。
5. **`src-tauri/src/session.rs`**：第三张表 `sftps`，与 `live` / `tunnels` **共用同一张注册表**
   （ADR-0003 D6 的纪律）；`len()` / `registered()` 的对等关系、`shutdown_all` 与
   `remove_sftp` 一并接上。验证：`just test`（`snapshot` 的两个数仍相等）。
6. **`bindings.rs` + 生成物**：四条命令登记进 `collect_commands!`（**没有新事件** —— 连接结果
   由命令返回，`sftp` 探针供诊断）。`just gen-types` 之后提交 `src/ipc/bindings.ts` 的差异。
   验证：`just gen-types-check`。
7. **前端**：`src/ipc/sftp.ts`（唯一的后端调用口）、`src/sftp/SftpPanel.tsx`（双栏：每栏选主机 →
   连接 → 列目录 → 点目录进入 / 点 `..` 返回上级）、`TabStrip` 多一个入口。
   ⚠️ 面板**不是**标签页：它是仅渲染的视图（`scope.md` §5.6），关面板不停后端会话 ——
   停止是面板里的显式动作（`sftp_close`）。验证：`pnpm build`。
8. **E2E `sftp_dual_pane`**（真实 app）：见「验收命令」。新增目标必须登记进
   `src-tauri/justfile` 的 `E2E_TARGETS`（漏加 = 位于门禁之外，问题 #36）。
9. **文档同步**：本 plan 置「已完成」并移入 `archive/`（索引与 ROADMAP 指针同步）；
   ADR-0006 按需修订（实现暴露的问题照规矩记一行）；`docs/STATUS.md` 覆盖写。

## 验收命令（可直接粘贴执行，并写出预期输出）

```bash
# 1. 类型检查（新依赖 + 新模块）
just check          # 预期：退出码 0

# 2. crate 层与 app 层的单测（含两侧互不影响、sftp 子系统的正例与负例）
just test           # 预期：退出码 0，全绿

# 3. 门禁（格式 / lint / 全量测试 / 依赖 / 生成物已提交 / 文档）
just ready          # 预期：6/6 全部通过，退出码 0

# 4. 真实 app 上的验收（自包含：没有 app 就自己起一套）
just test-e2e       # 预期：退出码 0；清单含新增的 sftp_dual_pane

# 5. 前端类型（不在 `just ready` 内）
pnpm build          # 预期：退出码 0
```

真实路径的对照（`sftp_dual_pane` 逐条做同一件事）：

| 断言 | 手段 | 预期 |
|---|---|---|
| **判据：不打开任何终端也能用** | app 打开后**不新建任何终端标签页**，只开 SFTP 面板并连接两侧 | `sftp` 探针报出那个会话；`sessions` 的 `live` / `registered` 只比 SFTP 打开前多 1 |
| **判据：两侧各自列目录成功** | 池里两台主机分别指向**两台**进程内服务端（目录内容不同） | 左栏列出 A 的条目、右栏列出 B 的条目，两组条目内容不同 |
| 两侧各自建了一条连接 | 两台服务端各自的观察点 | 各记到 1 次 sftp 子系统请求 |
| 两侧互不影响 | 只连左侧、右侧保持未连 | `sftp` 探针里左侧 `connected`、右侧 `disconnected`；列右侧目录报 `notConnected` |
| 关闭会话 | 面板里的显式停止动作（`sftp_close`） | 探针里那个会话消失、`live` / `registered` 回到打开前、两台服务端各看到连接断开 |

## 回滚

- 代码：新增一个依赖（`russh-sftp`）、一个 crate 模块（`akasha-ssh/src/sftp.rs`）、一个 app
  模块（`src-tauri/src/sftp.rs`）、一个前端目录（`src/sftp/`）；对既有路径的改动是四处 ——
  `session.rs` 多一张表与对应的三个方法支路、`bindings.rs` 多四条命令、`testing.rs` 多
  `subsystem_request` 与一个观察点、`TabStrip` / `App` 各多一个入口。回退即恢复这四处。
- 数据：**不涉及格式变更**（没有新表、没有迁移）。
- 契约：新增四条命令与一个只读探针；**没有新事件**，既有事件载荷不动。
- ⚠️ `Sftp` 实体的回收走 `SshConnection::disconnect`（runtime 上排队）—— 回退时若只删命令而
  留着 `shutdown_all` 的支路，退出路径会漏收两侧连接。

## 实施记录（边做边追加）

- **2026-09-15 展开**：骨架 → 进行中（补齐先定死的三件事、9 步、可粘贴的验收命令与判据对照表）。
- **2026-09-15 落地**：`akasha-ssh/src/sftp.rs`（`SftpClient`：`SshConnection::sftp()` 开子系统会话；
  `list()` 先 `canonicalize` 再 `read_dir`，条目**按名字排序**——上游按服务端给的随机顺序返回，
  不确定的顺序会让"同一个目录两次列出不一样"这类假象四处出现）；失败分档新增 `SshError::Sftp`；
  新增依赖 `russh-sftp = "=3.0.0"`（ADR-0006 D1）。
- **测试服务端支持 sftp 子系统**：`ServerOptions.sftp`（`SftpItem::file` / `SftpItem::dir`）、
  `subsystem_request`（没开这一档就**明确回绝**）、观察点 `sftp_subsystems`；`channel_open_session`
  从此把通道存下来（子系统请求只给 `ChannelId`，而它要的是通道本体 —— 此前那个通道是被丢掉的）。
- **app 侧**：`src-tauri/src/sftp.rs`（`Sftp` 实体 = 两侧，每侧一条独立连接 + 会话句柄；
  `prepare_connect` 把上一次那条连接**交出去**在锁外回收）；`session.rs` 第三张表 `sftps`
  （与 `live` / `tunnels` 共用同一张注册表与那把锁，`len()` / `shutdown_all` / `remove_sftp` 一并接上）；
  只读探针 `sftp`；六条命令（比 ADR-0006 D7 原表多一条 `sftp_sessions`，理由见那里）。
- **前端**：`src/ipc/sftp.ts`（唯一的后端调用口）、`src/sftp/SftpPanel.tsx`（双栏：选主机 → 连接 →
  列目录 → 进目录 / 上级）、`TabStrip` 多一个入口、`App` 接线。面板是**仅渲染的视图**：关面板
  不停会话，所以打开时会先 `sftp_sessions` 接回已有会话，结束会话是面板里的显式动作。
- **一处上游事实（写进 ADR-0006 D2 的修订）**：`russh` 的 `request_subsystem` **只发送、不等回复**
  （走 `send_msg`）。于是"对端没开 SFTP"表现为**初始化会话等满期限**，而不是一句明确的拒绝 ——
  所以错误消息要把"多半是它没开 SFTP"说出来，且期限必须可调
  （`SshConnection::sftp_with_timeout`，否则负例只能靠等满 10 秒来证明）。
- **一处返工：E2E 第一版把基准写死了**。原先断言"SFTP 打开前只有一个终端标签页" —— 实测拿到
  **0**：各 E2E 目标**共用一个 app**，前面的目标（`tab_close` 那类）会把标签页关到零。
  改成**现读基准**（标签页数与会话数都相对"打开之前"断言），失败信息因此只说本次改动的事。
- **判据实测**（`sftp_dual_pane`，真实 app + 测试进程内**两台**服务端，**7.34 s**）：
  两侧各答一轮（主机密钥 + 口令，口令提示里的地址分别是两台服务端）→ probe
  `sftp = {"handle":27,"sides":[{"side":"left","name":"e2e-sftp-left","state":"connected","path":"/",…},
  {"side":"right",…}]}` → 左栏列出 `left-alpha.txt` / `left-dir`、右栏列出 `right-beta.txt`，
  且左栏**没有**右栏的条目 → 只连左侧时右侧仍是 `disconnected` → 两台服务端各记到 **1 次**
  `sftp` 子系统请求 → 打开前后**终端标签页数不变** → 注册表 `live`/`registered` 从 **1** 到 **2**
  再回到 **1** → 两台服务端各看到那条连接断开。
- **库内判据**：`akasha-ssh` 新增 2 条（`sftp_session`）—— 正例（列出的条目与对端给的一致、
  路径是 `realpath` 的结果、对端记到一次子系统请求）+ **负例**（服务端没开 sftp →
  `sftp_with_timeout(1)` 在 1 秒附近失败、错误里带"sftp 子系统"、且**没被记成认下**）。
- **门禁**：`just ready` **6/6**（其中 `deny-offline` 首次因新依赖的 wasm 目标 crate 需要写
  `~/.cargo/registry/cache` 而失败 —— 那是 `AGENTS.md` §1 记录的沙箱权限问题，提权重试后
  `bans / licenses / sources all ok`）；`just test` **340 passed**（+3）；`just test-e2e`
  退出码 **0**（27 个用例 / 21 个目标，新增 `sftp_dual_pane` **7.34 s**）；
  `pnpm build` 退出码 0（**865.52 kB / gzip 238.18 kB**，改前 859.44 / 236.54 —— 增量就是这个面板）。

