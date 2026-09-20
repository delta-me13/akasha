# just 快速上手（面向使用者）

> **30 秒版**：记不清有哪些命令时执行 `just --list`；改完代码提交前执行 `just ready`。
>
> 本文件是「我想做什么 → 执行什么」的任务视角，**并且就是命令的权威清单**（见下面 §2）。
> `AGENTS.md` §11 只讲归属原则，不重复这张表。

## 1. 最常用的五条

| 我想… | 命令 | 说明 |
|---|---|---|
| 开始开发 | `just dev` | 启动 app。前端改动走 Vite HMR；**Rust 改动自动重编译并重启**。启动一次即保持运行 |
| 只改 Rust 逻辑 | `just watch` | bacon 秒级 clippy，**完全不启动 app** —— 这是本项目最快的反馈循环 |
| 只改界面 | `just dev-web` | 浏览器里运行 Vite，不启动 app |
| 提交前 | `just ready` | 一条命令执行全部门禁 |
| 忘了有哪些命令 | `just --list` | 列出全部配方及其一句话说明 |

## 2. 全部命令（权威清单）

> **这是全部配方的权威清单。** 由 `just docs-check` 强制保证：**每个配方都必须出现在下面这张表里**
> （只认这张表，不认别处的顺带提及），且 `AGENTS.md` / `README.md` / `ROADMAP.md` 与
> `docs/**/*.md` 里提到的每个命令都必须真实存在。（`CLAUDE.md` 不在反向检查范围内：
> 它含 Victauri 自动生成块，块内英文散文会被裸词正则读成配方名。）
> `AGENTS.md` §11 只讲归属原则，不再重复这张表。

「归属」列的含义：**根** = 直接实现在根 `justfile`；**转发** = 实现在 `src-tauri/justfile`、
根只转发；**根组合** = 跨根与 crate 的组合，只能在根定义。

