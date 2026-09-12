# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-12

## 一句话

**阶段 2「端到端最小终端」5/5 完成**：输出合批（0201）、IPC raw 字节通道（0202）、
前端 xterm + WebGL 渲染（0203）、真正退出零残留（0204）、**被 SIGKILL 的退出路径也零残留（0205）**
全部落地并实测。四条回收路径现在是：

| 路径 | 谁在回收 |
|---|---|
| 关窗口 / 正常退出 | `RunEvent::Exit` → `Sessions::shutdown_all()` |
| panic | panic hook（打崩溃现场 → 回收 → `abort()`） |
| `tauri dev` 重编译重启 / `kill -9` / `kill -TERM` | **另一个进程**：看门狗读到管道 EOF → 收掉登记过的会话（ADR-0005） |

本轮最值钱的两条判据：① **"忽略 SIGHUP 的进程"**——普通 `sleep` 靠内核 session 级
SIGHUP 本来就不残留，拿它当判据等于没测；② **管道 EOF 与线程无关**——
`PDEATHSIG` 的"父"是创建进程的那个**线程**，从线程池里起会话就会误杀用户的会话。

`ROADMAP.md` 共 52 个条目（10 个阶段）：阶段 1 完成 5/7（CI 与 E2E 入口都待 CI 实跑），
阶段 2 完成 **5/5**。**CI 仍未真正跑过** —— 仓库没有配置任何 git remote。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + **gen-types-check** + docs-check） | 退出码 **0**，**6/6 全绿** |
| `just test` | **66 tests run: 66 passed**（`akasha-pty` 37 + `akasha` 21 + `akasha-core` 8） |
| ↑ 看门狗用例 | 8 条：协议往返/负例、EOF 收掉登记过的会话、**诱饵会话不被误杀**、`-` 撤销后原样活着、真管道"写完就死的写端"、`watch`/`forget` 的线上表示、argv 认领 |
| ↑ `tests/session_watchdog.rs`（**真 SIGKILL**） | ✅ 真 app 二进制（`CARGO_BIN_EXE_akasha`）跑成看门狗 + 真管道 + `holder.kill()`（SIGKILL）+ 忽略 SIGHUP 的探针 → 探针随会话消失、**诱饵毫发无伤**、看门狗自己收工。红-绿自检过：临时跳过 kill 时它报红 |
| **E2E 入口** `just test-e2e`（自包含：起 Vite + app → 逐个目标 → 收尾） | 退出码 **0**，**9 个用例全绿**：`smoke` 3 / `integration` 2 / `session_channel` 1 / `terminal_render` 2 / `exit_residue` 1 |
| ↑ 退出零残留（plan 0204，关窗口路径） | ✅ `app 已退出（pid 155）`；`✅ 零残留：忽略 SIGHUP 的 1460 已随会话被收掉`（**没被 0205 的改动带坏**） |
| ↑ 三条"没人能跑代码"的路径（plan 0205，手动实测） | ✅ A `tauri dev` 重载（cli 的 SIGKILL）：探针 479 消失、旧 app 184 消失、零 `sleep 600`；B `kill -9`：探针 1393 消失；C `kill -TERM`：探针 1746 消失。每条路径的 app 日志都有 `看门狗已启动 … watchdog=Some(N)` |
| ↑ panic 路径（plan 0204） | ✅ 临时把 `greet` 改成 panic 实测：hook 先打崩溃现场 → `shut_down=2 failures=[]` → 两条「会话已显式回收」→ `abort()` |
| ↑ 终端判据 | `renderer = webgl`、`canvas` 2 块、DOM 行容器 **0** 个；按键 → `akasha-probe-42` 出现在屏幕（求值结果，不是回显）；8 MB 分 **134 批**送达、之后仍可交互；`WEBGL_lose_context` 后退到 canvas **且屏幕内容保留** |
| ↑ 会话判据 | `yes \| head -c 10000000` 的 **10 802 602** 字节分 **162 批**送达 JS，帧类型 = `ArrayBuffer`（JSON 帧 **0** 个）；关闭后收到频道**收尾帧 1 个**，console 里**零 error** |
| ↑ 平台差异 | 本机（Wayland）`smoke::screenshot_captures_window` **显式跳过**并打印原因；`exit_residue` 与 `session_watchdog` 的强断言**只在 Linux** 跑（会话级回收只有 Linux 有实现），别处打印原因后跳过 |
| `pnpm build`（`tsc && vite build`） | 退出码 0；产物 839 kB / gzip 229 kB。生产包里**没有** mock 与探针（命中数 **0**） |
| **CSP** | `csp` 与 `devCsp` 均非 `null`；把 `devCsp` **临时设成与 `csp` 相同**也真跑过一轮 —— 渲染 / IPC / 8 MB 灌流全部照常、零 console error |
| **日志** | `tauri-plugin-log` 只挂 stdout、级别显式 `Info`：回收/panic/看门狗都有记录。⚠️ **早于插件注册的 `tracing` 事件会静默消失** —— 看门狗的启动记录因此推迟到 `.setup()`（坑 #47） |
| `just gen-types` / `just gen-types-check` | 生成 `src/ipc/bindings.ts`；比对通过（本轮**没有**加减 command，生成物无差异） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数字，本轮未复跑**） |
| `just check` / `just clippy`（`--workspace --all-targets`） | 退出码 **0** |
| `just deny-offline` | `bans ok, licenses ok, sources ok`（本轮**没有新增依赖**：看门狗只用 `std` + 已有的 `rustix`） |
| `just docs-check` | 三部分全过（ROADMAP **52** 条目在 3 行内 / plan **45** 份 ≤200 行且索引一致） |
| `ast-grep scan` | 退出码 **0**；**四条**规则均已用正负例验证 |
| `cargo tree -p akasha-core` / `-p akasha-pty` \| `grep -c tauri` | **0** / **0**（分层成立） |
| `just dev` / `just doctor` | 起窗口；Vite 1420；Victauri 发现文件写在 `/tmp/victauri/<pid>/`；`just doctor` 13/13（沙箱内） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。

