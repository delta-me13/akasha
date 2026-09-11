# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-11

## 一句话

**阶段 1 布局收口**：Rust 成员全部收进 `src-tauri/`（仓库根零 Rust 成员，见
[ADR-0004](./adr/0004-rust-workspace-under-src-tauri.md)），两个纯逻辑 crate 立住，
CI 改成**一份工作流同时支持 Gitea 与 GitHub**。

`ROADMAP.md` 共 50 个条目（10 个阶段），阶段 1 的 6 个工作项已完成 5 项、1 项等 CI 实跑。
**下一步**：[`docs/plans/0201`](./plans/0201-output-batching.md)（`Transport` 输出合批）——
阶段 2「端到端最小终端」的第一步，前置 plan 0105 已完成。

**CI 仍未真正跑过**：仓库**没有配置任何 git remote**。计划的顺序是
**先在 Gitea 上跑通 CI/CD，再推 GitHub**（cyrene 2026-09-11 指示）。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + ci-check + test + deny-offline + docs-check） | 退出码 **0**，6/6 全绿 |
| `just test` | **27 tests run: 27 passed**（`akasha-core` 8 + `akasha-pty` 14 + `akasha` 5） |
| `just check` / `just clippy`（`--workspace`） | 退出码 **0**，覆盖全部三个成员 |
| `just deny-offline`（配置在 `src-tauri/deny.toml`） | `bans ok, licenses ok, sources ok` |
| `just docs-check` | 三部分全过（ROADMAP 50 条目在 3 行内 / plan 43 份 ≤200 行且索引一致） |
| `just ci-check` | 通过；**两条负例都报红**（加一行 `runner` 上下文、建一个空的 `.gitea/workflows/`） |
| `cargo tree -p akasha-core` / `-p akasha-pty` \| `grep -c tauri` | **0** / **0**（分层成立） |
| `ast-grep scan` | 退出码 **0**；三条规则均已用负例验证会红（**成员搬家后重新验过**） |
| `just dev` | 起窗口；CLI 打印**一行** `Watching …/src-tauri`（成员在其内，默认覆盖）；增量重编译 6.09–6.26s |
| `just doctor` | **13/13 passed**，`Connected to Victauri server`（v0.8.8，端口 7373） |
| IPC 端到端 | `greet` → `Hello, preflight! You've been greeted from Rust!` |
| `.github/workflows/ci.yml` | YAML 解析通过：3 个 job、矩阵 windows/macos 带 `github.server_url` 门、`e2e.needs = checks-linux`；四个 just 资产 URL 全部 HTTP 200 |
| `just syscheck` | webkit2gtk-4.1 2.52.6 / javascriptcoregtk-4.1 2.52.6 / gtk+-3.0 3.24.52 / librsvg-2.0 2.62.3 |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。
> 首次跑 `just test` 要编译测试目标，可能超过 60 秒，别误判为卡死。

## 待验证（本地跑不了）

- **CI 在两个 forge 上是否真能变绿** —— 顺序是**先 Gitea，后 GitHub**。
  本地只能校验 YAML 结构、双 forge 约束（`just ci-check`）与资产 URL 可下载性。
  三条只在真 runner 上才见分晓的风险（Gitea 是否在调度前求值 job 级 `if`、
  缓存服务行为、E2E 能否在 job 容器里起窗口）逐条记在
  [`docs/plans/0102`](./plans/0102-ci-platform-matrix.md) 的实施记录里。
- **宿主 MCP 连不到沙箱内运行的 app** —— agent 的 bash 调用跑在 bwrap 里
  （`--tmpfs /tmp`、`--unshare-pid`），Victauri 的发现文件在沙箱私有 `/tmp`。
  这不是项目问题；沙箱内 `just doctor` 与 IPC 均正常（实测见上表）。

