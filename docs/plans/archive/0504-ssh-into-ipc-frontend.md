# Plan 0504: SSH 接进 IPC / 前端

- **关联**：ROADMAP 阶段 5 ·「SSH 接进 IPC / 前端」· ADR-0003（**实现中**）
- **前置**：plan 0502（连接 + 认证在 crate 层成立）· plan 0503（host key 的信任策略已定，
  未知 host key 为"提问"而不是"临时接受"）
- **状态**：已完成（2026-09-13）
- **本 plan 落 ADR-0003 的哪几条**：D2（runtime 归 app，启动时建**一个**专用 runtime）、
  D3（同步门面 + 异步在门后）、D11（策略接口接到前端）；新增 **D16**（提问往返的形状与超时）

## 目标

使**当前运行的 app** 能够打开一个 SSH 会话：从界面选一个主机 → 后端连接 → 终端字节双向流动。
`akasha-ssh` 已能连接与认证，但**没有任何命令或界面可达它**（plan 0502 / 0503 的判据只到 crate 层）。

以下三件事均为必需：

1. **一条带目标的命令**：`open_session` 目前只调用 `PtyTransport::spawn_default()`。
   `Sessions::register` 已为 `<T: Transport + 'static>`，因此 `SshTransport` 可进入同一个注册表 ——
   需要新增的只是"按目标选载体"（ADR-0003 D3 的落点）。
2. **一条"后端提问 → 前端作答"的往返协议**：凭据（口令 / 私钥口令 / keyboard-interactive）与
   未知 host key 都是**后端在连接过程中需要用户回答**的内容。事件与命令成对，且必须有超时与取消。
3. **一个提示界面**：达到验证壳层级别即可（`AGENTS.md` §4.0）—— 判据是"这条后端行为能被验证"。

## 非目标

- **正式 UI**：尚无设计稿（`scope.md` §1.3）。本 plan 只实现能被 E2E 驱动的提示与标签页。
- **跳板 / 转发 / SFTP**：plan 0505 与阶段 6 / 7 的内容。
- **连接复用**：`scope.md` §2.2 已定案不做。本 plan 只保证"同一主机第二次不再询问凭据"在**真实 app 上**成立。
- **解锁界面**：口令仍只从 `vault_unlock` 进入（plan 0407 的现状），E2E 用 `invoke_command` 解锁。
- **主机池的增删改查界面**：池的 CRUD 在阶段 4 已落地（crate 层），本 plan 只加一条**只读**列表。

## 判据（来自 ROADMAP）

真实 app 上开一个 SSH 会话：字节能双向流动（输入命令后看到输出）、凭据只问一次、关标签页零残留。

⚠️ 该判据需要**一台真实的 SSH 服务端**。做法：把 plan 0502/0503 用的**进程内服务端**从
`akasha-ssh/tests/support/` 提为 `akasha_ssh::testing`（与 `akasha_pty::testing` 同一个先例），
E2E 在自己的进程内启动它 —— 于是"服务端观察到的"与"界面观察到的"是同一件事的两种视角。

## 步骤

### 1. 库那一侧：可共享句柄 + `HostKeyCache` 适配器（plan 0503 预留的接口）

- `Vault { unlocked: Mutex<Option<Unlocked>> }` → `Vault { inner: Arc<Mutex<Option<Unlocked>>> }`
  （`#[derive(Default, Clone)]`，与 `Sessions` 形状一致）：`State<'_, Vault>` 无法借出 `'static` 句柄，
  而 `KnownHostsVerifier` 需存活至连接结束。
- 新增 `Vault::with_conn`：**短借**已解锁的连接（闭包内用完即还），返回 `VaultError`；新增 `VaultError::Locked`。⚠️ **不得在持锁期间连接** —— `remember` 会在连接过程中重新获取库锁。
- `HostKeyCache` 的适配器（`src-tauri/src/ssh.rs`）接到 `akasha_store::known_hosts::{lookup, remember}`。
  库错误 / 锁定 → `SshError::HostKeyCache`（`akasha-ssh` 新增变体）→ **拒绝连接**：
  信任记录与密钥池都在库中，"绕过缓存连接"将使 D11 降级。

### 2. 提问往返（事件 + 命令 + 超时）

- 事件 `ssh_prompt_request`（`#[serde(tag = "kind")]`：`hostKey` = host/port/algorithm/fingerprint；
  `credential` = target/哪一种/prompt）；事件 `ssh_prompt_dismissed`（超时或连接已结束时界面收起）。