| 命令 | 作用 | 归属 |
|---|---|---|
| `just dev` | 启动 app：前端 HMR + Rust 改动自动重编译并重启（workspace 就在 `src-tauri/` 内，默认监听已覆盖全部成员）。**启动一次即保持运行** | 根 |
| `just dev-web` | 只运行前端，不启动 app（浏览器里迭代界面） | 根 |
| `just watch` | bacon 秒级反馈循环，不启动 app —— 改纯逻辑时最快 | 转发 |
| `just ready` | **提交前运行这一个**：全部门禁（安静聚合，失败才倾倒） | 根组合 |
| `just check` | 类型检查（含 tests / benches），**workspace 全成员** | 转发 |
| `just clippy` | clippy，警告即错误，**workspace 全成员** | 转发 |
| `just lint` | clippy + `ast-grep scan` + `ast-grep test`（结构护栏：**真代码**有没有违规 / **规则自己**还对不对） | 根组合 |
| `just fmt` | rustfmt 格式化（`--all` = workspace 全成员） | 转发 |
| `just fmt-check` | 只检查格式，不改文件 | 转发 |
| `just test` | 单元测试（cargo-nextest），**全部测试目标** | 转发 |
| `just test-e2e` | E2E：真实 app 上的验收（契约 + 交互）。**自包含** —— 已有 app（`just dev`）就复用，没有就自行启动 Vite + app，运行结束后自行结束；跳过的用例会把原因输出。**自起时运行三段**：前两段验关窗语义（它由数据目录里的配置决定、只在启动时读）——**两段都先在 bin 同目录备好便携数据目录**（没有它 app 会退回 OS 数据目录，两段就运行在不同的目录里，第二段写的配置也就读不到），第一段没有配置文件 = 收托盘（`window_close` 在这里有判据），第二段写 `close_behavior=exit` 再启动一次 app（`exit_residue` 的退出刺激），配置文件在运行结束后还原；**第三段就是 `just portable`**（可搬迁性 —— 它自己复制 bin、自行启动 app，所以只能在此时没有别的 akasha 时运行）。目标按 `E2E_TARGETS` / `E2E_TARGETS_EXIT` 的顺序逐个串行运行（`cargo test` 一次收多个 `--test` 时是按名字排序的）；guard 要求 `tests/*.rs` 出现在 **`E2E_TARGETS` / `E2E_TARGETS_EXIT` / `E2E_NO_APP` / `E2E_SELF_APP`** 四处之一（`E2E_NO_APP` = 不需要真实 app 的集成测试：只把二进制当子进程用的 `session_watchdog`，与 plan 0109 迁进 `tests/` 的那批域模块集成测试 —— 它们由 `just test` 的 nextest 全量运行执行；`E2E_SELF_APP` = 自行启动 app 的 `portable`）。⚠️ **发现目录按候选逐个找**（`/tmp` 与本 shell 的 `$TMPDIR` 各算一个）：它由**每个进程**的 `std::env::temp_dir()` 给出，而它读 `TMPDIR` —— 执行器给动作的最小环境不含该变量，两侧取值因此可能不同；只看一个候选时，症状是「app 已启动但没有登记到 discovery 目录」（`STATUS.md` 问题 #171） | 转发 |
| `just bench` | 吞吐基线（criterion）。**不是门禁**，用于改动前后对比 | 转发 |
| `just portable` | 可搬迁性：把 **bin 所在文件夹整个移动**之后数据还在吗（`docs/portable.md` §5 的五步，外加 §4 第 3 条"便携目录不可写就拒绝启动"）。**自行启动 app** —— 把二进制复制进临时布局，在 A 启动一次、移动为 B、再启动一次，两次都用 app 自己的命令读回四类池，并用库函数逐项比对内容。⚠️ **不能与别的 akasha 同时运行**：单实例（plan 0304）会让它启动的第二份自己退掉，所以先查一遍并说清该关闭什么；Vite 复用或自起（与 `just test-e2e` 同一套做法），发现目录同样按候选逐个找。`just test-e2e` 的自起分支在**第三段**调用它，复用别人的 app 时那一段显式跳过并输出原因 | 转发 |
| `just libudev-check` | serial 的 libudev 只在 Linux 上（plan 0801 的判据）：按**目标**核对整包的依赖图里有没有 libudev —— Linux 上必须有，Windows / macOS 上必须没有。⚠️ `cargo tree --target` 会取回那个目标独有的依赖，冷缓存下需要联网；每条分支都先看 `cargo tree` 自己的退出码，否则「图没解析出来」会被读成「没有 libudev」 | 转发 |
| `just serial-check` | serial 的**两条枚举实现都要真的执行一次**（plan 0802）：串口的两个测试目标（`enumeration` / `pty_roundtrip`）在默认配置（libudev）与 `--no-default-features`（sysfs）下各执行一遍。Linux 上哪一套实现被编译进去完全由那个 feature 决定（`serialport` 的 `enumerate.rs` 两个分支），所以「只执行默认配置」等于另一套一次都没有被执行过 —— 而它正是发行版缺 libudev 时的降级路径。本机实测：默认配置列出 32 条 `/dev/ttyS*`（`/dev` 下一个都没有），关闭该 feature 后列出 0 条 | 转发 |
| `just serial-unit` | serial 域的**平台面单元测试**（plan 0803）：`cargo test --lib -p akasha serial::` —— 只构建 `akasha` 这一个包（成员模块是它的模块，见 ADR-0008），**不需要 app、不需要串口设备**，几秒钟就有读数。用途是让**每个平台的原生构建**都能执行一次串口代码：那三条串口 E2E 在 Windows 上全部跳过（ConPTY 没有设备节点），而 CI 的 E2E 格子在每个平台都执行这条配方。⚠️ **并不绕开 vendored OpenSSL**（`store` 是 `akasha` 的模块，`rusqlite` 在这个包的依赖里），冷缓存下仍需原生 perl；⚠️ 用 `cargo test` 而非 nextest：那个 job 只装 `just`，为一个聚焦子集给三平台各加一个工具不划算 | 转发 |
| `just deny` | 依赖门禁：许可证 / 漏洞 / 来源（需联网）。⚠️ 带 `--workspace`，理由见下 | 转发 |
| `just deny-offline` | 同上，跳过需要联网的 advisories | 转发 |
| `just gen-types` | Rust command/event → `src/ipc/bindings.ts`（生成物，**禁止手改**） | 转发 |
| `just gen-types-check` | 生成物是否与 Rust 侧一致（改了 IPC 忘了生成就失败） | 转发 |
| `just doctor` | 确认 Victauri 连的是本项目，而不是别的实例 | 根 |
| `just syscheck` | 检查系统库是否齐（缺 webkit2gtk 会提前报错） | 根 |
| `just tools` | 按 `mise.toml` 装齐全局 CLI 工具 | 根 |
| `just tools-ls` | 看工具版本与来源 | 根 |
| `just docs-style` | 文档语体：剥离代码块与行内代码后匹配禁用语表（第二人称、语气词、口语虚词、比喻与纯口语动词）。表在 [`style.md`](./style.md)，由 `just docs-check` 第一步调用（规则见 `AGENTS.md` §8.2）。**非快速失败**：逐份文档各查一遍，全部查完才汇总报错 —— 单份文档命中超过 20 条时列出前 20 条并写明剩余条数 | 根 |
| `just docs-check` | 文档纪律：① 文档语体（调用 `just docs-style`）② 命令未漂移 ③ ROADMAP 没长细节（每条 ≤3 行、无代码块、无命令调用）④ plan 预算（≤200 行）+ 索引一致 + 骨架不许开工。四部分**每轮全部执行**：第一类失败不终止其余三类，一轮给出全部待修项 | 根 |

