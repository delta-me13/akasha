# Plan 0107: E2E 入口（`just test-e2e` 自包含）

- **关联**：ROADMAP 阶段 1 ·「E2E 入口能真跑起来」
- **前置**：plan 0202 / 0203（E2E 目标已存在）、plan 0102（CI 矩阵）
- **状态**：进行中（本地已完成并实测；CI 三平台格子待首次推送实跑）
- **影响面**：`src-tauri/justfile`、`src-tauri/tests/{smoke,session_channel}.rs`、
  `.github/workflows/ci.yml`、`docs/just.md`

## 目标

一条 `just test-e2e` 跑完真 app 上的验收：不需要谁先手动起 app，也不需要记住
`VICTAURI_E2E=1` 与 `--test-threads=1`。判据（ROADMAP 条目）：

1. **没有任何 app 在跑**时，`just test-e2e` 退出码 0（自己起、自己收）；
2. 已经有 app 在跑（`just dev`）时**复用**它，且不碰别人的进程；
3. 平台能力不足的用例**显式跳过并写明原因**（不是静默放过，也不是把红说成绿）；
4. `tests/` 下新增目标不接入就红 —— 坑 #36（"生成了用例但没人跑"）不许复发。

## 非目标

- **不在本地覆盖多平台**：本机是 Wayland，窗口截图那条路径在这里拿不到原生句柄；
  覆盖面交给 CI 矩阵（Linux/xvfb = X11 + Windows + macOS），本地只跑"这个环境能跑的"。
- **不把 E2E 塞进 `just ready`**：它要真 app + Vite dev server，与 `ready` 的
  "秒级、随时可跑"定位冲突。入口独立，由 CI 的 `e2e` job 与本地按需调用。
- **不做"每个用例一个全新 app"的隔离**：一个 app 跑完全部目标；跨用例污染修在源头
  （探针不许往 console 里丢异常），而不是靠给每条用例重启一次 app 掩盖。

## 前置检查

```bash
just --list                 # test-e2e 在列
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:1420/   # Vite 的地址：注意是 localhost 不是 127.0.0.1
```

## 步骤

1. **修探针**（`tests/session_channel.rs`）：频道的收尾帧是 `{index, end:true}`、
   没有 `message` —— 官方 `Channel` 正是先判 `'end' in raw` 再取 `raw.message`。
   探针少了这一跳就在 `buf.byteLength` 上抛 `TypeError`，而它**不会**让本用例红，
   只会留在 webview 的 console 里，把同一个 app 上后跑的 `smoke::ipc_integrity_passes`
   （no console errors）弄红。修在源头：非数据帧直接跳过，并断言收尾帧确实到达。
2. **补 smoke 的两处**：`get_plugin_info` 断言 `app.identifier`（端口上可能坐着别的
   Victauri app，AGENTS.md §7）；截图用例在**拿不到原生句柄的环境**里显式跳过并说明；
   失败时把 `detail` 一起打出来（只报"哪条检查失败"定位不到东西）。
3. **配方自包含**（`src-tauri/justfile`）：有 app 就复用；没有就起 Vite（1420，
   `node_modules/.bin/vite`，自己持有 pid）+ `cargo run`（debug = `cfg(dev)` → 走 devUrl），
   从 discovery 目录拿 app 的 pid 收尾（不能收 `cargo run` 的 pid，那会留孤儿）。
   前端必须走 **Vite dev**：`window.__akashaTerminal` 探针只在 dev 构建里注册。
4. **目标清单与 guard**：`E2E_TARGETS` 列出目标；与 `tests/*.rs` 对账，漏了就红。
5. **CI 换成同一条命令**：`e2e` job 扩成三平台矩阵，三格跑的**都是** `just test-e2e`
   （Linux 那格套 `xvfb-run`，于是 X11 的原生句柄路径真的被跑到）。

## 验收命令

```bash
just test-e2e      # 期望：退出码 0；六个用例全绿；截图的跳过原因出现在输出里
```

- 期望输出（本机实测）：`integration 2 passed` / `session_channel 1 passed` /
  `smoke 3 passed` / `terminal_render 2 passed`，附
  `跳过 screenshot：Wayland 会话…`（XVFB / Windows / macOS 上这条会真跑）。
- 复用路径：先 `just dev`，再 `just test-e2e` → 打印 `→ 复用已在运行的 app`，
  且退出后 `just dev` 的窗口还在。
- guard：往 `src-tauri/tests/` 扔一个 `probe.rs` → `just test-e2e` 立刻红
  （不接进 `E2E_TARGETS` 就跑不到）。

## 回滚

配方退回"要求 app 已经在跑"的旧形态（`VICTAURI_E2E=1 cargo test --test smoke --test integration`），
CI 的 `e2e` job 退回单平台。用例侧的探针修复与断言应当保留 —— 那两条本来就该在。

## 实施记录

**为什么旧入口等于没有**：`test-e2e` 既不在 `ready` 里、又要一个真 app，于是它那几个
用例自生成后没人跑过（坑 #36）。本轮实测三处先前就红：

| 用例 | 症状 | 定性 |
|---|---|---|
| `smoke::screenshot_captures_window` | `cannot get window handle: unsupported window handle type on this platform` | **环境能力**：本机是 Wayland，webview 是 Wayland surface，而 Victauri 只认 Xlib/Xcb/Win32/AppKit |
| `smoke::ipc_integrity_passes` | `[FAIL] no console errors` | **用例间污染**：0202 探针把收尾帧当数据帧，抛 `TypeError` 留在 console 里 |
| `integration::command_greet` | 缺 `name` 参数 | 桩本身写错（本轮之前已修） |

**踩到的坑（已进 STATUS）**：`vite` 默认只监听 `[::1]:1420`，用 `127.0.0.1` 探活会得到
"起不来"的假象 —— 端口探活一律写 `localhost`。另外 `--nocapture` 是必需的：
跳过原因走 `eprintln!`，不打开它，"跳过"和"跑过了"在摘要里长得一模一样。

**还没验证的**：CI 三个 E2E 格子（本机没有 remote，也没法模拟 Windows/macOS）。
仓库首次推送时要盯的就是这里 —— 尤其 Windows（Git Bash 下的判活/收尾走
`tasklist` / `taskkill`）与 macOS 的 Vite 监听地址。