## 当前基线（2026-09-11 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` |
| 二进制落点 | `src-tauri/target/debug/akasha` |
| target 目录 | `src-tauri/target`（13G，含迁移前的缓存） |
| 增量重编译 | **6.09 / 6.18 / 6.26s** |
| dev server | Vite 就绪于 `http://localhost:1420` |
| Rust 监听范围 | CLI 打印**一行**：`Watching …/src-tauri`（`crates/` 在其内部，无需额外配置） |
| 改 `src-tauri/crates/` 文件 | `Rebuilding application...` → 重启 |
| `just doctor` | 13/13 passed，端口 7373 |
| IPC 端到端 | `greet` → 与迁移前逐字相同 |
| 前提条件 | **需要能写 `$HOME`**；沙箱内会刷 `dconf-CRITICAL` 与 WebKit 缓存 hard-link 告警，但 **app 仍正常起窗口** |

## 进行中 / 下一步

- [~] **plan 0102（CI 平台矩阵 + 双 forge）**：本地部分完成，最终判据 = **Gitea 上全绿之后再 GitHub 全绿**，
  卡在没有 remote。见 [`docs/plans/0102`](./plans/0102-ci-platform-matrix.md)
- [ ] **阶段 2 第一步**：[`docs/plans/0201`](./plans/0201-output-batching.md)（输出合批）。
  接上 `src-tauri → src-tauri/crates/*` 的真实依赖后，**顺手再改一次 `crates/` 下的文件**
  确认开发循环仍然生效（本次是用临时依赖证明的，见 plan 0104 的实施记录）

### 本轮完成（阶段 1 收口）

- [x] **plan 0106 布局收口**：`crates/` → `src-tauri/crates/`，`Cargo.lock` / `deny.toml` / `target/`
  随之外移，根 `Cargo.toml` 删除，workspace root 改为 `src-tauri/Cargo.toml` → 归档
- [x] **ADR-0004** 记录该决定并取代 ADR-0001 决策一（ADR-0001 按规则只追加取代指针）
- [x] **CI 双 forge**：一份工作流，Linux 两个 forge 都跑，Windows/macOS 用
  `github.server_url` 整条 job 门在 GitHub —— 并新增 `just ci-check` 把四条约束变成机器检查
- [x] **规则路径重验**：ast-grep 规则的 `files:` 改为 `src-tauri/crates/**` 后，
  用探针重新验证它们仍会命中（否则规则会**静默失效**）
- [x] `AGENTS.md` §1/§3.1/§7/§9/§11 同步布局与 `ready` 步骤（**宪法改动，单独提交**）

### 已定案（cyrene 裁定）

| 项 | 结论 |
|---|---|
| **Rust 成员位置** | **全部收在 `src-tauri/` 下**，仓库根不放 Rust 成员或 manifest（2026-09-11，见 ADR-0004） |
| **CI** | 一份工作流**同时支持 Gitea 与 GitHub**；**先在 Gitea 上验 CI/CD，再推 GitHub**（2026-09-11） |
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
| `akasha-vt` | **维持延后**；若必要则建于 **`src-tauri/crates/akasha-vt/`**，不在仓库根平铺 |

### 待实测 / 待确认

- [ ] **Gitea 上的 CI 实跑**（三条风险见上）
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

- **workspace root 在 `src-tauri/`**（ADR-0004）：成员 = `akasha`（app 包）+ `crates/*`
  （`akasha-core` / `akasha-pty`）。`Cargo.lock`、`deny.toml`、`target/` 都在 `src-tauri/` 下。
  **仓库根没有 `Cargo.toml`** —— 在根目录直接跑 `cargo …` 会失败（坑 #8），一律用 `just` 转发。
- **两个纯逻辑 crate**（零 Tauri 依赖，由 ast-grep 强制）：`akasha-core`（Session 模型，零依赖）、
  `akasha-pty`（`Transport` + portable-pty）。`src-tauri` 目前**还没有**依赖它们 ——
  真实依赖在 plan 0201 / 0202 建立。
- **文档三级粒度**：`ROADMAP.md`（判据）→ `docs/plans/TTxx-*`（手段）→ 本文件的坑（痕迹）。
  完成的 plan **整份移入 `docs/plans/archive/`**（不拼接、不追加，见 `AGENTS.md` §8）。
  归档 plan 里的路径按**当时**布局书写（`crates/…` 现读作 `src-tauri/crates/…`）。
