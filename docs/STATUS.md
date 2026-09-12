# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-12

## 一句话

**阶段 2「端到端最小终端」5/5 完成**；**阶段 3 已完成 2/5**：前端第一次有了**标签页**，
并且标签页与它自己的会话**同生命期**——两个方向都落地了：

| 方向 | 行为 |
|---|---|
| 用户关终端标签页（plan 0305） | **立刻丢弃**它自己的 `Session`（kill + wait 收尸、摘牌、撤销兜底登记）；别的标签页毫发无伤 |
| **会话自己结束**（敲 `exit` / shell 崩溃，plan 0306） | 后端**立刻收掉它**并发事件 → **标签页跟着关掉** |

"何时回收"因此有五种触发（前四种是进程/窗口级，最后一种粒度最小）：

| 触发 | 谁被回收 |
|---|---|
| 关一个终端标签页 / 会话自己结束 | **只有那一个会话** |
| 关窗口 / 正常退出 | 全部（`RunEvent::Exit` → `Sessions::shutdown_all()`） |
| panic | 全部（panic hook：打崩溃现场 → 回收 → `abort()`） |
| `tauri dev` 重载 / `kill -9` / `kill -TERM` | 全部 —— **另一个进程**：看门狗读到管道 EOF（ADR-0005） |

⚠️ **关标签页 ≠ 关窗口 ≠ 退出应用**：关掉**最后一个**标签页只是**空状态**（界面空了、进程留着）。
只有**三大终端**（local / ssh / serial）的标签页有关闭按钮；转发 / 密码库 / 文件传输是**仅渲染**的
视图标签页（**无关闭按钮**），关前端不影响后端执行 —— 见 `docs/scope.md` §5.6。

## ⚠️ UI 现状：**当前界面是功能验证壳层，不是设计稿**

**正式 UI 的布局 / 视觉 / 交互尚未有设计稿。** `src/**` 现有的界面（标签栏、状态栏、配色、
空状态文案）只有一个用途：让后端行为能被看见、能被验证。规则写在 `AGENTS.md` §4.0，
展开在 `docs/scope.md` §1.3。三句话：

- **不要**把当前界面当产品约束或"既有风格"，不要在它上面做视觉打磨；
  前端改动的判据是"**这条后端行为能不能被验证**"，不是"好不好看"。
- 后端**不得**依赖前端的呈现方式：界面整体重做时，命令 / 事件 / `Session` 状态机与收尾路径
  应当**原样可用**。
- 验证用的探针与选择器（`window.__akashaTerminal`、`.tab-pane.is-active …`）是**测试接口**，
  不是 UI 规范。**重做界面属于尚未规划的工作**（没有设计稿就没有验收标准，因此不进 ROADMAP）。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + gen-types-check + docs-check） | 退出码 **0**，**6/6 全绿** |