- 回答命令三条：`ssh_prompt_credential { id, secret }`（口令是 `PassphraseInput`，**没有 `Debug`**）、
  `ssh_prompt_host_key { id, accept }`、`ssh_prompt_cancel { id }`。对过期 id 作答 → `PromptError::Gone`。
- **超时 120 s，超时即拒绝**（既不接受，也不留下悬置的连接）：该超时同时是"这一档可安全占用一个
  runtime worker"的依据（`check_server_key` 是同步回调，提问期间停留在此）。
- `Prompts` 的**发布口可注入**（`with_sink`）：单测中替换为记录并自动作答的闭包，不涉及 Tauri。

### 3. `open_ssh_session`：一条命令，两条线程，一条收尾路径

- **async command**（tauri 把 async command 交给 `async_runtime::spawn`，因此命令体**不在**主线程、
  不阻塞 IPC），其内部创建一条**普通 `std::thread`** 执行 `SshTransport::connect`
  （`RuntimeHandle::try_current()` 在 `spawn_blocking` 中**同样**返回 `Ok` → 触发 `BlockingInsideRuntime`），
  结果经 `tokio::sync::oneshot` 取回。
- app 在启动时创建**一个**专用 runtime（D2）；创建失败 → 降级（记录一条 `error` 日志 + SSH 命令返回
  明确错误），**不阻塞启动**（`AGENTS.md` §3.3）。
- 失败面收敛成 `SshIpcError`：`Locked` / `NoSuchHost` / `Failed { kind, message }`（`kind` 区分
  `hostKeyChanged` / `hostKeyRejected` / `auth` / `connect`）/ `Internal`。`HostKeyChanged` 的提示文本中
  **两个指纹都在**（D11 的告警），E2E 按 `kind` 断言而不是按字符串。
- 注册之后与本地终端**共用**同一条收尾路径（`forward` + 收尾线程 + `session_ended`），不复制第二份。

### 4. `vault_hosts`（只读）：界面据以选择主机

- 一个新命令 + `HostView { id, name, host, port, user }`。认证方式由池行决定：
  `password` → 不使用 agent、不带密钥；`agent` → 只用 agent；`publickey` + `key_id` → 该密钥
  （PEM 从库中取出 → 进入受保护页 → `KeyCandidate`）。跳板列（`jump_id`）本 plan 不使用（plan 0505）。

### 5. 前端：入口、提示面板、标签页

- 标签栏新增一个 SSH 入口（位于 `+` 旁）：列出 `vault_hosts` → 选一台 → 连接。
- 提示面板订阅上述两个事件：未知 host key 显示**待核对的那串指纹**（接受 / 拒绝）；
  凭据显示一句 prompt + 口令框（确定 / 取消）。答案经三条命令提交。
- `TabKind` 新增 `"ssh"`：**它同属"三大终端"，有关闭按钮**（`scope.md` §5.6）；
  `TerminalPane` 按目标选 `openTerminalSession`（PTY）还是 `openSshTerminalSession`。

### 6. E2E（真实 app + 进程内服务端）

`src-tauri/tests/ssh_session.rs`：自行构造库（v2，以只读方式检查，不使用用户的库）→ 写入一行主机，
指向服务端的随机端口 → `vault_unlock` → **在界面中打开** SSH → 回答两个提示 → 在终端中执行命令并
看到回声 → 再开一个（**零次提问**）→ 关闭标签页（服务端观察到连接断开）→ 删除自行构造的库。

## 验收命令

```bash
# 1) 生成物、文档与门禁（含新命令的 TS 生成物必须已提交）
just gen-types          # 新增 5 条命令 + 2 个事件写进 src/ipc/bindings.ts
just ready              # 期望：6/6 全绿；test 计数 = 244 上下（packages 见输出）

# 2) crate 层：SSH 三态 + 提问往返的单测（不需要 app）
cd src-tauri && cargo nextest run -p akasha-ssh      # 期望：27 passed（+ testing 提为公开模块后不变）

# 3) 真实 app 的 E2E（自包含：复用或自起 Vite + app，执行结束后回收）
just test-e2e           # 期望：stdout 出现 "── E2E: ssh_session ──" 且该目标通过

# 4) 前端类型检查（不在 just ready 里，必须手动跑）
pnpm build              # 期望：tsc + vite build 退出码 0

# 5) 关标签页零残留（进程表 + 后端会话表）
#    E2E 里已断言：服务端观察到连接断开；victauri app_state { probe: "sessions" } 回 { live: 0 }
```