- 命令入口分两处：项目级在根 `justfile`，crate 级在 `src-tauri/justfile`（cwd 与 manifest 同层）。
  根只**转发**，命令体只有一处。**权威清单在 `docs/just.md` §2**，由 `just docs-check` 强制同步。
  ⚠️ crate 级配方必须显式带 `--workspace`（坑 #20）。
- CI 只有一个文件 `.github/workflows/ci.yml`：`checks-linux` + `checks-other`（仅 GitHub）+ `e2e`。
  **不要创建 `.gitea/workflows/`**（坑 #24）。

## 踩过的坑（避免重复踩）

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency** ——
   它不会传递给你；缺了 `cargo check` 退出码 101。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**，
   直接启用会让 check 全红。另外它**不需要在仓库根执行** —— 只要求当前目录含 `Cargo.toml`。
3. **just 的变量写 `$p`，不是 Make 的 `$$p`**（后者会展开成 PID）。
4. **just 用 justfile 所在目录作为配方工作目录** —— 这是 crate 级命令放在
   `src-tauri/justfile` 后可以彻底去掉 `--manifest-path` 的原因（workspace root 也在那里）。
5. **系统库缺失只会在 cargo 的构建脚本阶段暴露**（`javascriptcore-rs-sys`）。
   本机是 **CachyOS（Arch 系）**，用 `pacman -S webkit2gtk-4.1`，不是 apt。
6. **Tauri 没有 Rust 热重载，Victauri 也不提供**。迭代速度靠"逻辑下沉到 `crates/` +
   bacon 独立循环"拿回来，不靠热重载。
7. **just 的 shebang 配方需要可写的 runtime dir**，在受限环境会失败 —— 用普通配方。
8. **仓库根没有 `Cargo.toml`** → 一切没显式指定 manifest 的 cargo 命令在根目录失败：
   `cargo build`、`cargo metadata`、`cargo fmt --all`。
   2026-09-11 起这是**有意为之**（ADR-0004）：manifest 在 `src-tauri/`，用 `just` 转发或显式
   `--manifest-path`。踩到它的人多半是照着一份"根目录有 workspace"的旧记忆在操作。
9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** ——
   `fmt-check` 会红。跑一次 `just fmt` 规范化即可（已做）。
10. **CI 里 `libappindicator3-dev` 在 ubuntu-latest 上已不存在**，要用
    Tauri 官方列表里的 `libayatana-appindicator3-dev`（还漏了 `libxdo-dev`）。
11. **受限环境下"写工作区之外被拒"看起来像工具/代码故障**（`just dev` 的 `os error 30`、
    `just deny` 的 advisory lock 失败）。已写成规则：识别 → **直接提权重试**，
    见 `AGENTS.md` §1 末。**不要把它当成项目 bug 去翻代码。**
    （补充实测：在 bwrap 沙箱里 `just dev` **仍然能起窗口** —— dconf / WebKit 缓存的写入失败
    只是一串告警，不是致命错误。）
12. **`git checkout <file>` 会静默丢弃未提交的改动** —— 做负例自检时用它"还原"过一次，
    结果整段文档被回滚，靠提交前核对才发现。对未提交的工作区改动，`git checkout`
    是**破坏性操作**。负例自检请用 `cp` 备份 + `cp` 还原。
13. **正向校验若不限定到目标段落就形同虚设** —— `docs-check` 曾只检查"全文提到过某命令"，
    于是删掉 §2 表格里的一行后**仍然通过**（排错段落里顺带提及了同一条命令）。
    改用 `awk` 取 §2 段落再匹配，负例才如实报错。
14. **`docs-check` 的反向检查原本只扫 `AGENTS.md` + `docs/just.md`** —— 新加的文档
    里的过期命令完全不被覆盖。已扩到 `AGENTS.md` / `ROADMAP.md` / `docs/**/*.md`。
15. **后台遗留的 `just dev` 会在仓库根重跑 cargo** —— 重建失败后 **app 不再启动，但 Vite
    dev server 仍在监听 1420**，现象很有迷惑性。判别方法：`just doctor`（它直接问 app）。
