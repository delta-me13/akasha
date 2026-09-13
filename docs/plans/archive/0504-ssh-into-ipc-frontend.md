# Plan 0504: SSH 接进 IPC / 前端

- **关联**：ROADMAP 阶段 5 ·「SSH 接进 IPC / 前端」· ADR-0003（**实现中**）
- **前置**：plan 0502（连接 + 认证在 crate 层成立）· plan 0503（host key 的信任策略已定，
  未知 host key 是"问"而不是"临时接受"）
- **状态**：已完成（2026-09-13）
- **本 plan 落 ADR-0003 的哪几条**：D2（runtime 归 app，启动时建**一个**专用 runtime）、
  D3（同步门面 + 异步在门后面）、D11（策略接口接到前端）；新增 **D16**（提问往返的形状与超时）

## 目标

让**当前运行的这个 app** 能开一个 SSH 会话：从界面选一个主机 → 后端连上 → 终端里字节能双向流。
今天 `akasha-ssh` 已经能连能认证，但**没有任何命令或界面碰得到它**（plan 0502 / 0503 的判据只到 crate 层）。

三件事，缺一不可：

1. **一条带目标的命令**：`open_session` 今天只会 `PtyTransport::spawn_default()`。
   `Sessions::register` 已经是 `<T: Transport + 'static>`，所以 `SshTransport` 进同一个注册表 ——
   要加的只是"按目标选载体"（ADR-0003 D3 的落点）。
2. **一条"后端问 → 前端答"的往返协议**：凭据（口令 / 私钥口令 / keyboard-interactive）与
   未知 host key，都是**后端在连接中途需要用户回答**。事件 + 命令成对，且必须有超时与取消。
3. **一个提示界面**：验证壳层级别即可（`AGENTS.md` §4.0）—— 判据是"这条后端行为能被验证"。

## 非目标

- **正式 UI**：没有设计稿（`scope.md` §1.3）。这里只做能被 E2E 驱动的提示与标签页。
- **跳板 / 转发 / SFTP**：plan 0505 与阶段 6 / 7 的事。
- **连接复用**：`scope.md` §2.2 已定案不做。本 plan 只保证"同一主机第二次不再问凭据"在**真 app 上**成立。
- **解锁界面**：口令仍然只从 `vault_unlock` 进来（plan 0407 的现状），E2E 用 `invoke_command` 解锁。
- **主机池的增删改查界面**：池的 CRUD 在阶段 4 已落地（crate 层），本 plan 只加一条**只读**列表。

## 判据（来自 ROADMAP）

真 app 上开一个 SSH 会话：字节能双向流（敲命令看到输出）、凭据只问一次、关标签页零残留。

⚠️ 它需要**一台真的 SSH 服务端**。做法：把 plan 0502/0503 用的**进程内服务端**从
`akasha-ssh/tests/support/` 提为 `akasha_ssh::testing`（与 `akasha_pty::testing` 同一个先例），
E2E 在自己的进程里起它 —— 于是"服务端看到的"与"界面看到的"是同一件事的两种观察。

## 步骤

### 1. 库那一侧：可共享句柄 + `HostKeyCache` 适配器（plan 0503 留下的口子）

- `Vault { unlocked: Mutex<Option<Unlocked>> }` → `Vault { inner: Arc<Mutex<Option<Unlocked>>> }`
  （`#[derive(Default, Clone)]`，照 `Sessions` 的形状）：`State<'_, Vault>` 借不出 `'static` 句柄，
  而 `KnownHostsVerifier` 要活到连接结束。
- 新增 `Vault::with_conn`：**短借**解好的连接（闭包内用完即还），返回 `VaultError`；
  新增 `VaultError::Locked`。⚠️ **绝不在持锁期间连接** —— `remember` 会在连接中途回头锁库。
- `HostKeyCache` 的适配器（`src-tauri/src/ssh.rs`）接到 `akasha_store::known_hosts::{lookup, remember}`。
  库错 / 锁着 → `SshError::HostKeyCache`（`akasha-ssh` 新增变体）→ **拒绝连接**，
  因为信任记录与密钥池都在库里，"绕过缓存连上"等于把 D11 降级。