| `just runner-policy` | 生成 Agent 执行器的 policy 样板（含当前脚本哈希），并刷新 `docs/agent-runner.policy.json` | 根 |
| `just runner-status` | 执行器的授权清单、在飞请求与 dev 状态（校验 policy 与脚本哈希，不需要提权） | 根 |
| `just runner-start` | 常驻启动执行器：沙箱受限时的**唯一提权点**（沙箱内失败于 openpty 即为提权依据，见 [`agent-runner.md`](./agent-runner.md)） | 根 |
| `just runner-stop` | 停止执行器并回收它名下的全部进程组（缩权，不需要提权） | 根 |
| `just runner-stop-dev` | 只回收 dev（app），保留执行器 | 根 |
| `just runner-run` | 提交动作并**阻塞**到结束；退出码 = 动作退出码（默认最多等 540s，低于 DSH 前台调用上限 600s）—— AI 的默认入口 | 根 |
| `just runner-submit` | 只提交动作，立刻打印 `request=<id>`：异步路径的第一步 | 根 |
| `just runner-wait` | **阻塞**等待某个 `request=<id>` 结束；已结束则立即返回，可重复等待。等待由 FIFO + `select` 唤醒，不是轮询 | 根 |
| `just runner-result` | 非阻塞读某个 `request=<id>` 的结果（未结束退出码 4；已结束的退出码与 `runner-wait` 一致） | 根 |

> 裸 `just`（不带配方名）命中的是 `default`：它只输出 `just --list`，因此**不上表**，
> `docs-check` 的正向检查也据此跳过它。除它以外，**每个配方都必须有上面这一行**。

## 3. 两个 justfile 是什么关系

```
justfile                 ← 在此执行命令（项目级 + 转发）
├── dev / dev-web / tools / syscheck / doctor
├── lint / ready / docs-check        ← 跨根与 crate 的组合
└── check / fmt / test / deny / ... → 转发 ──┐
                                            ↓
src-tauri/justfile       ← crate 级命令真正实现的地方
    check / clippy / fmt / fmt-check / watch
    test / test-e2e / portable / bench / libudev-check / serial-check / serial-unit
    deny / deny-offline / gen-types / gen-types-check
```

**为什么要分两个**：just 用 **justfile 所在目录**作为配方的工作目录。
把 crate 级命令放在 `src-tauri/` 里，`cargo` 就天然找得到 manifest ——
不需要在每条命令后面跟 `--manifest-path`（少一类需要遵守的纪律）。

副作用（有益）：`cd src-tauri && just check` 也能独立使用。

**命令体只写一处**：根 justfile 里的 crate 级配方**只做转发**，不复制实现。

## 4. 典型工作流

**改 Rust 逻辑**（大多数时候）
```bash
just watch        # 一个终端常驻，改一次看一次结果
just test         # 写完了跑测试
just ready        # 全绿再提交
```

**改界面**
```bash
just dev-web      # 浏览器里迭代，Vite HMR 最快
```

**运行 E2E**（真实 app 上的契约与交互验收）
```bash
just test-e2e     # 自包含：已有 app（just dev）就复用，没有则自行启动 Vite + app，执行结束后回收
```
跳过的用例会把原因输出（例如 Wayland 下截不了窗口）——**跳过不是"过了"**，
所以输出里一定要看得见它。

**改 IPC（command / event）**
1. 先 `just dev`（app 得运行着，Victauri 才能连）
2. 改完运行 `just gen-types`，**提交生成的类型差异**
3. 用 Victauri 走一遍真实路径（见 `AGENTS.md` §7）

