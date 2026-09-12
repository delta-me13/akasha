# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-11

## 一句话

**阶段 2「端到端最小终端」走到一半**：`Transport` 输出合批（plan 0201）与
**IPC raw 字节通道**（plan 0202）都已落地并实测；下一步是
[`docs/plans/0203`](./plans/0203-xterm-webgl-render.md)（前端 xterm + WebGL 渲染）。

顺带把 **IPC 类型边界接通了**：`tauri-specta` 生成 `src/ipc/bindings.ts`，
`just gen-types` 不再是桩，`gen-types-check` 进了 `just ready`（5 步 → **6 步**）。

`ROADMAP.md` 共 50 个条目（10 个阶段）：阶段 1 完成 5/6（剩 CI 实跑），
阶段 2 完成 2/4。**CI 仍未真正跑过** —— 仓库没有配置任何 git remote。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + **gen-types-check** + docs-check） | 退出码 **0**，6/6 全绿 |
| `just test` | **45 tests run: 45 passed**（`akasha` 11 + `akasha-core` 8 + `akasha-pty` 26） |
| **会话 E2E**（`VICTAURI_E2E=1 cargo test --test session_channel`） | **1 passed**：真 app + 真 PTY，`yes \| head -c 10000000` 的 **11 383 949 字节分 181 批**送达 JS，全程无错误 |
| `just gen-types` / `just gen-types-check` | 生成 `src/ipc/bindings.ts`；比对通过（生成物已提交） |
| `just bench`（criterion，配方 #20） | 52.7 GiB/s（容量路径）/ 14.1 ns 每批 / 9.64 GiB/s（含线程与 channel） |
| `just check` / `just clippy`（`--workspace --all-targets`） | 退出码 **0** |
| `just deny-offline` | `bans ok, licenses ok, sources ok` |
| `just docs-check` | 三部分全过（ROADMAP 50 条目在 3 行内 / plan 43 份 ≤200 行且索引一致） |
| `ast-grep scan` | 退出码 **0**；**四条**规则均已用正负例验证（新规则 `no-string-pty-channel` 命中 `Channel<Vec<u8>>`，不命中 `Channel<InvokeResponseBody>`） |
| `cargo tree -p akasha-core` / `-p akasha-pty` \| `grep -c tauri` | **0** / **0**（分层成立） |
| `just dev` | 起窗口；CLI 打印 **3 行** `Watching`（`src-tauri` + `crates/akasha-core` + `crates/akasha-pty`）—— 成员在监听范围内 |
| `just doctor` | 13/13 passed（沙箱内，与 app 同一次调用） |
| `just syscheck` | webkit2gtk-4.1 2.52.6 / javascriptcoregtk-4.1 2.52.6 / gtk+-3.0 3.24.52 / librsvg-2.0 2.62.3 |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。
> `just bench` 整组约 30 秒；`gen-types-check` 会重新编译并运行生成器（约 6 秒）。

## 待验证（本地跑不了 / 沙箱跑不了）

- **CI 三个 job 是否真能变绿** —— 仓库还没有 remote，从没跑过。本地只能校验 YAML 结构、
  job 图与资产可下载性；三条只在真 runner 上见分晓的风险记在
  [`docs/plans/0102`](./plans/0102-ci-platform-matrix.md) 的实施记录里。
  ⚠️ 现在 `checks-linux` 里那一条 `just ready` 会**编译整个 app**（`gen-types-check`
  要 `cargo run --bin gen-types`），CI 时长会明显变长 —— 首次实跑时留意。
- **宿主 MCP 连不到沙箱内运行的 app**（私有 PID / 临时目录）。沙箱内可用，
  但**必须让 app 与测试在同一次 bash 调用里**（坑 #33）。