### 2. 提问往返（事件 + 命令 + 超时）

- 事件 `ssh_prompt_request`（`#[serde(tag = "kind")]`：`hostKey` = host/port/algorithm/fingerprint；
  `credential` = target/哪一种/prompt），事件 `ssh_prompt_dismissed`（超时或连接已结束 → 界面收起）。
- 回答命令三条：`ssh_prompt_credential { id, secret }`（口令是 `PassphraseInput`，**没有 `Debug`**）、
  `ssh_prompt_host_key { id, accept }`、`ssh_prompt_cancel { id }`。答一个过期的 id → `PromptError::Gone`。
- **超时 120 s，超时即拒绝**（不是接受、也不留一个悬着的连接）：它同时是"这一档能安全地占住一个
  runtime worker"的依据（`check_server_key` 是同步回调，提问期间就停在那儿）。
- `Prompts` 的**发布口可注入**（`with_sink`）：单测里换成一个记录 + 自动作答的闭包，不碰 Tauri。

### 3. `open_ssh_session`：一条命令，两条线程，一条收尾尾巴

- **async command**（tauri 把 async command 丢给 `async_runtime::spawn`，所以命令体**不在**主线程、
  不挡 IPC），内部起一条**普通 `std::thread`** 跑 `SshTransport::connect`
  （`RuntimeHandle::try_current()` 在 `spawn_blocking` 里**也是** `Ok` → 会吃 `BlockingInsideRuntime`），
  结果经 `tokio::sync::oneshot` 拿回来。
- app 在启动时建**一个**专用 runtime（D2）；起不来 → 降级（`error` 一条日志 + SSH 命令返回明确错误），
  **不挡启动**（`AGENTS.md` §3.3）。
- 失败面收敛成 `SshIpcError`：`Locked` / `NoSuchHost` / `Failed { kind, message }`（`kind` 区分
  `hostKeyChanged` / `hostKeyRejected` / `auth` / `connect`）/ `Internal`。`HostKeyChanged` 的文案里
  **两个指纹都在**（D11 的警报），E2E 按 `kind` 断言而不是按字符串。
- 注册之后与本地终端**共用**同一条尾巴（`forward` + 收尾线程 + `session_ended`），不抄第二份。

### 4. `vault_hosts`（只读）：界面凭什么选主机

- 一个新命令 + `HostView { id, name, host, port, user }`。认证方式由池行决定：
  `password` → 不用 agent、不带钥匙；`agent` → 只用 agent；`publickey` + `key_id` → 那一把钥匙
  （PEM 从库里取出 → 进受保护页 → `KeyCandidate`）。跳板列（`jump_id`）本 plan 不用（0505）。

### 5. 前端：入口、提示面板、标签页

- 标签栏多一个 SSH 入口（`+` 旁边）：列出 `vault_hosts` → 选一台 → 连接。
- 提示面板订阅上面两个事件：未知 host key 显示**要核对的那串指纹**（接受 / 拒绝）；
  凭据显示一句 prompt + 口令框（确定 / 取消）。答案走三条命令。
- `TabKind` 新增 `"ssh"`：**它同属"三大终端"，有关闭按钮**（`scope.md` §5.6）；
  `TerminalPane` 按目标选 `openTerminalSession`（PTY）还是 `openSshTerminalSession`。

### 6. E2E（真 app + 进程内服务端）

`src-tauri/tests/ssh_session.rs`：自己造库（v2，只读地检查不是用户的库）→ 灌一行主机指向
服务端的随机端口 → `vault_unlock` → **界面点开** SSH → 答两个提示 → 终端里敲命令看到回声 →
再开一个（**一次都不问**）→ 关标签页（服务端看到连接断开）→ 删掉自己造的库。

## 验收命令