**加依赖**
```bash
cd src-tauri && cargo add <crate>      # Rust 依赖（没有根 Cargo.toml，所以要进目录）
pnpm add <pkg>                          # 前端依赖（在仓库根）
just deny-offline                       # 新依赖的许可证要过门禁
```

## 5. `just ready` 的输出约定

```
→ fmt-check        ✅ 1s
→ lint             ✅ 3s
→ test             ✅ 2s
→ deny-offline     ✅ 6s
→ gen-types-check  ✅ 2s
→ docs-check       ✅ 0s
✅ just ready 全绿（6/6）
```

**成功时只有这些**。失败时会把**那一步**的完整输出倒出来（超过 80 行则首尾各 40 行，
完整内容落在 `.just-ready-fail.log`）。

为什么这么设计：`deny-offline` 对 Tauri 这种依赖树会打 **5000+ 行**「重复版本」警告，
而它是 warn 级、永远不会让门禁失败 —— 在成功的运行里那全是噪音，会掩盖真正的错误。

想看细节就**单独运行那一步**（`just deny-offline` / `just lint` / `just test`），输出是完整的。

### 为什么 `deny` / `deny-offline` 仍带 `--workspace`

成员已按 ADR-0008 并入 `src/`，所以 `--workspace` 与"这一个包"等价。保留它的理由有两条：
它曾经修掉一个**静默失效**（cargo-deny 默认只把 manifest 指向的那个包当作依赖图的根，
未进图的子树连 `[bans] deny` 都不生效 —— plan 0402 实测）；将来若再拆出成员，写法不必改。

⚠️ 位置也有讲究：`--workspace` 是**顶层参数**，必须在 `check` **之前**
（`cargo deny --workspace … check bans`），放在后面会被当成未知参数直接报错。

## 6. 出问题了

| 症状 | 原因 / 处理 |
|---|---|
| `just: command not found` | 工具没装。`just tools`（走 `mise.toml`），或 `cargo install just` |
| 提示找不到 `Cargo.toml` | 在根目录执行了 crate 级命令。改用根转发（`just check`），或 `cd src-tauri` |
| 缺 webkit2gtk 之类的系统库 | `just syscheck` 会指出来。Arch 系：`sudo pacman -S webkit2gtk-4.1` |
| app 无法启动，或 Victauri 连不上 | `just test-e2e` 会自行启动 app（手动启动用 `just dev`）；失败时它会输出 app 日志尾部（前两段的完整日志在 `$TMPDIR/akasha-e2e-app1.log` / `-app2.log`；第三段那个 app 的日志归 `just portable` 自己，在它的临时布局里、运行结束后随布局一起删除）。再用 `just doctor` 确认连的是本项目 |
| `just test-e2e` 说 Vite 无法启动 | 它探的是 `localhost` —— vite 默认只监听 `[::1]`，拿 `127.0.0.1` 探会得到"无法启动"的假象（`STATUS.md` 问题 #40）。日志：`$TMPDIR/akasha-e2e-vite.log` |
| E2E 里"敲命令"之后屏幕没反应 | 先确认是否把 shell **阻塞**了：敲进终端的那一行必须 fish / bash / sh 都成立（`(cmd) &` 在 fish 里是**命令替换**，会一直等下去 —— 问题 #41）。症状是**所有**敲命令的用例一起超时，很容易误判成前端坏了 |
| `just ready` 有一步失败 | 看结尾提示的那一步，或 `.just-ready-fail.log` |
| 改了配方但 `just --list` 没显示 | 检查缩进（配方体必须是 tab 或统一缩进），以及是否写在了对的 justfile 里 |
| 改了 `src-tauri/src/` 下的文件，app 却不重编译 | 先确认它**确实在 `src-tauri/` 里面**（tauri CLI 默认只监听 `src-tauri`）。源码若被放到它外面，必须另配监听范围，否则开发循环**静默失效** —— 见 `STATUS.md` 问题 #21 |
| CI 上失败但本地全部通过 | 先看是哪条 job：Linux 执行的就是本地这条完整门禁，Windows / macOS 只做类型检查 —— 那两条失败多半是 cfg 分支或平台 API。构造与边界见 `AGENTS.md` §12 与 `docs/plans/0102` |

**受限环境里运行 app**（容器 / agent 沙箱 / 无写权限的家目录）：
Tauri 启动时要写 `$HOME` 下的数据目录，被拒时会 panic 在
`Failed to setup app: 只读文件系统 (os error 30)`。**这是环境权限问题，不是项目 bug。**

