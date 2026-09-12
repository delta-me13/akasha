# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-11

## 一句话

**阶段 2「端到端最小终端」走到 3/4**：输出合批（0201）、IPC raw 字节通道（0202）、
**前端 xterm + WebGL 渲染（0203）** 都已落地并实测 —— 现在能开一个真 shell、敲命令、
看到输出；8 MB 灌下来界面不卡死，WebGL 丢了上下文会自动退到 canvas。
下一步是 [`docs/plans/0204`](./plans/0204-exit-zero-residue.md)（真正退出零残留）。

本轮顺带把 **`csp` 从 `null` 换成了最小放行**（`AGENTS.md` §4.3 的要求）：
`csp` / `devCsp` 两处，只差 Vite HMR 用的 `ws://localhost:1420`。

`ROADMAP.md` 共 50 个条目（10 个阶段）：阶段 1 完成 5/6（剩 CI 实跑），
阶段 2 完成 **3/4**。**CI 仍未真正跑过** —— 仓库没有配置任何 git remote。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + **gen-types-check** + docs-check） | 退出码 **0**，6/6 全绿 |
| `just test` | **47 tests run: 47 passed**（`akasha` 13 + `akasha-core` 8 + `akasha-pty` 26） |
| **终端 E2E** `VICTAURI_E2E=1 cargo test --test terminal_render -- --test-threads=1` | **2 passed**（真 app）：`renderer = webgl`、`canvas` 2 块、DOM 行容器 **0** 个；按键 → `akasha-probe-42` 出现在屏幕（求值结果，不是回显）；8 MB 分 **135–141 批**送达、排空哨兵出现、队列归零、**之后仍可交互**；`WEBGL_lose_context` 后退到 canvas **且屏幕内容保留** |
| **会话 E2E** `VICTAURI_E2E=1 cargo test --test session_channel` | **1 passed**：`yes \| head -c 10000000` 的 11 179 553 字节分 170 批送达 JS，帧类型 = `ArrayBuffer`（JSON 帧 0 个） |
| `pnpm build`（`tsc && vite build`） | 退出码 0；产物 839 kB / gzip 229 kB。生产包里**没有** mock 与探针（`模拟后端` / `mockIPC` / `__akashaTerminal` 命中数 **0**） |
| **CSP** | `csp` 与 `devCsp` 均非 `null`；把 `devCsp` **临时设成与 `csp` 相同**也真跑过一轮 —— 渲染 / IPC / 8 MB 灌流全部照常、零 console error（即生产那条字符串是被跑过的） |
| `just gen-types` / `just gen-types-check` | 生成 `src/ipc/bindings.ts`；比对通过（生成物已提交） |
| `just bench`（criterion，配方 #20） | 52.7 GiB/s（容量路径）/ 14.1 ns 每批 / 9.64 GiB/s（**0201 的数字，本轮未复跑**） |
| `just check` / `just clippy`（`--workspace --all-targets`） | 退出码 **0** |
| `just deny-offline` | `bans ok, licenses ok, sources ok` |
| `just docs-check` | 三部分全过（ROADMAP 50 条目在 3 行内 / plan 43 份 ≤200 行且索引一致） |
| `ast-grep scan` | 退出码 **0**；**四条**规则均已用正负例验证 |
| `cargo tree -p akasha-core` / `-p akasha-pty` \| `grep -c tauri` | **0** / **0**（分层成立） |
| `just dev` | 起窗口；Vite 1420；Victauri 发现文件写在 `/tmp/victauri/<pid>/` |
| `just doctor` | 13/13 passed（沙箱内，与 app 同一次调用） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。
> 终端的 console 输出：**零 error**；只有 xterm 自己的 2 条
> `warn task queue exceeded allotted deadline by N ms`（启动挂载时出现，本机 MESA
> 软件渲染栈下稳定复现，非 error）与降级时的 1 log + 1 warn。

## 待验证（本地跑不了 / 沙箱跑不了）

- **CI 三个 job 是否真能变绿** —— 仓库还没有 remote，从没跑过。本地只能校验 YAML 结构、
  job 图与资产可下载性；三条只在真 runner 上见分晓的风险记在
  [`docs/plans/0102`](./plans/0102-ci-platform-matrix.md) 的实施记录里。
  ⚠️ `checks-linux` 里那一条 `just ready` 会**编译整个 app**（`gen-types-check` 要
  `cargo run --bin gen-types`），CI 时长会明显变长 —— 首次实跑时留意。