单独运行第 3 条（不重新执行全部 E2E）时：

```bash
# 先 just dev 起 app（或让 test-e2e 自起），另开一个终端：
cd src-tauri && VICTAURI_E2E=1 cargo test --test ssh_session -- --test-threads=1 --nocapture
```

预期输出（节选，来自 E2E 自己打印的观察点）：

```
服务端：127.0.0.1:41xxx 指纹 SHA256:xxxx
SSH 提示：hostKey（指纹与上面一致）
SSH 提示：credential（password）
终端回声：ssh-hello
第二个会话：0 次提示，服务端第 2 次收到同一句口令
关标签页：服务端看到连接断开；sessions probe = {"live":1,"registered":1}（本地终端会话仍在）
库里 known_hosts：1 行（127.0.0.1:41xxx ssh-ed25519）
```

## 与 plan 0505 的接口对齐

`Handle` 的归属本 plan **未改动**：`SshTransport` 仍自己持有 `Handle`，并把它交给那条 `pump` task
（收尾的唯一出口在那里）。`direct_tcpip` 原语因此需要另外获得句柄。该归属由 **plan 0505** 定案：
选择"跳板那条路自己持有连接"（`connect_stream` 的用法），不采用由 `SshTransport` 暴露受控借用口
那条路。本 plan 仅记录该点。

## 实施记录（2026-09-13）

- **门禁**：`just ready` **6/6**；`cargo nextest run --workspace` = **235 passed**
  （akasha **65** + core 15 + pty 39 + store 89 + ssh 27；`akasha` 里 +7 = prompt 的 6 条单测
  + 新的 `ssh_session` E2E 目标）。`pnpm build`（tsc）退出码 0。
- **判据实测**（`just test-e2e` 里的 `ssh_session`，真实 app + 测试进程内的服务端，2.15 s）：
  界面选主机 → 主机密钥提示里的指纹**等于服务端的** → 接受 → 口令提示 → 填答 →
  终端敲 `echo ssh-hello` 看到回声（服务端也收到同一串）→ 库里 `known_hosts` **1 行** →
  第二个会话 **0 次提示**连上（服务端第 2 次收到同一句口令）→ 关标签页：
  `sessions` probe 回 `{"live":1,"registered":1}`、服务端看到 **2 条连接断开**。
- **`Cargo.lock` 只多一行**（`akasha` 依赖 `akasha-ssh`）：`russh` / `tokio` / `rand` 本来就在图里。
- **同时完成的三件事**：① plan 0502/0503 的服务端从 `tests/support/` 提成
  `akasha_ssh::testing`（app 的 E2E 也需要启动它，而 `tests/` 中的内容无法跨 crate 使用）；
  ② 新增 `sessions` probe（SSH **没有本地进程**，"关标签页零残留"在进程表上看不到）；
  ③ `session.rs` 抽出 `open_terminal` —— 本地与 SSH 共用同一条收尾路径。
- **已知问题**：① `i64` 过 IPC 被生成器拒绝（按 `SessionHandle` 用 `u32` 代理 + checked 转换）；
  ② 后端类型名里的 `View` 被 `no-ui-vocab-in-types` 拦截（`HostView` → **`HostEntry`**、
  `AuthView` → **`AuthMethod`**、`CredentialKindView` → **`SecretKind`**）；
  ③ React StrictMode 在 dev 中将 effect 执行两遍 → SSH 会话被打开两次，两次的提示**同时显示在同一面板**，
  而用户只回答其中一个（另一个持续等待）—— 表现为"点击 SSH 后一直无法连接"。处置：SSH 的连接
  **推迟一个微任务**再发出（同一次 commit 中排入的微任务在清理之后才执行），本地 PTY 不变。

## 文档同步

- ADR-0003：新增 **D16**（提问往返：事件 + 三条命令、120 s 超时、库锁定时不连接）、D2 标记为"已落地"、
  §11 补运行时 drop 顺序、§12 删掉已定的两条、§14 记一行。
- `ROADMAP.md` 勾选；本文件已 `git mv` 进 `archive/` 并同步索引；`docs/STATUS.md` 覆盖写；
  `docs/scope.md` §2.2 补一行"交互式提问"。