```bash
# 1) 生成物、文档与门禁（含新命令的 TS 生成物必须已提交）
just gen-types          # 新增 5 条命令 + 2 个事件写进 src/ipc/bindings.ts
just ready              # 期望：6/6 全绿；test 计数 = 244 上下（packages 见输出）

# 2) crate 层：SSH 三态 + 提问往返的单测（不需要 app）
cd src-tauri && cargo nextest run -p akasha-ssh      # 期望：27 passed（+ testing 提为公开模块后不变）

# 3) 真 app 的 E2E（自包含：复用/自起 Vite + app，跑完收掉）
just test-e2e           # 期望：stdout 出现 "── E2E: ssh_session ──" 且该目标通过

# 4) 前端类型检查（不在 just ready 里，必须手动跑）
pnpm build              # 期望：tsc + vite build 退出码 0

# 5) 关标签页零残留（进程表 + 后端会话表）
#    E2E 里已断言：服务端观察到连接断开；victauri app_state { probe: "sessions" } 回 { live: 0 }
```

单跑第 3 条时（不想重跑全部 E2E）：

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
关标签页：服务端看到连接断开；sessions probe = {"live":0,"registered":0}
库里 known_hosts：1 行（127.0.0.1:41xxx ssh-ed25519）
```

## 与 plan 0505 的接口对齐

`Handle` 的归属本 plan **没改**：`SshTransport` 仍自己持有 `Handle`，并把它交给那条 `pump` task
（收尾的唯一出口在那里）。0505 的 `direct_tcpip(&Handle, host, port)` 因此要**另外**拿到句柄 ——
要么由 `SshTransport` 暴露一条受控的借用口，要么跳板那条路自己持有连接（`connect_stream` 的用法）。
本 plan 只把它写清楚：**别两边各定一套**。

## 实施记录（2026-09-13）

- **门禁**：`just ready` **6/6**；`cargo nextest run --workspace` = **235 passed**
  （akasha **65** + core 15 + pty 39 + store 89 + ssh 27；`akasha` 里 +7 = prompt 的 6 条单测
  + 新的 `ssh_session` E2E 目标）。`pnpm build`（tsc）退出码 0。
- **判据实测**（`just test-e2e` 里的 `ssh_session`，真 app + 测试进程内的服务端，2.15 s）：
  界面选主机 → 主机密钥提示里的指纹**等于服务端的** → 接受 → 口令提示 → 填答 →
  终端敲 `echo ssh-hello` 看到回声（服务端也收到同一串）→ 库里 `known_hosts` **1 行** →
  第二个会话 **0 次提示**连上（服务端第 2 次收到同一句口令）→ 关标签页：
  `sessions` probe 回 `{"live":1,"registered":1}`、服务端看到 **2 条连接断开**。
- **`Cargo.lock` 只多一行**（`akasha` 依赖 `akasha-ssh`）：`russh` / `tokio` / `rand` 本来就在图里。
- **顺手做掉的三件事**：① plan 0502/0503 的服务端从 `tests/support/` 提成
  `akasha_ssh::testing`（app 的 E2E 也要起它，而 `tests/` 里的东西跨不了 crate）；
  ② 新增 `sessions` probe（SSH **没有本地进程**，"关标签页零残留"在进程表上看不到）；
  ③ `session.rs` 抽出 `open_terminal` —— 本地与 SSH 共用同一条收尾尾巴。
- **踩到的坑**：① `i64` 过 IPC 被生成器拒绝（照 `SessionHandle` 用 `u32` 代理 + checked 转换）；
  ② 后端类型名里的 `View` 被 `no-ui-vocab-in-types` 拦下（`HostView` → **`HostEntry`**、
  `AuthView` → **`AuthMethod`**、`CredentialKindView` → **`SecretKind`**）；
  ③ React StrictMode 在 dev 里把 effect 走两遍 → SSH 会话被开两次，两次的提示**叠在同一个面板**
  里，而用户只答其中一个（另一个一直等）—— 表现为"点了 SSH 一直连不上"。处置：SSH 那条路的
  连接**推迟一个微任务**再发（同一次 commit 里排的微任务在清理之后才跑），本地 PTY 不变。

## 记账

- ADR-0003：新增 **D16**（提问往返：事件 + 三条命令、120 s 超时、库锁着不连）、D2 标"已落地"、
  §11 补运行时 drop 顺序、§12 删掉已定的两条、§14 记一行。
- `ROADMAP.md` 勾选；本文件已 `git mv` 进 `archive/` 并同步索引；`docs/STATUS.md` 覆盖写；
  `docs/scope.md` §2.2 补一行"交互式提问"。
