# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-11

## 一句话

**阶段 1 基本落地**：根 workspace、两个不依赖 Tauri 的 crate（`akasha-core` / `akasha-pty`）、
CI 三平台矩阵、迁移后的开发循环复测 —— 其中**监听范围确实失效过，已修好并实测**。

`ROADMAP.md` 共 49 个条目（10 个阶段），阶段 1 的 5 个工作项已完成 4 项、1 项待 CI 实跑。
**下一步**：[`docs/plans/0201`](./plans/0201-output-batching.md)（`Transport` 输出合批）——
它是阶段 2「端到端最小终端」的第一步，前置 plan 0105 已完成。

**CI 仍未真正跑过**：仓库**没有配置任何 git remote**，所以这不是"等一次推送"，而是等托管到位。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + docs-check） | 退出码 **0**，5/5 全绿 |
| `just test` | **27 tests run: 27 passed**（`akasha-core` 8 + `akasha-pty` 14 + `akasha` 5） |
| `just check` / `just clippy`（`--workspace`） | 退出码 **0**，覆盖全部成员 |
| `just deny-offline` | `bans ok, licenses ok, sources ok` |
| `just docs-check` | 三部分全过（命令未漂移 / ROADMAP 49 条目在 3 行内 / plan 42 份 ≤200 行且索引一致） |
| `cargo tree -p akasha-core` / `-p akasha-pty` \| `grep -c tauri` | **0** / **0**（分层成立） |
| `ast-grep scan` | 退出码 **0**；`no-println` / `no-tauri-in-core-crates` / `no-ui-vocab-in-types` 三条规则均已用负例验证会红 |
| `just dev` | 起窗口；CLI 打印**两行**监听：`src-tauri` + `crates`；增量重编译 6.09–6.26s |
| `just doctor` | **13/13 passed**，`Connected to Victauri server`（v0.8.8，端口 7373） |
| IPC 端到端 | `greet` → `Hello, preflight! You've been greeted from Rust!`（与迁移前逐字相同） |
| `.github/workflows/ci.yml` | YAML 解析通过：2 个 job、矩阵三项、`e2e.needs = checks`；4 个 just 资产 URL 全部 HTTP 200 |
| `just syscheck` | webkit2gtk-4.1 2.52.6 / javascriptcoregtk-4.1 2.52.6 / gtk+-3.0 3.24.52 / librsvg-2.0 2.62.3 |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。
> 首次跑 `just test` 要编译测试目标，可能超过 60 秒，别误判为卡死。

## 待验证（本地跑不了）

- **CI 是否真能变绿** —— 三个矩阵 job + e2e job。本地只能校验 YAML 合法性、
  矩阵结构与资产 URL 可下载性；runner 环境差异（apt 包、macOS SDK、
  Windows 上 Tauri 构建脚本能否过类型检查、Windows runner 的 Git Bash）必须在 CI 上见分晓。
  **且仓库当前没有 remote**（见上）。
- **宿主 MCP 连不到沙箱内运行的 app** —— agent 的 bash 调用跑在 bwrap 里
  （`--tmpfs /tmp`、`--unshare-pid`），Victauri 的发现文件在沙箱私有 `/tmp`。
  这不是项目问题；沙箱内 `just doctor` 与 IPC 均正常（实测见上表）。

## 迁移后基线（2026-09-11 实测，根 workspace 布局）

ADR-0001 决策一落地后的实测记录 —— 后续再动 workspace 结构时拿它逐项对比。
迁移前的旧基线（`src-tauri/target`、47.53s 冷编译等）已随 plan 0101 归档。

| 项 | 实测结果 |
|---|---|
| 二进制落点 | `/home/lycurgus/akasha/target/debug/akasha`（**根 target**） |
| 增量重编译 | **6.09 / 6.18 / 6.26s**（迁移前为 6.09s，同量级） |
| dev server | Vite 就绪于 `http://localhost:1420`，端口在听 |
| Rust 监听范围 | CLI 打印**两行**：`Watching …/src-tauri` 与 `Watching …/crates` |
| 改 `crates/` 文件 | `Rebuilding application...` → `Compiling akasha-core` → `Compiling akasha` → 重启 |
| `just doctor` | 13/13 passed，端口 7373 |
| IPC 端到端 | `greet` 与迁移前返回同一字符串 |
| 前提条件 | **需要能写 `$HOME`**；沙箱内会刷 `dconf-CRITICAL` 与 WebKit 缓存 hard-link 告警，但 **app 仍正常起窗口** |