- **宿主 MCP 连不到沙箱内运行的 app**（私有 PID / 临时目录）。沙箱内可用，
  但**必须让 app 与测试在同一次 bash 调用里**（坑 #33）。
- **`just dev-web` 的模拟后端没在真浏览器里点过**（本环境没有浏览器）：
  0203 的这条验收只做到 `pnpm build` + 代码审查 + `tsc` 类型检查。
- **大流量下的 JS heap 数字没取**：只验到"8 MB 灌完队列归零、界面仍可交互"，
  plan 0203 里写的 `get_performance` 取数没做。

## 当前基线（2026-09-11 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 增量重编译 | 6.09–6.26s（纯逻辑 crate 改动同样触发） |
| 出字节路径 | PTY read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write` |
| 前端渲染器 | **WebGL**（WebKitGTK + MESA 软件栈下仍拿到 WebGL2）；`canvas` 元素 2 块（纹理图集另算）；DOM 渲染器未启用（`.xterm-rows` 为 0） |
| 大输出实测 | 11.18 MB / **170 批**（0202）；8 MB / **135–141 批**（0203，含渲染消费） |
| 前端产物 | 839 kB（gzip 229 kB）—— xterm + React 占绝对多数，暂不做代码分割 |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| CSP | `csp`：`default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| 前提条件 | **需要能写 `$HOME`**；沙箱内会刷 `dconf-CRITICAL` 与 WebKit 缓存 hard-link 告警，但 app 仍正常起窗口 |

## 进行中 / 下一步

- [~] **plan 0102（CI 平台矩阵，GitHub Actions 一份）**：本地部分完成，
  最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [ ] **阶段 2 第四步**：[`docs/plans/0204`](./plans/0204-exit-zero-residue.md)
  （真正退出零残留：窗口关闭退出 / app 重载 / panic 三条路径）。
  ⚠️ 现在**没有**任何退出路径在关会话 —— 前端只在组件卸载时 `close_session`，
  而"收托盘"语义属于阶段 3，两条要一起想清楚（见 `AGENTS.md` §3.3）
- [ ] **E2E 入口要能真跑起来**（独立工作项，未开 plan）：`just test-e2e` 不在 `ready` 里、
  又要真 app，因此它的用例生成后就没人跑过；本轮查证时发现**三处先前就红**（坑 #36）

### 本轮完成（plan 0203：前端 xterm + WebGL 渲染）

- [x] **`src/terminal/surface.ts`**：xterm + `addon-webgl` / `addon-canvas` / `fit` /
  `search` / `serialize` / `unicode11`；渲染器 WebGL → canvas → **明确报错**三档
  （选定结果写进 `host.dataset.renderer`）；`term.write` 是**全工程唯一调用点**
- [x] **`src/terminal/attach.ts`**：命令式接线（会话 ↔ 渲染面）；`ResizeObserver`
  按帧合并 + 会话刚开时 `force` 补发真实行列数；**StrictMode 双挂载**下把没人要的
  会话显式关掉（否则每挂载一次多留一个真 PTY）；挂载失败折成报错，不让异常冒到 React
- [x] **`src/terminal/TerminalPane.tsx`**：React 只管宿主 DOM + 壳层状态；
  写入路径上**没有任何 state**（`useState` 命中只有"连接状态 / 渲染器种类"两处）
- [x] **`src/ipc/mock.ts`**：`just dev-web` 的本地模拟后端（`mockIPC` + 假 shell + `big`
  灌流），模拟的是**线上格式**（`__CHANNEL__:<id>` + `ArrayBuffer`）；只在
  `DEV && 没有 __TAURI_INTERNALS__` 时动态 import，生产包实测命中数 0
- [x] **`src-tauri/tests/terminal_render.rs`**：两条真 app 用例 ——
  「画布渲染 + 按键来回 + 8 MB 不卡死」与「WebGL 丢上下文 → canvas 且屏幕保留」
- [x] **CSP 最小化放行**（`csp` + `devCsp`），并用"两者取同一个字符串"的方式实跑验证
- [x] 顺手修 `integration.rs::command_greet` 的缺参（生成器写的桩，先前就红 —— 坑 #36）
- [x] 前端脚手架清理：`App.tsx` 的 greet 演示 → 终端；`App.css` 重写为终端布局；
  `index.html` 标题；删掉不再引用的 `src/assets/react.svg`

### 上一轮完成（plan 0202：IPC raw 字节通道）