| `just test` | **70 tests run: 70 passed**（`akasha-pty` 37 + `akasha` 25 + `akasha-core` 8） |
| ↑ 本轮新增 | 3 条 `Sessions::retire` 单测：收干净（含撤销兜底登记）/ **幂等** / 收尸失败**也摘牌** |
| `just test-e2e`（自包含：起 Vite + app → 逐个目标 → 收尾） | 退出码 **0**，**10 个用例全绿**：`smoke` 3 / `integration` 2 / `session_channel` 1 / `terminal_render` 2 / `tab_close` 1 / `exit_residue` 1 |
| ↑ **关标签页 = 立刻丢弃会话**（0305，真 UI 点击） | ✅ 两个忽略 SIGHUP 的探针 → 点 `+` / 切换 / 点 `×`：探针 A 在 **83–93 ms** 内消失、**不需要第二次点击**；探针 B 仍在且屏幕内容还在；关掉最后一个 → 空状态 + 探针 B 也被丢；再开一个仍可交互 |
| ↑ **敲 `exit` → 标签页跟着关**（0306，反方向） | ✅ `在终端里敲 exit：标签页自己关掉（app 仍在）`；app 日志：`会话自己结束：已收掉并从登记簿摘牌 handle=8 session=8 retired.status=Some(Code(0))`；随后再开一个标签页可交互 |
| ↑ 退出零残留（0204/0205，未退化） | ✅ `app 已退出（pid 428）`；`✅ 零残留：忽略 SIGHUP 的 2153 已随会话被收掉` |
| ↑ 终端判据（未退化） | `renderer = webgl`、canvas 2 块、DOM 行容器 **0** 个；8 MB 分 **134 批**、之后仍可交互；`WEBGL_lose_context` 后退到 canvas **且屏幕内容保留** |
| ↑ 会话判据（未退化） | raw 通道 10.4 MB / 159 批，帧类型 = `ArrayBuffer`（JSON 帧 **0**）；收尾帧 1 个、console 零异常 |
| `pnpm build`（tsc + vite build） | 退出码 0；产物 **843 kB / gzip 231 kB**；生产包里 `akashaTerminal` / `activateProbe` / `mockIPC` 命中数 **0** |
| `just gen-types` / `just gen-types-check` | 本轮生成了**第一个事件**（`events.sessionEnded` / `SessionEnded` 类型），产物已提交 |
| `just check` / `just clippy`（`--workspace --all-targets`） | 退出码 **0** |
| `just deny-offline` | `bans ok, licenses ok, sources ok`（本轮**没有新增依赖**：事件手写 `impl`，不引 `derive` 特性） |
| `just docs-check` | 三部分全过（ROADMAP **54** 条目在 3 行内 / plan **47** 份 ≤200 行且索引一致） |
| `ast-grep scan` | 退出码 **0**；**四条**规则均已用正负例验证 |
| `cargo tree -p akasha-core` / `-p akasha-pty` \| `grep -c tauri` | **0** / **0**（分层成立） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数字，本轮未复跑**） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。

## 待验证（本地跑不了 / 沙箱跑不了）

- **CI 三个 job 是否真能变绿** —— 仓库还没有 remote，从没跑过。三条只在真 runner 上见分晓的
  风险记在 [`docs/plans/0102`](./plans/0102-ci-platform-matrix.md) 的实施记录里。
- **E2E 矩阵的三个格子**（Linux/xvfb + macOS + Windows，三格跑同一条 `just test-e2e`）——
  同上：没有 remote 就没跑过。本地只覆盖 **Linux/Wayland** 这一格。
  ⚠️ `tab_close` 的进程判据、"敲 exit" 与"会话级回收"在**非 Linux** 上本来就是缺口
  （见坑 #46 / plan 0204），CI 首跑时要盯这一格。
- **`tauri dev` 重载那条路径没有门禁**：只能手动实测（plan 0205 的实施记录里有脚本与输出）。
- **前端类型检查不在任何门禁里**：`just ready` 只覆盖 Rust + 文档，`pnpm build`（tsc）要手动跑。
  要不要把它并进门禁（牵涉 CI 的 checks job 是否 `pnpm install`）是**独立的一件事**。
- **在"有后台作业握着 PTY"的标签页里敲 `exit`**：不会有 EOF、不会关标签页（刻意，见坑 #55），
  但没有用例守着它。
- **宿主 MCP 连不到沙箱内运行的 app**（私有 PID / 临时目录）。沙箱内可用，
  但**必须让 app 与测试在同一次 bash 调用里**（坑 #33）。
- **`just dev-web` 的模拟后端没在真浏览器里点过**（本环境没有浏览器）。
- **大流量下的 JS heap 数字没取**。