## 进行中 / 下一步

- [~] **plan 0102（CI 平台矩阵）**：本地部分完成，最终判据 = CI 上三个矩阵 job 与 e2e 全绿，
  **卡在没有 remote**。见 [`docs/plans/0102`](./plans/0102-ci-platform-matrix.md)
- [ ] **阶段 2 第一步**：[`docs/plans/0201`](./plans/0201-output-batching.md)（输出合批）。
  接上 `src-tauri → crates/*` 的真实依赖后，**顺手再改一次 `crates/` 下的文件**确认开发循环
  仍然生效（本次是用临时依赖证明的，见 plan 0104 的实施记录）

### 本轮完成（阶段 1）

- [x] **plan 0101 根 workspace**：根 `Cargo.toml`（members / `[workspace.lints]` / release profile）、
  `Cargo.lock` 与 `deny.toml` 上移、`.gitignore` 调整、构建缓存整体迁移 → 归档
- [x] **plan 0103 `crates/akasha-core`**：`SessionId` / `SessionKind` / `SessionRegistry`（登记 + 事件路由
  合一体，保证"关闭后不再投递"）/ `SessionEvent`；零依赖零 Tauri，8 个单测 → 归档
- [x] **plan 0105 `crates/akasha-pty`**：通用 `Transport` trait（`write` / `output_stream` / `resize` /
  `shutdown` / `exited` + `Capabilities`）、`portable-pty` 实现、`ShellLaunch`、
  `testing::FakeTransport`；14 个单测（含 3 个真实 PTY）→ 归档
- [x] **plan 0104 迁移后复测**：**发现并修复了监听范围的静默失效**
  （`build.additionalWatchFolders` 必须含 `../crates`）→ 归档
- [x] **两条 ast-grep 规则落地**（`no-tauri-in-core-crates` / `no-ui-vocab-in-types`），
  规则与代码同 PR，并用"应命中 + 诱饵不应命中"的负例验证
- [x] **`docs(agents)` 单独提交**：§6 登记两条规则 + 写明"规则必须用负例验证"

### 已定案（cyrene 裁定，2026-09-11）

| 项 | 结论 |
|---|---|
| P3 | **撤销** —— 豁免 webview 及其依赖栈的一切写入；判据改为"搬走文件夹后还能开" |
| SSH 实现 | **纯 Rust `russh`**，不调系统 `ssh`（→ 原 `ssh -G` 捷径作废） |
| `~/.ssh/config` | **只支持受限子集**；遇 `Match`/`Include` **显式报错**，不静默跳过 |
| Bitwarden 接入 | `bw` CLI 作**用户自备前置**（不打包）+ v1 只读导入 |
| `bw` 许可证 | **专有变体禁止分发（2.3(i)）与生产使用（2.1）**；OSS 变体是 GPL-3.0-only |
| **命名** | 后端容器叫 **`Session`**；原 `Session` trait 改名 **`Transport`**；SSH 连接叫 `Connection`。**后端类型名不得编码 UI 呈现方式**（详见 `scope.md` §1.2） |
| 连接模型 | **不复用** —— 每个 `Session` 各一条 SSH 连接 |
| 连接生命周期 | **= 拥有它的 `Session` 的生命周期**；关 `Session` 立刻断连（连带中止重连与传输） |
| 重连 | **3 次 + 指数退避**，然后标记失败 |
| 传输落盘 | **临时名 + 原子重命名**；失败/取消/关 `Session` 删除临时文件；不做断点续传 |
| `libudev` | 做成 **cargo feature，仅 Linux 编译时启用** |
| `akasha-vt` | **维持延后**；若必要则建于 **`crates/akasha-vt/`**，不在仓库根平铺 |

### 待实测 / 待确认

- [ ] **`bw` 对 `sshKey` 条目的非交互行为** —— 需要真实 vault 才能测
      （未解锁报错形态 / `bw list items --raw` 的 JSON 形状 / 条目是否稳定可见 /
      **如何分辨专有变体与 OSS 变体**）
- [ ] **可搬迁性收尾**（详见 [`portable.md`](./portable.md)）：
      数据目录相对可执行文件推导；**库里不存绝对路径**；便携模式由标记触发，
      不可写时**启动即报错**；实现"搬家后仍可用"的验证配方（当前只有方法）