## 待验证（本地跑不了 / 沙箱跑不了）

- **CI 三个 job 是否真能变绿** —— 仓库还没有 remote，从没跑过。三条只在真 runner 上见分晓的
  风险记在 [`docs/plans/0102`](./plans/0102-ci-platform-matrix.md) 的实施记录里。
- **E2E 矩阵的三个格子**（Linux/xvfb + macOS + Windows，三格跑同一条 `just test-e2e`）——
  同上：没有 remote 就没跑过。本地只覆盖 **Linux/Wayland** 这一格。
- **`tauri dev` 重载那条路径没有门禁**：`just test-e2e` 起的是它自己的 app，改不了源码去触发
  重载 —— 只能手动实测（plan 0205 的记录里有脚本与输出）。**机制**本身有门禁
  （`tests/session_watchdog.rs` 用真 SIGKILL 打真会话，在 `just test` 里）。
- **宿主 MCP 连不到沙箱内运行的 app**（私有 PID / 临时目录）。沙箱内可用，
  但**必须让 app 与测试在同一次 bash 调用里**（坑 #33）。
- **`just dev-web` 的模拟后端没在真浏览器里点过**（本环境没有浏览器）。
- **大流量下的 JS heap 数字没取**。

## 当前基线（2026-09-12 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 增量重编译 | 6.09–6.93s（纯逻辑 crate 改动同样触发） |
| 出字节路径 | PTY read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write` |
| 回收路径（进程内） | `RunEvent::Exit` / panic hook → `Sessions::shutdown_all()` → `Transport::shutdown()`（PTY：**收整个 session** → kill 子进程 → wait 收尸） |
| 回收路径（进程外） | 看门狗（每个 app 实例一个）读管道：`register` 写 `+<会话首进程 pid>`，收干净后写 `-<pid>`；**EOF = app 死了** → 逐个 `kill_session` |
| 看门狗进程的生命周期 | `main` 第一行认领 `--akasha-session-watchdog` → `setsid` 脱钩 → 阻塞在 `read_line` 直到 EOF → 收尾 → 退出。它**不输出任何东西**（醒来时没有读者） |
| 前端渲染器 | **WebGL**（WebKitGTK + MESA 软件栈下仍拿到 WebGL2）；`canvas` 元素 2 块；DOM 渲染器未启用 |
| 大输出实测 | 11.18 MB / 170 批（0202）；11.28 MB / 168 批（0204）；**10.80 MB / 162 批**（0205 复测） |
| 前端产物 | 839 kB（gzip 229 kB） |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| CSP | `csp`：`default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| 前提条件 | **需要能写 `$HOME`**；沙箱内会刷 `dconf-CRITICAL` 与 WebKit 缓存 hard-link 告警，但 app 仍正常起窗口 |

## 进行中 / 下一步

- [~] **plan 0102（CI 平台矩阵，GitHub Actions 一份）**：本地部分完成，
  最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测 ——
  `just test-e2e` 自包含、9 个用例全绿；剩 CI 三平台格子（同上）
- [ ] **阶段 3 托盘**（[plan 0301](./plans/0301-tray-icon-menu.md)）：注意它建立在
  "退得干净"之上；而"点叉收托盘"要在 `ExitRequested` 里 `prevent_exit` ——
  0204 的回收**故意**挂在 `RunEvent::Exit`，别挪到 `ExitRequested`
- [ ] 阶段 3 还要**复核 0205 的看门狗**：托盘时代"窗口关掉但进程还在"是常态，
  看门狗的生命周期仍然 = 一个 app 实例（ADR-0005 §6 的复审条件之一）

### 本轮完成（plan 0205：被 SIGKILL 的退出路径也零残留）