- [x] **`Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`**（不是 `Channel<Vec<u8>>`
  —— 那是 JSON 数组，见坑 #31）。实测 11.38 MB / 181 批到达 JS
- [x] **会话命令**（`src-tauri/src/session.rs`，薄壳）：`open_session` / `write_session` /
  `resize_session` / `close_session`
- [x] **会话错误收敛**（`IpcError`）：core 与 pty 的错误在壳层折成可序列化形状
- [x] **IPC 类型边界接通**：`tauri-specta` 生成 `src/ipc/bindings.ts`；
  `gen-types-check` 纳入 `ready`（5 → 6 步）
- [x] **前端接收层** `src/ipc/session.ts`：建 raw 频道 → `ArrayBuffer` → 命令式回调
- [x] **ast-grep 规则 `no-string-pty-channel`**（正负例都验过）

### 更早

- [x] **plan 0201**：`OutputBatcher`（纯逻辑、注入时钟）+ `spawn_batcher`；criterion 基线
  3 条 + `just bench`；`AGENTS.md` §7 写下「性能基线不是门禁」
- [x] **CI 去 Gitea 化 + 吃透 GitHub 专属能力**：单 forge；`concurrency` +
  `cancel-in-progress`、`permissions: contents: read`、`defaults.run.shell`；
  工具安装统一走 `taiki-e/install-action`
- [x] **阶段 1 布局收口**：`crates/` → `src-tauri/crates/`（ADR-0004），
  根 `Cargo.toml` 删除；ast-grep 规则的 `files:` 改路径后**用探针重验**

### 已定案（cyrene 裁定）

| 项 | 结论 |
|---|---|
| **Rust 成员位置** | **全部收在 `src-tauri/` 下**，仓库根不放 Rust 成员或 manifest（2026-09-11，见 ADR-0004） |
| **CI** | **只维护 GitHub Actions 一份**；不做别的 forge 的兼容层（理由见 plan 0102） |
| **IPC 字节通道** | **raw**：`Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`；`Channel<Vec<u8>>` 是 JSON 数组，由规则拦下（2026-09-11，见坑 #31） |
| **类型边界** | **Rust 是唯一真相源**：`tauri-specta` 生成 `src/ipc/bindings.ts`。唯一手写处 = raw 频道的构造 |
| **渲染器** | **WebGL**；不可用退 **canvas**；两者都不可用**明确报错**，不静默落到 DOM（0203 落地） |
| **CSP** | **不为 `null`**：最小放行 + `devCsp` 只多 HMR 的 `ws://`（2026-09-11 实测通过） |
| **会话句柄** | 过 IPC 用壳层 `u32` + **checked** 转换，不过 `u64`（生成器拒绝 BigInt；截断会串会话） |
| P3 | **撤销** —— 豁免 webview 及其依赖栈的一切写入；判据改为"搬走文件夹后还能开" |
| SSH 实现 | **纯 Rust `russh`**，不调系统 `ssh` |
| `~/.ssh/config` | 只支持受限子集；遇 `Match`/`Include` **显式报错** |
| Bitwarden 接入 | `bw` CLI 作**用户自备前置**（不打包）+ v1 只读导入 |
| 命名 | 后端容器叫 `Session`；字节载体叫 `Transport`；**后端类型名不得编码 UI 呈现方式** |
| 连接模型 / 生命周期 | 不复用连接；连接生命周期 = 拥有它的 `Session`；关 `Session` 立刻断连 |
| 重连 | 3 次 + 指数退避，然后标记失败 |
| 传输落盘 | 临时名 + 原子重命名；不做断点续传 |
| `libudev` | 做成 cargo feature，仅 Linux 编译时启用 |
| `akasha-vt` | 维持延后；若必要则建于 `src-tauri/crates/akasha-vt/` |
| 性能基线 | criterion 数字**不进门禁**（`AGENTS.md` §7） |

### 待实测 / 待确认

- [ ] **CI 首次推送实跑**（现在还要看 `gen-types-check` 在 runner 上的耗时）
- [ ] **`just test-e2e` 里的红用例**（坑 #36）：`smoke::screenshot_captures_window`
      报 `unsupported window handle type on this platform`（全新 app 上也红，平台限制）；
      `smoke::ipc_integrity_passes` 的 `no console errors` 被 0202 探针在 `{end:true}`
      帧上抛的 `TypeError` 弄红（**同一次 app 里先跑过 `session_channel` 才出现** ——
      全新 app 上这条是绿的；修法：探针收到非 message 帧要跳过）。
      `integration::command_greet` 本轮已修