- [ ] **`tauri.conf.json` 的 `csp` 目前是 `null`**，与 `AGENTS.md` §4.3「`csp` 不得为 `null`」相冲突；
      接入前端渲染时按需最小化放行（阶段 2）
- [ ] 阶段 2 接上真实 `src-tauri → crates/*` 依赖后，复验 `crates/` 改动仍触发重编译
- [ ] 托盘的 Linux 依赖 `libayatana-appindicator3` 已在 CI apt 列表里，需实测
- [ ] 单实例处理（第二个实例应唤起已有窗口，而不是各跑一套隧道）
- [ ] 动态转发（`-D`）需要自己实现 SOCKS5 服务端
- [ ] 托盘图标在 macOS/Linux 的尺寸与模板图标要求（UI 阶段）

## 结构现状（容易找错地方）

- **根 workspace**：成员 = `src-tauri` + `crates/*`；`Cargo.lock`、`deny.toml`、`target/` 都在仓库根。
  `[workspace.lints]` 定义 `unsafe_code = forbid` 与 `clippy::unwrap_used = warn`
  （成员必须写 `[lints] workspace = true` 才继承）。
- **两个纯逻辑 crate**（零 Tauri 依赖，由 ast-grep 强制）：
  `crates/akasha-core`（Session 模型，零依赖）、`crates/akasha-pty`（`Transport` + portable-pty）。
  `src-tauri` 目前**还没有**依赖它们 —— 真实依赖在 plan 0201 / 0202 建立。
- **文档三级粒度**：`ROADMAP.md`（判据）→ `docs/plans/TTxx-*`（手段）→ 本文件的坑（痕迹）。
  完成的 plan **整份移入 `docs/plans/archive/`**（不拼接、不追加，见 `AGENTS.md` §8）。
- 命令入口分两处：项目级在根 `justfile`，crate 级在 `src-tauri/justfile`。
  根只**转发**，命令体只有一处。**权威清单在 `docs/just.md` §2**，由 `just docs-check` 强制同步。
  ⚠️ crate 级配方必须显式带 `--workspace`（见坑 #20）。
- CI 只有一个文件 `.github/workflows/ci.yml`：`checks`（三平台矩阵）+ `e2e`（Linux，xvfb + 真 app）。

## 踩过的坑（避免重复踩）

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency** ——
   它不会传递给你；缺了 `cargo check` 退出码 101。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**，
   直接启用会让 check 全红。另外它**不需要在仓库根执行** —— 只要求当前目录含 `Cargo.toml`。
3. **just 的变量写 `$p`，不是 Make 的 `$$p`**（后者会展开成 PID）。
4. **just 用 justfile 所在目录作为配方工作目录** —— 这是把 crate 级命令搬进
   `src-tauri/justfile` 后可以彻底去掉 `--manifest-path` 的原因。
5. **系统库缺失只会在 cargo 的构建脚本阶段暴露**（`javascriptcore-rs-sys`）。
   本机是 **CachyOS（Arch 系）**，用 `pacman -S webkit2gtk-4.1`，不是 apt。
6. **Tauri 没有 Rust 热重载，Victauri 也不提供**。迭代速度靠"逻辑下沉到 `crates/` +
   bacon 独立循环"拿回来，不靠热重载。
7. **just 的 shebang 配方需要可写的 runtime dir**，在受限环境会失败 —— 用普通配方。
8. **仓库根没有 `Cargo.toml`** → 一切没显式指定 manifest 的 cargo 命令在根目录失败：
   `cargo build`、`cargo metadata`（原 CI 因此坏掉）、`cargo fmt --all`（退出码 141）。
   已由 plan 0101 整体解决。
9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** ——
   `fmt-check` 会红。跑一次 `just fmt` 规范化即可（已做）。
10. **CI 里 `libappindicator3-dev` 在 ubuntu-latest 上已不存在**，要用
    Tauri 官方列表里的 `libayatana-appindicator3-dev`（还漏了 `libxdo-dev`）。
11. **受限环境下"写工作区之外被拒"看起来像工具/代码故障**（`just dev` 的 `os error 30`、
    `just deny` 的 advisory lock 失败）。已写成规则：识别 → **直接提权重试**，
    见 `AGENTS.md` §1 末。**不要把它当成项目 bug 去翻代码。**