## 当前基线（2026-09-12 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 出字节路径 | PTY read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write` |
| 前端结构与布局 | `src/tabs/TabStrip.tsx`（标签栏）+ `src/App.tsx`（标签模型，多面**同时挂载**、非活动的 `visibility: hidden` 叠放）+ `src/terminal/`（xterm 面与会话接线）+ `src/ipc/`（唯一的后端入口） |
| **关闭一个标签页** | 移除 → React 卸载该面 → `attachTerminal` 清理（**先** `close_session`，**后** `surface.dispose()`）→ `Sessions::close` → `Transport::shutdown()` → 撤销看门狗登记。**实测 83–93 ms** |
| **会话自己结束** | 合批读循环结束（EOF / EIO）→ `forward` 收工 → 收尾线程 `Sessions::retire`（收尸 + 摘牌 + `registry.close` + `forget`）→ `app.emit("session_ended", SessionEnded { handle, status })` → 前端关掉那个标签页 |
| 回收路径（进程内） | `RunEvent::Exit` / panic hook / `close_session` / `retire` → `Transport::shutdown()` |
| 回收路径（进程外） | 看门狗（每个 app 实例一个）读管道：`register` 写 `+<会话首进程 pid>`，收干净后写 `-<pid>`；**EOF = app 死了** → 逐个 `kill_session` |
| 事件通道 | `tauri-specta` 生成 `events.sessionEnded`（`src/ipc/bindings.ts`）；`Builder::mount_events` 在 `.setup()` 里必须调用（坑 #53） |
| 前端渲染器 | **WebGL**（WebKitGTK + MESA 软件栈下仍拿到 WebGL2）；`canvas` 元素 2 块；DOM 渲染器未启用 |
| 大输出实测 | 11.18 MB / 170 批（0202）；11.28 MB / 168 批（0204）；10.80 MB / 162 批（0205）；10.41 MB / 159 批（0305 复测） |
| 前端产物 | 843 kB（gzip 231 kB） |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| CSP | `csp`：`default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| 前提条件 | **需要能写 `$HOME`**；沙箱内会刷 `dconf-CRITICAL` 与 WebKit 缓存 hard-link 告警，但 app 仍正常起窗口 |

## 进行中 / 下一步

- [~] **plan 0102（CI 平台矩阵）**：本地部分完成，最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测 —— `just test-e2e` 自包含、
  **10 个用例全绿**；剩 CI 三平台格子（同上）
- [ ] **阶段 3 托盘**（[plan 0301](./plans/0301-tray-icon-menu.md)）：注意它建立在"退得干净"之上；
  而"点叉收托盘"要在 `ExitRequested` 里 `prevent_exit` —— 0204 的回收**故意**挂在 `RunEvent::Exit`，
  别挪到 `ExitRequested`
- [ ] 阶段 3 还要**复核 0205 的看门狗**：托盘时代"窗口关掉但进程还在"是常态，看门狗的
  生命周期仍然 = 一个 app 实例（ADR-0005 §6 的复审条件之一）
- [ ] 托盘时代还要**复核 0305/0306 的前提**：① 关窗口（收托盘）时标签页与它们的会话必须原样存活；
  ② "关最后一个标签页 = 空状态"在"窗口隐藏"成为常态之后是否仍然合适；
  ③ 会话自己结束时**窗口是隐藏的** —— 事件照样要送到前端（现在走 `AppHandle::emit`，不依赖窗口可见）
- [ ] **正式 UI**：等设计稿（见上面「UI 现状」）—— 没有验收标准，故**不进 ROADMAP**

### 本轮完成（plan 0306：会话自己结束 = 收掉它 + 关掉那个标签页）

- [x] `Sessions::retire(handle)`：摘牌 + 显式收尸 + `registry.close` + 撤销兜底登记；
  **幂等**（用户点 × 与 shell 自己退出可能撞在一起）、**不半途而废**（收尸失败也摘牌）
- [x] `SessionEnded` 事件 + `open_session` 的收尾线程（`join` 掉 `forward` 之后才收尾 + 发事件）
- [x] 前端订阅：一个全局监听 + `handle → 回调`表（含"事件早于订阅"的补发）；
  会话结束之后 `write` / `resize` / `close` 变成空操作
- [x] E2E：敲 `exit` → 标签页自己关掉 → 再开一个仍可用
- [x] 规范：`AGENTS.md` §3.3（事件表加一行 + "同生命期，两个方向"）、
  **§4.0 前端是验证壳层**；`docs/scope.md` §1.3（UI 现状）+ §5.6（反方向）

### 上一轮完成（plan 0305：关闭终端标签页 = 立刻丢弃该 Session）

- [x] 标签栏 + 多标签宿主（保持挂载、`visibility: hidden`）+ `×` 按 `kind` 渲染
- [x] `activateProbe`（探针跟活动面走）；拆渲染面兜异常 + 清理顺序"先交会话、后拆面"（坑 #50）
- [x] **发现并修掉**：丢过 WebGL 上下文的终端在 `term.dispose()` 时抛异常 → 关一个标签页
  会把整棵树卸载成空白，且会话回收被跳过

### 更早

- [x] **plan 0205 / 0204 / 0203 / 0202 / 0201 / 0107 / CI 去 Gitea 化 + 布局收口**
  （见 git 历史与各自的 `docs/plans/archive/`）

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接跑 `cargo …` 会失败（坑 #8），一律用 `just` 转发。⚠️ **临时脚本里也一样**：
  `cargo run` 的 cwd 必须是 `src-tauri/`。
- **三个 crate 的分工**：`akasha-core`（Session 模型，零依赖）、`akasha-pty`
  （`Transport` + portable-pty + 合批 + `teardown`（会话级回收）+ **`watchdog`**（进程外兜底））、
  `akasha`（app 包 = IPC 薄壳 + 退出钩子 + 看门狗接线 + 事件 + 代码生成 bin）。
- **前端四层**：`src/ipc/`（唯一允许碰后端，含会话事件订阅）、`src/tabs/`（标签栏）、
  `src/terminal/`（xterm 面与会话接线）、`src/App.tsx`（标签模型 = 谁在、谁是活动的）。
- **调试白屏**：Victauri 的 `logs {action:"console"}`；读不到"模块执行期就抛错"的失败 ——
  那时临时往 `index.html` 塞 `window.onerror` 钩子（坑 #35）。⚠️ **React 的 effect 清理函数里抛错
  会卸载整棵树**（坑 #50）—— "界面突然全空"要先怀疑它，而不是先怀疑样式。
- **调试 E2E / 真 app**：`just test-e2e` 的 app 日志落在 `$tmp/akasha-e2e-app.log`；
  在**同一个 bash 调用**里才能同时读到 app 与测试（坑 #33）。
- **文档三级粒度**：`ROADMAP.md`（判据）→ `docs/plans/TTxx-*`（手段）→ 本文件的坑（痕迹）。
  完成的 plan **整份移入 `docs/plans/archive/`**（不拼接、不追加）。
- 命令入口分两处：项目级在根 `justfile`，crate 级在 `src-tauri/justfile`。
  **权威清单在 `docs/just.md` §2**（21 个配方），由 `just docs-check` 强制同步。

## 踩过的坑（避免重复踩）

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency**。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**。
3. **just 的变量写 `$p`，不是 Make 的 `$$p`**。
4. **just 用 justfile 所在目录作为配方工作目录**。
5. **系统库缺失只在 cargo 构建脚本阶段暴露**；本机是 CachyOS（Arch 系），不是 apt。
6. **Tauri 没有 Rust 热重载，Victauri 也不提供**。
7. **just 的 shebang 配方需要可写的 runtime dir**，受限环境会失败。
8. **仓库根没有 `Cargo.toml`** → 根目录下一切 cargo 命令失败（**临时脚本也算**）。
9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** —— 跑一次 `just fmt`。
10. **CI 里 `libappindicator3-dev` 已不存在**，要用 `libayatana-appindicator3-dev`。
11. **受限环境下"写工作区之外被拒"看起来像工具/代码故障** —— 识别 → **直接提权重试**。
12. **`git checkout <file>` 会静默丢弃未提交的改动** —— 负例自检用 `cp` 备份/还原
    （⚠️ 备份别放 `/tmp`：沙箱每次调用一个私有 `/tmp`，下一次就找不到了）。
13. **正向校验若不限定到目标段落就形同虚设** —— `docs-check` 用 `awk` 取 §2。
14. **`docs-check` 的反向检查**已扩到 `AGENTS.md` / `ROADMAP.md` / `docs/**/*.md`。
15. **后台遗留的 `just dev` 会让 Vite 继续监听 1420 而 app 早已不在**。
16. **含反引号的 grep 模式在 justfile 配方里必须整体放进单引号**。
17. **多文件行数检查要逐文件取**（awk 的 `NR` 会跨文件累加）。
18. **`[profile.*]` 写在 workspace 成员里会被静默忽略**。
19. **整体 `mv` 构建缓存会留下写死的绝对路径**。
20. **`cargo` 在成员目录里只选当前包** —— crate 级配方不写 `--workspace` 会**静默漏掉**成员。
21. **tauri CLI 默认只监听 `src-tauri`** —— 成员放外面 = 开发循环静默失效。
22. **cargo 的空 glob 是硬错误**（`members = ["crates/*"]`）。
23. **justfile 里不能出现完整的 `{{ … }}`**（要写字面量用 `{{{{`）。
24. **"一份工作流喂两个 forge"是一笔持续交的税**。
25. **兼容层的遗产会以"看起来更稳"的样子留下来**。
26. **阻塞的 `Read` 与"按时间交付"天生冲突** —— 正解是读线程 + `recv_timeout(期限)`。
27. **零匹配的测试过滤器在 nextest 里是"报错"**。
28. **`cargo bench` 会顺带用 bench 模式跑一遍单测目标**（正常）。
29. **仓库里出现第二个 bin 会让 `tauri dev` 起不来** —— 修法是 `default-run`。
30. **只写 `path` 的依赖等于版本号写 `*`** —— path 依赖要**同时写 `version`**。
31. **`Channel<Vec<u8>>` 不是二进制通道** —— 真正走 raw 的只有
    `Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`。**别只看字节数验收**：
    小消息（<1 KiB）的 raw 帧曾变成 `number[]`，用例里要数"JSON 帧 == 0"。
32. **`u64` 不能直接过 IPC**：改用壳层 `u32` 句柄 + **checked** 转换。
33. **沙箱里 E2E 必须与 app 在**同一次** bash 调用内**（每次调用都是独立的 bwrap）。
34. **`pkill -f <模式>` 会匹配到自己** —— 用 `pkill -f '[v]ite'` 或按 PID/进程组杀。
35. **`@xterm/addon-unicode11` 需要 `allowProposedApi: true`** —— 不开的话 `loadAddon` 抛在
    React **effect** 里 → React 卸载整棵树 → 症状是**整屏白屏**且看不到报错。
36. **不在门禁里的测试等于没测**：`just test-e2e` 既不在 `ready` 里、又要真 app。
37. **`git mv` 之后 `docs-check` 会同时验两件事**（文件在不在、索引指得对不对）。
38. **每加一个依赖就多一份要维护的放行**：CSP 的 `style-src 'unsafe-inline'` 就是 xterm 逼出来的。
39. **"测试自己抛的异常"会污染同一 app 上后跑的用例**（raw 频道的收尾帧曾被当成数据帧）。
40. **vite 默认只监听 `[::1]:1420`**；`ls /tmp/victauri/<pid>/` 里的 `pid` 目录名**就是 app 的 pid**。
41. **`(cmd) &` 在 fish 里是命令替换，不是子 shell** —— E2E 敲进**用户登录 shell** 的命令必须
    在 fish / bash / sh 下语义相同（用 `sh -c '…' &`）。
42. **`cargo test` 一次收多个 `--test` 时按目标名字母序跑**，不按参数顺序 ——
    要按顺序就得**逐个目标各跑一条**。
43. **`tauri-plugin-log` 默认的 `TargetKind::LogDir` 会让"日志目录不可写"变成 app 打不开**。
44. **`tracing/log-always` 会把依赖树的 TRACE 一起转成 `log` 记录**（含 `tracing::span::active`）。
    默认级别下 app 日志被刷到几十万行并明显拖慢 app —— 级别要显式定。
45. **SIGKILL 的投递是异步的**：`kill()` 返回后立刻读 `/proc/<pid>/stat` 会读到 `R`，
    那不是"没杀掉"。判据必须等"消失"（有截止时间的轮询）。
46. **portable-pty(unix) 的 `Child::kill()` 不是纯 SIGKILL**：它先发 **SIGHUP**、等 5×50 ms
    宽限，再退到 SIGKILL；而且它只管那个 shell —— 会话里其他进程得自己收（plan 0204）。
47. **早于日志插件注册的 `tracing` 事件会静默消失**：看门狗的"已启动"记录因此推迟到 `.setup()`。
48. **`/proc/<pid>` 存在 ≠ 进程还活着**：僵尸（`Z`）也有目录项。判活要读
    `/proc/<pid>/stat` 的状态位；判自己的子进程结束要走 `wait`/`try_wait`。
49. **按"命令行里含某段文本"找进程会误伤**：沙箱包装进程（bwrap）自己的 cmdline 里带着
    整段脚本文本。要找探针就比对 **argv 恰好等于**那两个词。
50. **`term.dispose()`（xterm）会抛，而它跑在 React 的 effect 清理函数里** —— 触发状态是
    "WebGL 上下文丢过、已退到 canvas"的终端（`TypeError: … 'this._linkifier2.onShowLinkUnderline'`）。
    后果是**双重的**：① 抛在 effect 清理里 = React **卸载整棵树** → 关一个标签页整个界面变空白；
    ② 它若排在别的工作前面，后面的工作**永远执行不到**（实测：会话回收那一句被跳过）。
    两条正解：**清理函数里的"必做项"排在第三方拆解之前**，拆第三方时**兜住异常**。
51. **多标签之后 DOM 选择器不再唯一**：`document.querySelector('.xterm-helper-textarea')` 是"第一个
    标签页里的那个"，而探针全局若要指"最后挂载的面"就会与"用户正在看的面"分叉。
    正解：输入与探针都跟着**活动面**走（`.tab-pane.is-active …` + 一个显式的"交探针"动作）。
52. **断言超时不一定是"慢"**：本轮 E2E 报的"标签页只剩一个 超时"，真实原因是**界面已经被卸载**
    （读到的是 0）。把"0 意味着界面没了"写进失败消息里，能省掉一整轮误判。
53. **`tauri-specta` 的事件必须 `mount_events`**：命令靠 `invoke_handler` 就够了，事件漏了这一步
    **不在启动时报错**，而是在**发**的时候 panic（`EventRegistry not found`）。
54. **官方 `Channel` 不会告诉你"流结束了"**：收到收尾帧 `{index, end:true}` 时它只把回调注销掉
    （`cleanupCallback`），**不通知** `onmessage` —— 想看"会话结束"必须另发一个事件。
55. **会话里还有别的进程握着 PTY 时，主端读不到 EOF**：这时会话**不算**结束、标签页**不该**关
    （真实终端同样如此）。E2E 里"敲 exit"必须在**干净**的标签页里做。
56. **接线一次的回调必须走 `ref`**：`attachTerminal` 只在挂载时接到回调（`useEffect(…, [])`），
    而调用方每次渲染都给一个新箭头函数（它闭包着**当时**的列表）—— 直接接会走进过期闭包，
    表现是"晚发生的会话事件处理错了"（本轮是"标签页关不掉"，且只在多标签时出现）。