- [ ] **前端渲染的内存表现**：8 MB 已不卡死，但**没有 heap 数字**（`get_performance` 未取）
- [ ] **`just dev-web` 的模拟后端**没在真浏览器里点过（本环境没有浏览器）
- [ ] **`bw` 对 `sshKey` 条目的非交互行为** —— 需要真实 vault
- [ ] **可搬迁性收尾**（详见 [`portable.md`](./portable.md)）
- [ ] 托盘的 Linux 依赖 `libayatana-appindicator3` 已在 CI apt 列表里，需实测
- [ ] 单实例处理；动态转发（`-D`）的 SOCKS5 服务端；托盘图标尺寸（UI 阶段）

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。`Cargo.lock` / `deny.toml` / `target/` 都在那里。
  **仓库根没有 `Cargo.toml`** —— 在根目录直接跑 `cargo …`（含 `cargo bench`）会失败（坑 #8），
  一律用 `just` 转发。
- **三个 crate 的分工**：`akasha-core`（Session 模型，**零依赖**）、`akasha-pty`
  （`Transport` + portable-pty + **输出合批**）、`akasha`（app 包 = IPC 薄壳 + 代码生成 bin）。
- **前端三层**：
  - `src/ipc/` = **唯一允许碰后端**的目录：`bindings.ts`（生成物，禁止手改）、
    `session.ts`（唯一手写处：建 raw 频道）、`mock.ts`（**只给 `just dev-web` 用**）；
  - `src/terminal/` = 终端：`surface.ts`（xterm + 渲染器 + 唯一 `term.write`）、
    `attach.ts`（会话 ↔ 渲染面的命令式接线）、`TerminalPane.tsx`（React 壳层，只管状态）；
  - `src/App.tsx` = 壳层，目前只有一个终端。
- **调试白屏**：Victauri 的 `logs {action:"console"}` 能读 webview console；
  但它读不到"模块执行期就抛错"的那种失败 —— 那时要临时往 `index.html` 塞
  `window.onerror` 钩子再用 `eval_js` 读（坑 #35）。
- **文档三级粒度**：`ROADMAP.md`（判据）→ `docs/plans/TTxx-*`（手段）→ 本文件的坑（痕迹）。
  完成的 plan **整份移入 `docs/plans/archive/`**（不拼接、不追加）。
- 命令入口分两处：项目级在根 `justfile`，crate 级在 `src-tauri/justfile`。
  **权威清单在 `docs/just.md` §2**（21 个配方），由 `just docs-check` 强制同步。

## 踩过的坑（避免重复踩）

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency**（缺了 `cargo check` 退出码 101）。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**。
3. **just 的变量写 `$p`，不是 Make 的 `$$p`**（后者会展开成 PID）。
4. **just 用 justfile 所在目录作为配方工作目录** —— 这是 crate 级命令能去掉 `--manifest-path` 的原因。
5. **系统库缺失只在 cargo 构建脚本阶段暴露**；本机是 CachyOS（Arch 系），不是 apt。
6. **Tauri 没有 Rust 热重载，Victauri 也不提供**；迭代靠"下沉 + bacon"。
7. **just 的 shebang 配方需要可写的 runtime dir**，受限环境会失败 —— 用普通配方。
8. **仓库根没有 `Cargo.toml`** → 根目录下一切 cargo 命令失败。踩到的人多半照着一份旧记忆在操作。
9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** —— 跑一次 `just fmt`。
10. **CI 里 `libappindicator3-dev` 已不存在**，要用 `libayatana-appindicator3-dev`。
11. **受限环境下"写工作区之外被拒"看起来像工具/代码故障** —— 识别 → **直接提权重试**（`AGENTS.md` §1 末）。
12. **`git checkout <file>` 会静默丢弃未提交的改动** —— 负例自检用 `cp` 备份/还原。
13. **正向校验若不限定到目标段落就形同虚设** —— `docs-check` 改用 `awk` 取 §2 段落。
14. **`docs-check` 的反向检查**已扩到 `AGENTS.md` / `ROADMAP.md` / `docs/**/*.md`。
15. **后台遗留的 `just dev` 会让 Vite 继续监听 1420 而 app 早已不在** —— 用 `just doctor` 判别。
16. **含反引号的 grep 模式在 justfile 配方里必须整体放进单引号**。
17. **多文件行数检查要逐文件取**（awk 的 `NR` 会跨文件累加）。
18. **`[profile.*]` 写在 workspace 成员里会被静默忽略**（只有 root 那份生效）。
19. **整体 `mv` 构建缓存会留下写死的绝对路径**（`target/debug/build/*/output` 里的 `DEP_*`）。
20. **`cargo` 在成员目录里只选当前包** —— crate 级配方不写 `--workspace` 会**静默漏掉**成员。
21. **tauri CLI 默认只监听 `src-tauri`** —— 成员放外面 = 开发循环静默失效。
22. **cargo 的空 glob 是硬错误**（`members = ["crates/*"]`）。
23. **justfile 里不能出现完整的 `{{ … }}`**（要写字面量用 `{{{{`）。
24. **"一份工作流喂两个 forge"是一笔持续交的税**（四条约束与放弃理由在 plan 0102）。
25. **兼容层的遗产会以"看起来更稳"的样子留下来** —— 见到 `uname` 选资产那类写法先问"它是为哪个 forge 写的"。
26. **阻塞的 `Read` 与"按时间交付"天生冲突** —— 只按容量合批会把提示符扣在缓冲里直到用户按键；
    正解是读线程 + `recv_timeout(期限)`。