## 当前基线（2026-09-11 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 增量重编译 | 6.09–6.26s（纯逻辑 crate 改动同样触发） |
| 出字节路径 | PTY read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` |
| 大输出实测 | 11.38 MB / **181 批**（≈63 KiB 每批）/ 1.65 s（其中绝大部分是 `yes` 在写 PTY，不是通道） |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| 合批吞吐（release） | 容量路径 52.7 GiB/s；每批开销 14.1 ns/批；端到端 9.64 GiB/s（复跑浮动 <3%） |
| 前提条件 | **需要能写 `$HOME`**；沙箱内会刷 `dconf-CRITICAL` 与 WebKit 缓存 hard-link 告警，但 app 仍正常起窗口 |

## 进行中 / 下一步

- [~] **plan 0102（CI 平台矩阵，GitHub Actions 一份）**：本地部分完成，
  最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [ ] **阶段 2 第三步**：[`docs/plans/0203`](./plans/0203-xterm-webgl-render.md)
  （前端 xterm + WebGL 渲染）。**字节流已经能到前端**（`src/ipc/session.ts` 的
  `openTerminalSession`），0203 要做的是把它写进 xterm 缓冲并接 `onData` 回写
- [ ] 0203 顺手补：**IPC 侧消费慢会不会把 PTY 反压死**（读线程在有界队列上阻塞是设计，
  要确认现象是"输出暂停"而不是"卡死"）

### 本轮完成（plan 0202：IPC raw 字节通道）

- [x] **`Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`**（不是 `Channel<Vec<u8>>`
  —— 那是 JSON 数组，见坑 #31）。实测 11.38 MB / 181 批到达 JS
- [x] **会话命令**（`src-tauri/src/session.rs`，薄壳）：`open_session` / `write_session` /
  `resize_session` / `close_session`；`Sessions` 只管"谁存在 + 字节怎么走"，
  会话归属与回收在 `akasha-core` 的注册表
- [x] **会话错误收敛**（`IpcError`）：core 与 pty 的错误在壳层折成可序列化形状 ——
  它们**不知道 IPC 存在**
- [x] **IPC 类型边界接通**：`tauri-specta` 生成 `src/ipc/bindings.ts`；
  `just gen-types` 变成真的、新增 `gen-types-check` 并纳入 `ready`（5 → 6 步）
- [x] **前端接收层** `src/ipc/session.ts`：建 raw 频道 → `ArrayBuffer` → 字节交给
  命令式回调（**不进 React state**）；`App.tsx` 的 `greet` 改走生成层，去掉裸 `invoke`
- [x] **ast-grep 规则 `no-string-pty-channel`**（正负例都验过）
- [x] **端到端用例** `src-tauri/tests/session_channel.rs`：真 app + 真 PTY + 10 MB，
  用 `wait_for_expression` 等真正完成（**没有 sleep**）

### 上一轮完成（plan 0201：`Transport` 输出合批）

- [x] `OutputBatcher`（纯逻辑、注入时钟）+ `spawn_batcher`（读线程 → 有界队列 →
  合批线程 `recv_timeout(期限)`），两条触发 ≥64 KiB / ≥16 ms 的边界都有精确断言
- [x] criterion 基线 3 条 + 新增 `just bench` 配方；`AGENTS.md` §7 写下
  「性能基线不是门禁」

### 更早

- [x] **CI 去 Gitea 化 + 吃透 GitHub 专属能力**：单 forge；`concurrency` +
  `cancel-in-progress`、`permissions: contents: read`、`defaults.run.shell`；
  工具安装统一走 `taiki-e/install-action`（四条 Gitea 约束留在 plan 0102 的「放弃记录」）
- [x] **阶段 1 布局收口**：`crates/` → `src-tauri/crates/`（ADR-0004），
  根 `Cargo.toml` 删除；ast-grep 规则的 `files:` 改路径后**用探针重验**

### 已定案（cyrene 裁定）

| 项 | 结论 |
|---|---|
| **Rust 成员位置** | **全部收在 `src-tauri/` 下**，仓库根不放 Rust 成员或 manifest（2026-09-11，见 ADR-0004） |
| **CI** | **只维护 GitHub Actions 一份**；不做别的 forge 的兼容层（理由见 plan 0102） |
| **IPC 字节通道** | **raw**：`Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`；`Channel<Vec<u8>>` 是 JSON 数组，由规则拦下（2026-09-11，见坑 #31） |
| **类型边界** | **Rust 是唯一真相源**：`tauri-specta` 生成 `src/ipc/bindings.ts`，`ready` 里比对生成物。唯一手写处 = raw 频道的构造（生成器写不出 `InvokeResponseBody` 的 TS 类型） |
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

- [ ] **CI 首次推送实跑**（现在还要看新增的 `gen-types-check` 在 runner 上的耗时）
- [ ] **`tauri.conf.json` 的 `csp` 仍是 `null`**，与 `AGENTS.md` §4.3 冲突 ——
      接前端渲染时（0203）按需最小化放行
- [ ] **前端渲染的内存表现**：11 MB 已能到达 JS，但"渲染 11 MB 不爆内存"是 0203 的判据
- [ ] **`bw` 对 `sshKey` 条目的非交互行为** —— 需要真实 vault
- [ ] **可搬迁性收尾**（详见 [`portable.md`](./portable.md)）
- [ ] 托盘的 Linux 依赖 `libayatana-appindicator3` 已在 CI apt 列表里，需实测
- [ ] 单实例处理；动态转发（`-D`）的 SOCKS5 服务端；托盘图标尺寸（UI 阶段）

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。`Cargo.lock` / `deny.toml` / `target/` 都在那里。
  **仓库根没有 `Cargo.toml`** —— 在根目录直接跑 `cargo …`（含 `cargo bench`）会失败（坑 #8），
  一律用 `just` 转发。
- **三个 crate 的分工**：`akasha-core`（Session 模型，**零依赖**，由 cargo tree 与 ast-grep 守着）、
  `akasha-pty`（`Transport` + portable-pty + **输出合批**）、`akasha`（app 包 = IPC 薄壳 +
  代码生成 bin）。依赖方向单向，**现在真的接上了**（`akasha → crates/*`）。
- **IPC 三层**：`src-tauri/src/bindings.rs`（命令清单，同时喂运行期分发与生成器）→
  `src/ipc/bindings.ts`（**生成物，禁止手改**）→ `src/ipc/session.ts`（唯一手写处：
  建 raw 频道）。改命令后必须 `just gen-types` 并提交产物。
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
29. **仓库里出现第二个 bin 会让 `tauri dev` 起不来** —— `cargo run` 不知道跑哪个，
    报 `could not determine which binary to run`。**症状极具迷惑性**：Vite 起得来、
    tauri 开始"Watching"、看起来什么都正常，但 app 根本没启动。
    修法是 `default-run = "<app bin>"`。踩到的场合：加了代码生成工具 `gen-types`。
30. **只写 `path` 的依赖等于版本号写 `*`** —— `cargo deny` 的 `wildcards = "deny"` 会判红
    （`found 2 wildcard dependencies for crate 'akasha'`）。path 依赖要**同时写 `version`**。
31. **`Channel<Vec<u8>>` 不是二进制通道** —— tauri 有 `impl<T: Serialize> IpcResponse for T`，
    所以它发出去的是"六万多个数字的 JSON 数组"，与默认 JSON IPC 是同一条慢路；
    `Channel<String>` 还要多一次 base64。**规范原文（§3.2 旧版）把这种写法当成"正确做法"，
    连 plan 一起抄错了** —— 真正走 raw 的只有 `Channel<InvokeResponseBody>` +
    `InvokeResponseBody::Raw`（JS 侧收到 `ArrayBuffer`）。现在由 `no-string-pty-channel` 拦下。
32. **`u64` 不能直接过 IPC**：生成器拒绝导出 BigInt 风格类型（精度），而
    `dangerously_cast_bigints_to_number` 是**全局**开关，会让将来每个 `u64` 字段都失去保护。
    改用壳层 `u32` 句柄 + **checked** 转换 —— 截断不是"数字变小"，是**把用户的按键送进另一个会话**。
33. **沙箱里 E2E 必须与 app 在**同一次** bash 调用内**：每次调用都是独立的 bwrap
    （私有 `/tmp`、独立 PID 命名空间），所以一个调用里起的 `just dev`，在下一个调用里
    既看不到进程也找不到 Victauri 的发现文件（`just doctor` 会说"没有发现文件"）。
    本机（非沙箱）分开跑没这个问题。在同一个调用里 `setsid just dev &` 然后跑用例即可。
34. **`pkill -f <模式>` 会匹配到自己**：`pkill -f 'vite'` 的进程命令行里就含 "vite"，
    于是它先杀掉发起它的那个 shell（表现为莫名其妙的退出码 143）。用 `pkill -f '[v]ite'`
    这种写法，或按 PID/进程组杀。

## 环境

CachyOS（Arch 系）/ rustc 1.98.1 / cargo 1.98.1 / node 26.8.2 / pnpm 12.3.4 / mise 2026.9.1