12. **`git checkout <file>` 会静默丢弃未提交的改动** —— 做负例自检时用它"还原"过一次，
    结果整段文档被回滚，靠提交前核对才发现。对未提交的工作区改动，`git checkout`
    是**破坏性操作**，不是"还原"。负例自检请用 `cp` 备份 + `cp` 还原。
13. **正向校验若不限定到目标段落就形同虚设** —— `docs-check` 曾只检查"全文提到过某命令"，
    于是删掉 §2 表格里的一行后**仍然通过**（排错段落里顺带提及了同一条命令）。
    改用 `awk` 取 §2 段落再匹配，负例才如实报错。
14. **`docs-check` 的反向检查原本只扫 `AGENTS.md` + `docs/just.md`** —— 新加的文档
    （如 `docs/scope.md`）里的过期命令完全不被覆盖。已扩到
    `AGENTS.md` / `ROADMAP.md` / `docs/**/*.md`（扩展后实测零过期引用，纯增益）。
15. **后台遗留的 `just dev` 会在仓库根重跑 cargo** ——
    `error: could not find Cargo.toml in /home/lycurgus/akasha`（即坑 #8）。
    重建失败后 **app 不再启动，但 Vite dev server 仍在监听 1420**。
    现象很有迷惑性：**端口在听、HTTP 200，但 Victauri 说 app 没在运行**。
    判别方法：`just doctor`（它直接问 app，而不是问端口）。
16. **含反引号的 grep 模式在 justfile 配方里必须整体放进单引号** ——
    配方体由 bash 执行，裸反引号会被当**命令替换**跑掉。
    `docs-check` 的 ROADMAP 纪律检查踩过这一点。
17. **多文件行数检查要逐文件取（`wc -l < 单文件`，或 awk 的 `FNR`）** ——
    awk 的 `NR` 会**跨文件累加**：42 份小 plan 会被报成"某一份 1813 行"。
    `docs-check` 的 plan 预算检查因此对每个文件单独 `wc -l`，不要图省事用一次 awk。
18. **`[profile.*]` 写在 workspace 成员里会被 cargo 静默忽略** —— 每个 workspace 只认
    root 的那一份，成员里写的只产出一行 warning（`profiles for the non root package
    will be ignored`）。不迁移就等于**悄悄丢掉 `lto` / `strip` / `panic = "abort"`**，
    而没有任何门禁会红。plan 0101 把 release profile 上移到了根 `Cargo.toml`。
19. **整体 `mv` 构建缓存（`target/`）会留下写死的绝对路径** —— 省下重建时间，
    但 `target/debug/build/<pkg>/output` 里记着旧的绝对路径，而 cargo 会把它们作为
    `DEP_*` 环境变量**原样重放**给下游；于是 tauri 的构建脚本去读一个已不存在的
    permissions 目录，报错看起来像"代码坏了"。处置：删掉那些构建脚本产物目录让其重跑。
    **下次迁 target 直接删掉重建**，别为省时间搬缓存。
20. **`cargo` 在成员目录里只选当前包** —— crate 级配方（cwd = `src-tauri/`）如果
    不显式带 `--workspace`，`crates/*` 的 check / clippy / test 会被**静默漏掉**：
    `just ready` 全绿，但那些 crate 根本没被编译过。实测：加上 `akasha-core`
    的 8 个单测后 `just test` 仍只报 5 个；加 `--workspace` 后才是 27 个。
21. **tauri CLI 默认只监听 `src-tauri`** —— 根 workspace 之后纯逻辑都在 `crates/`，
    少了 `src-tauri/tauri.conf.json` 里的 `build.additionalWatchFolders`，
    改 `crates/` 不会触发任何重编译：**开发循环静默失效**（门禁全绿，改了看不到效果）。
    两个附带的坑：命令行形式 `--additional-watch-folders` 的路径**相对 app 目录**解析
    （不是 cwd），写错只**警告后继续**（`not found, ignoring`）；
    而且 `src-tauri` 若尚未依赖该 crate，即使监听生效也是"空转重建"，看着像没反应。

## 环境

CachyOS（Arch 系）/ rustc 1.98.1 / cargo 1.98.1 / node 26.8.2 / pnpm 12.3.4 / mise 2026.9.1