- [x] **ADR-0005**：`PDEATHSIG` / 完整 supervisor / 扫孤儿 / 单独 bin 逐条否掉；
  定案 = **伴生看门狗进程 + 单向管道协议**（触发信号是 EOF，由**进程的 fd 表**决定，
  与线程无关）
- [x] `akasha-pty::watchdog`：协议（`+pid` / `-pid`，数字逐位校验）、`run`（EOF → 逐个
  `kill_session`）、`detach`（`setsid`，^C 与关终端带不走它）、`SessionWatchdog`
  （起进程、`watch`/`forget`、收尸线程）
- [x] `Transport::session_leader()`（默认 `None`）：PTY 覆写成子进程 pid，与 `shutdown`
  用的是同一个 pid
- [x] `Sessions`：`attach_watchdog` + `register` **先登记再插表** + `close`/`shutdown_all`
  **收干净才撤销**（收尾失败不撤销）
- [x] 启动顺序：看门狗最前面起、日志推迟到 `.setup()`（坑 #47）
- [x] 测试三层：`akasha-pty` 真进程用例（含诱饵）、`SessionWatchdog` 协议断言、
  `tests/session_watchdog.rs`（真 SIGKILL + 真 app 二进制）+ 三条路径手动实测

### 上一轮完成（plan 0204：真正退出零残留）

- [x] `akasha-pty::teardown::kill_session`：Linux 扫 `/proc` 的 session id 逐个 SIGKILL
- [x] `Sessions::shutdown_all()`（幂等、失败记账不上抛）；挂 `RunEvent::Exit` + panic hook
- [x] E2E `exit_residue`；日志管道接通（`tauri-plugin-log` stdout + Info）

### 更早

- [x] **plan 0107 / 0203 / 0202 / 0201 / CI 去 Gitea 化 + 布局收口**（见 git 历史与各自的
  `docs/plans/archive/`）

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接跑 `cargo …` 会失败（坑 #8），一律用 `just` 转发。⚠️ **临时脚本里也一样**：
  `cargo run` 的 cwd 必须是 `src-tauri/`（本轮实测踩到：脚本在根目录跑 cargo，静默等 5 分钟）。
- **三个 crate 的分工**：`akasha-core`（Session 模型，零依赖）、`akasha-pty`
  （`Transport` + portable-pty + 合批 + `teardown`（会话级回收）+ **`watchdog`**（进程外兜底））、
  `akasha`（app 包 = IPC 薄壳 + 退出钩子 + 看门狗接线 + 代码生成 bin）。
- **前端三层**：`src/ipc/`（唯一允许碰后端）、`src/terminal/`、`src/App.tsx`。
- **调试白屏**：Victauri 的 `logs {action:"console"}`；读不到"模块执行期就抛错"的失败 ——
  那时临时往 `index.html` 塞 `window.onerror` 钩子（坑 #35）。
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
    在 fish / bash / sh 下语义相同（用 `sh -c '…' &`）。踩到的样子极具误导性：那一行把 shell
    **挂住**（命令替换要等 `sleep 600` 结束），于是**后面所有**"敲命令"的用例一起超时。
42. **`cargo test` 一次收多个 `--test` 时按目标名字母序跑**，不按参数顺序 ——
    会"关掉 app"的用例（`exit_residue`）会**第一个**跑。要按顺序就得**逐个目标各跑一条**。
43. **`tauri-plugin-log` 默认的 `TargetKind::LogDir` 会让"日志目录不可写"变成 app 打不开**。
44. **`tracing/log-always` 会把依赖树的 TRACE 一起转成 `log` 记录**（含 `tracing::span::active`）。
    默认级别下 app 日志被刷到几十万行并明显拖慢 app —— 级别要显式定。
45. **SIGKILL 的投递是异步的**：`kill()` 返回后立刻读 `/proc/<pid>/stat` 会读到 `R`，
    那不是"没杀掉"。判据必须等"消失"（有截止时间的轮询），不能立刻断言。
46. **portable-pty(unix) 的 `Child::kill()` 不是纯 SIGKILL**：它先发 **SIGHUP**、等 5×50 ms
    宽限，再退到 SIGKILL；而且它只管那个 shell —— 会话里其他进程得自己收（plan 0204）。
47. **早于日志插件注册的 `tracing` 事件会静默消失**：`tauri-plugin-log` 是 tauri builder
    的一环，在它注册之前 `tracing::info!` 没有 `log` 出口。看门狗的"已启动"记录因此
    推迟到 `.setup()`（先起、后记）。
48. **`/proc/<pid>` 存在 ≠ 进程还活着**：僵尸（`Z`）也有目录项。判"某个进程是否还活着"
    必须读 `/proc/<pid>/stat` 的状态位；判"我的子进程是否结束"要走 `wait`/`try_wait`
    （坑 #45 的同族，只是对象从"信号投递"换成了"父进程收尸"）。
49. **按"命令行里含某段文本"找进程会误伤**：沙箱包装进程（bwrap）自己的 cmdline 里带着
    整段脚本文本。要找探针就比对 **argv 恰好等于**那两个词。
