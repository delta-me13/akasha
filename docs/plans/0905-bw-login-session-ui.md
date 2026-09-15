# Plan 0905: 登录 / 解锁 / 锁定接进前端（含自托管）

- **关联**：ROADMAP 阶段 9 ·「登录 / 解锁 / 锁定接进前端（含自托管）」
- **前置**：plan 0902（CLI 能用了）· [ADR-0007](../adr/0007-bitwarden-cli-acquisition.md)
- **状态**：未开始

## 目标

用户从界面上把 Bitwarden 这条链走通：**指定服务器（含自托管）→ 登录 → 解锁 → 锁定 / 登出**，
并且界面上的状态永远是 `bw` 自己说的那一个。

| 要发生什么 | 由谁负责 |
|---|---|
| 服务器地址可设（官方云默认 / 自托管 URL），明文 HTTP 在设置那一步就被拒 | plan 0902 的 `set_server` + 本条的界面 |
| 登录（邮箱 + 主密码；两步验证可选）→ 交出 session key | `akasha-bw` 的 `login` |
| 解锁、锁定、登出 | 同上 |
| session key **只在内存**、住在受保护页里 | ADR-0007 D7 |
| 三态（未登录 / 已登录未解锁 / 已解锁）与 `bw status` 一致，外部锁过也跟得上 | ADR-0007 D10（本条的同步） |
| 两个轴的切换与"下载一份"的动作在同一个面板上 | plan 0902 + 本条界面 |

## 非目标

- 只读导入 `sshKey` 条目（plan 0903）与离线缓存（plan 0904）—— 本条只到"拿得到 session key"。
- **API key 与 SSO 登录**：上游文档里有（`--apikey` / `--sso`），但它们各自的往返
  （浏览器跳转、`client_id` / `client_secret` 的存放）是另一件事，本条只做邮箱 + 主密码 + 两步验证。
- 不自动解锁、不做空闲超时（与库的锁定口径一致：只有显式动作）。
- 不把 session key 落盘、不进日志、不进事件载荷。

## 步骤

1. **app 侧的状态**（`src-tauri/src/bitwarden.rs`）：`BwState { settings, session: Option<Session>, last: Option<Status> }`
   一条长驻 `Mutex`（`bw` 自己没有任何互斥，两次调用同时写 `data.json` 会互相覆盖）。
2. **状态同步**（ADR-0007 D10）：每次动作**之前**读一次 `bw status --raw`，**之后**再读一次；
   `status` 说不是 `unlocked` 时立刻 `session = None`（外部用 `bw lock` 锁过之后，
   我们手里的旧 key 已失效，留着只会让下一条命令以 `You are not logged in.` 失败）。
3. **命令**：`bw_status` · `bw_server_get` · `bw_server_set` · `bw_login` · `bw_unlock` ·
   `bw_lock` · `bw_logout` · `bw_sync`。错误一律翻成 `BwIpcError { kind, message }`，
   `kind` 与 `akasha_bw::BwError` 的域一一对应（前端按 `kind` 分辨，不匹配消息字符串）。
4. **前端**：`src/ipc/bitwarden.ts`（唯一允许碰后端的目录）+ `src/bitwarden/BwPanel.tsx`
   （应用级浮层，与隧道 / SFTP 面板同类）。面板上四块：**CLI 那一块**（两个轴、版本、
   变体、许可证提示、下载按钮）、**服务器那一块**、**登录 / 解锁那一块**、**状态那一块**。
   选择器用 `data-bw-*`（测试接口，不是 UI 规范，`AGENTS.md` §4.0）。
5. **E2E**（`tests/bitwarden_login.rs`）：假上游（`akasha_bw::testing`）下载 → 面板显示版本与
   变体 → 设自托管地址 → 登录（对着假上游必然失败，断言**错误分类**与界面上那句话）→
   手工把状态摆成"已登录"（`bw` 的状态文件由用例自己写）→ 面板显示 `locked` →
   跑 `bw lock` 之后界面仍说 `locked`，且 session 那一路为空。登记进 `src-tauri/justfile`
   的 `E2E_TARGETS`。

## 验收命令

```bash
# 1) app 层单测：状态同步（status 说 locked 就丢 key）与错误分档
cargo nextest run --package akasha --lib bitwarden
# 预期：N passed（含"外部锁过之后 token 被丢掉"这条）

# 2) 门禁与前端类型检查
just ready
pnpm build
# 预期：just ready 6/6；pnpm build 退出码 0

# 3) 端到端（`just test-e2e` 自起 app；已有 `just dev` 则复用）
just test-e2e
# 预期：exit 0，且 ── E2E: bitwarden_login ── 打印：
#   下载前 status.problem = "这台机器上没有 bw：PATH 里找不到它"
#   下载后 version = <假上游的版本>、variant = oss
#   设自托管后 serverUrl = http://127.0.0.1:<port>（界面显示读回来的那个）
#   登录失败那句经 kind 分档（不是"失败"两个字的通用档）
#   面板上的三态与 bw status --raw 逐字一致

# 4) 真实路径（Victauri）：对着**真实**上游下载之后
#    invoke_command: bw_login {email, password}  → 失败，但 kind 是网络/凭据那一档，
#    且界面上那句话与 bw 自己说的对得上（措辞不匹配消息字符串，只看 kind）
```

## 回滚

- 摘掉面板与八条命令即可；`akasha-bw` 那条链不受影响（它有自己的测试）。
- 状态那一层是纯内存的，没有要清理的磁盘残留（CLI 自己的 `data.json` 归 CLI）。

## 实施记录

（落地时补：实际输出、与计划的差异、留下的验证空缺。）