27. **零匹配的测试过滤器在 nextest 里是"报错"**（`error: no tests to run`），
    而过滤器会随模块/用例改名静默失效 —— 判据要写成"哪些用例必须绿"。
28. **`cargo bench` 会顺带用 bench 模式跑一遍单测目标**（打印 `0 passed; N ignored`），正常。
29. **仓库里出现第二个 bin 会让 `tauri dev` 起不来** —— `cargo run` 不知道跑哪个。
    修法是 `default-run = "<app bin>"`。踩到的场合：加了代码生成工具 `gen-types`。
30. **只写 `path` 的依赖等于版本号写 `*`** —— `cargo deny` 的 `wildcards = "deny"` 会判红。
    path 依赖要**同时写 `version`**。
31. **`Channel<Vec<u8>>` 不是二进制通道** —— 真正走 raw 的只有
    `Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`。由 `no-string-pty-channel` 拦下。
    ⚠️ **别只看字节数验收**：小消息（<1 KiB）的 raw 帧曾变成 `number[]`（上游 PR #13268），
    用例里要数"JSON 帧 == 0"。
32. **`u64` 不能直接过 IPC**：生成器拒绝导出 BigInt 风格类型。改用壳层 `u32` 句柄 + **checked** 转换
    —— 截断不是"数字变小"，是**把用户的按键送进另一个会话**。
33. **沙箱里 E2E 必须与 app 在**同一次** bash 调用内**：每次调用都是独立的 bwrap
    （私有 `/tmp`、独立 PID 命名空间）。**推论**：上一个调用里放在 `/tmp` 的备份文件下一个调用就没了。
34. **`pkill -f <模式>` 会匹配到自己** —— 用 `pkill -f '[v]ite'` 或按 PID/进程组杀。
35. **`@xterm/addon-unicode11` 需要 `allowProposedApi: true`**（`term.unicode` 是 proposed API）。
    不开的话 `loadAddon` 抛在 React **effect** 里 → React **卸载整棵树** → 症状是**整屏白屏**、
    `#root` 空，且**看不到任何报错**。定位手段：临时往 `index.html` 塞
    `window.onerror` + `unhandledrejection` 钩子，再用 Victauri `eval_js` 读它；
    常规路径是 Victauri 的 `logs {action:"console"}`（但它只有"模块已执行"之后才收得到）。
36. **不在门禁里的测试等于没测**：`just test-e2e` 既不在 `ready` 里、又要真 app，
    于是它那几个用例自生成后就没跑过 —— 本轮查证发现**三处先前就红**（见「待实测」）。
    修 `command_greet` 只是把最明显的一处补上；**E2E 缺的是一条真跑得起来的入口**。
37. **`git mv` 之后 `docs-check` 会同时验两件事**（文件在不在、索引指得对不对）——
    归档时**两处一起改**，只改一处必红。
38. **每加一个依赖就多一份要维护的放行**：CSP 的 `style-src 'unsafe-inline'` 就是
    xterm 自己注入 `<style>` 逼出来的 —— 加前端库时先想"它要不要新的 CSP 指令"。

## 环境

CachyOS（Arch 系）/ rustc 1.98.1 / cargo 1.98.1 / node 26.8.2 / pnpm 12.3.4 / mise 2026.9.1