16. **含反引号的 grep 模式在 justfile 配方里必须整体放进单引号** ——
    配方体由 bash 执行，裸反引号会被当**命令替换**跑掉。
17. **多文件行数检查要逐文件取（`wc -l < 单文件`，或 awk 的 `FNR`）** ——
    awk 的 `NR` 会**跨文件累加**，42 份小 plan 会被报成"某一份 1813 行"。
18. **`[profile.*]` 写在 workspace 成员里会被 cargo 静默忽略** —— 每个 workspace 只认
    root 的那一份（`profiles for the non root package will be ignored`）。不迁移就等于
    **悄悄丢掉 `lto` / `strip` / `panic = "abort"`**，而没有任何门禁会红。
19. **整体 `mv` 构建缓存（`target/`）会留下写死的绝对路径** —— `target/debug/build/<pkg>/output`
    里记着旧路径，cargo 会把它们作为 `DEP_*` **原样重放**给下游；于是构建脚本去读一个
    已不存在的目录，报错看起来像"代码坏了"。处置：删掉那些构建脚本产物目录让其重跑
    （**删除要挑准**：按旧绝对路径 grep 出来的那些目录，别清整个 `build/`）。
    搬到 `src-tauri/` 时又踩了一次（13 处）—— **下次迁 target 直接删掉重建更省事**。
20. **`cargo` 在成员目录里只选当前包** —— crate 级配方（cwd = `src-tauri/`）如果
    不显式带 `--workspace`，`crates/*` 的 check / clippy / test 会被**静默漏掉**：
    `just ready` 全绿，但那些 crate 根本没被编译过。实测：加上 `akasha-core` 的 8 个单测后
    `just test` 仍只报 5 个；加 `--workspace` 后才是 27 个。
21. **tauri CLI 默认只监听 `src-tauri`** —— 成员若在它**外面**，改成员不触发任何重编译：
    **开发循环静默失效**（门禁全绿，改了看不到效果）。当时的修法是
    `build.additionalWatchFolders`（其路径**相对 app 目录**解析，写错只**警告后继续**）。
    2026-09-11 起成员收进了 `src-tauri/`（ADR-0004），这条约束**从根上消失** ——
    但"别把成员挪到 `src-tauri/` 外面"这条纪律要留着。
22. **cargo 的空 glob 是硬错误，不是空匹配** —— `members = ["crates/*"]` 在 `crates/`
    不存在时报 `failed to load manifest for workspace member .../crates/*`。
    所以 workspace 定义与首个 crate 的创建有先后依赖。
23. **justfile 配方里不能出现完整的 `{{ … }}`** —— just 会把它当插值去求值，
    报 `error: unknown start of token '.'`（`runner.os` 那种内容不是合法表达式）。
    要写字面量就用 `{{{{`（渲染成一个 `{{`）。`ci-check` 的匹配模式因此写成
    `runner\.(os|arch|…)` 而不是那个上下文的原样。
24. **Gitea 只读"第一个存在"的 workflow 目录** —— `[actions] WORKFLOW_DIRS` 默认
    `.gitea/workflows,.github/workflows`（PR #36619）。仓库里**一旦有了 `.gitea/workflows/`**，
    Gitea 就只用它、**忽略 `.github/workflows/`**，CI 静默不跑。
    所以"为了兼容 Gitea 而建 `.gitea/workflows/`"是**反向操作**；`just ci-check` 守着这条。
25. **Gitea Actions 的兼容边界**（与 GitHub 不同，会直接让工作流跑不起来）：
    上下文里**没有 `runner.*`**（其 Quickstart 用过，但未文档化）；
    `runs-on` 只支持 `xyz` / `[xyz]`，不支持复杂对象形式；
    表达式函数**只支持 `always()`**（`contains` / `format` / `fromJSON` 都不可用）；
    没有 Windows/macOS runner（那两条分支必须整条 job 门住，**步骤级 `if` 拦不住排队**）。
    出处：Gitea 文档 *Compared to GitHub Actions* 与 *Variables*。

## 环境

CachyOS（Arch 系）/ rustc 1.98.1 / cargo 1.98.1 / node 26.8.2 / pnpm 12.3.4 / mise 2026.9.1