**顺序很重要 —— 先提权，重定向只是退路**：让这一次运行能写工作区之外
（agent 场景下就是对该命令申请提权；规则见 `AGENTS.md` §1 末）。
只有提权不可用（被拒绝 / 无人审批）时，才用下面的 XDG 重定向：

```bash
mkdir -p .devhome/{data,config,cache}
XDG_DATA_HOME=$PWD/.devhome/data \
XDG_CONFIG_HOME=$PWD/.devhome/config \
XDG_CACHE_HOME=$PWD/.devhome/cache \
just dev
```

（`.devhome/` 已在 `.gitignore` 里。dconf 的 `dconf-CRITICAL` 警告无害，可忽略。）

> ⚠️ **重定向有副作用**（实测）：mise 也读 `XDG_DATA_HOME` / `XDG_CACHE_HOME`，
> 于是它会把 node / pnpm / just **重新下载进 `.devhome`**（实测约 94 MB，启动明显变慢），
> 而不是复用 `~/.local/share/mise`。**这就是它只配当退路的原因** ——
> 能提权就直接提权，不应为绕开权限而付出这份代价。

> 同一类问题还有 `just deny`：它要写 `~/.cargo/advisory-dbs`，受限环境下会报
> `failed to acquire advisory database lock ... failed to create parent directories`。
> 处理方式相同：**先提权**。

## 7. 想加一条新命令

1. **先判断归属**：涉及 cargo / Rust → 写进 `src-tauri/justfile`；
   前端、环境、跨仓库的组合 → 写进根 `justfile`。
2. 根 justfile 里给 crate 级命令写**转发**，不得复制命令体。
3. **在本文 §2 的表格里加/改一行** —— 那是唯一权威清单，`AGENTS.md` §11 不放表。
4. 运行 `just docs-check` 验证（它已纳入 `just ready` 和 CI，不同步会直接失败）。

## 8. 和 CI 的关系

CI（`.github/workflows/ci.yml`，GitHub Actions 一份）六个 job —— **每个平台一条流水线**，
检查与 E2E 两段在平台内串行，平台之间不互相等待：

| job | 执行什么 |
|---|---|
| `checks-linux` | **就是本地那一条 `just ready`** —— 门禁只有一处定义 |
| `checks-macos` / `checks-windows` | 各自平台上只运行 `just check`（挡 cfg 分支错误） |
| `e2e-linux` | `needs: checks-linux`；本地那条 `just test-e2e`（xvfb 提供 X11 显示） |
| `e2e-macos` | `needs: checks-macos`；同上 |
| `e2e-windows` | `needs: checks-windows`；同上 |

同一分支上来了新推送，**上一次未完成的运行会被取消**（`main` 除外）——
所以连着推几次只有最后一次运行完成，这是刻意的成本开关。

⚠️ **`main` 上的新运行是排队等待，不是立刻开始**：前一次还没结束时，它的状态一直显示
`pending` —— **看起来像卡住，不是异常**。一个分组里最多留一个排队中的运行；取消正在执行的那次
会把紧随其后的排队运行一起结束。要重新验证某一次提交（尤其是上一次运行被取消之后），
用 Actions 页面上的手动触发，不必为触发它而加一个空提交。

各 job 的典型耗时（2026-09-15 一次冷缓存运行实测）：Linux 门禁 **9 分**、macOS 类型检查
**3.5 分**、Windows 类型检查 **8 分**、E2E（Linux）**9 分**、E2E（macOS）**13 分**；
**Windows 的 E2E 最慢** —— 先做一次约 14 分钟的全量构建（vendored OpenSSL 与 SQLCipher 都在内），
再串行执行 28 个 E2E 目标。两者都有硬超时（45 / 60 分钟），不会无限执行。

**所以本地通过 ≈ CI 通过** —— 不存在"本地通过而 CI 失败"的两套标准。
规则见 `AGENTS.md` §12；为什么不做别的 forge 的兼容层，见 `docs/plans/0102`。

## 9. 关于 `mise.toml`

全局 CLI 工具（just / bacon / cargo-nextest / cargo-deny）记在 `mise.toml`。
装了 mise 并激活 shell 后，**进入本目录会自动装齐缺失的工具**（`just tools` 是同一件事的手动版）。
不想让 mise 管理、继续使用 `~/.cargo/bin` 里那套，删除 `mise.toml` 即可。
