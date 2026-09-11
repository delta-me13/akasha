# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-11

## 一句话

地基阶段收尾：项目可编译、`just ready` 全绿、CI 已重写完成；**CI 的真实验证要等首次推送**。

**范围与文档已同步完毕，可以开工。** 产品范围为三后端（local/ssh/serial）+ SFTP +
四套配置池 + Bitwarden + SSH 端口转发（L/R/D）+ 常驻托盘，见
[`docs/scope.md`](./scope.md)。阶段划分已按新范围重写（[`ROADMAP.md`](../ROADMAP.md)，
10 个阶段，**CI 平台矩阵已提前到阶段 1**），ADR 收敛为 3 份
（[`docs/adr/README.md`](./adr/README.md)）。

**下一步**：[`docs/plans/0001`](./plans/0001-root-workspace.md)（根 workspace）——
ADR-0001 决策二已裁定，不再阻塞。

两条定位性约束：**可搬迁**（搬走 bin 文件夹后仍能开且数据还在）与**不依赖 OS 组件**
（webview 及其依赖栈的写入已由用户豁免，见 `scope.md` §1.1）。

## 已验证为绿（命令 + 实际结果）

| 命令 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + docs-check） | 退出码 **0** |
| `just docs-check` | `✅ 文档命令与 justfile 同步（权威清单 docs/just.md §2）` |
| `just check` | 退出码 **0** |
| `just lint`（clippy `-D warnings` + ast-grep scan） | 退出码 **0** |
| `just deny-offline`（licenses / bans / sources） | `bans ok, licenses ok, sources ok` |
| `just syscheck` | webkit2gtk-4.1 2.52.6 / javascriptcoregtk-4.1 2.52.6 / gtk+-3.0 3.24.52 / librsvg-2.0 2.62.3 |
| `.github/workflows/ci.yml` YAML | 用 `js-yaml` 解析通过；2 个 job，`e2e.needs = checks` |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。
> 首次 `just ready` 要编译测试目标（nextest），可能超过 60 秒，别误判为卡死。

## 待验证（本地跑不了）

- **CI 是否真能变绿** —— 需要首次 push。本地只能校验 YAML 合法性、命令存在性、
  依赖/action 版本可获取性；runner 环境（apt 包可用性、xvfb 起 app）必须在 CI 上见分晓。
- **`just dev` 在根 workspace 布局下能否起窗口** —— ADR-0001 决策一的验收项，
  见 `docs/plans/0001` 第 4 条。当前布局下尚未跑过 `tauri dev`。

## 预跑基线（2026-09-11 实测，当前布局）

ADR-0001 决策一（根 workspace 迁移）**迁移前**的实测记录 —— 迁移后必须拿它逐项对比。

| 项 | 实测结果 |
|---|---|
| 二进制落点 | `src-tauri/target/debug/akasha`（日志 `Running target/debug/akasha`，cwd = `src-tauri`），393 MB |
| 冷编译 | **47.53s** |
| 增量重编译 | **6.09s** |
| dev server | Vite 就绪于 `http://localhost:1420`（实测 HTTP 200） |
| Rust 监听范围 | CLI 打印 `Watching /home/lycurgus/akasha/src-tauri for changes` |
| app 数据目录 | `~/.local/share/fans.cyrene.akasha-terminal/`（CacheStorage / hsts-storage.sqlite） |
| Victauri | 连上，`identifier = fans.cyrene.akasha-terminal`，35 个工具，端口 7373 |
| **IPC 端到端** | `invoke('greet', {name:'preflight'})` → `Hello, preflight! You've been greeted from Rust!` ✅ |
| 前端渲染 | `dom_snapshot` 拿到完整模板 UI（heading / link / form / textbox / button） |
| 前提条件 | **需要能写 `$HOME`**；受限环境下会在 `Failed to setup app: 只读文件系统 (os error 30)` panic |

> **当前没有任何 `just dev` 在运行**（2026-09-11 已停掉遗留的那个；它当时已半死 ——
> app 未运行、仅 Vite 占着 1420，详见下面「踩过的坑」#15）。
> 1420 端口已释放，开工时从根目录**重新**起一个。

**预跑得出的、迁移后必须复测的点**（已写入 `docs/plans/0001`）：
CLI 打印的监听路径是 `src-tauri`。根 workspace 迁移后 `crates/*` 落在 `src-tauri` 之外，
**必须确认改 `crates/` 下的文件仍会触发重编译与重启** —— 否则开发循环会静默失效，
而 `cargo check` 完全看不出来。

## 进行中 / 下一步

- [ ] **落地决策一**（根 workspace）：步骤与验收见 [`docs/plans/0001`](./plans/0001-root-workspace.md)
      —— ADR-0001 **已接受**（决策二裁定见其 §0.3），前置条件已满足，**可以开工**
      注意其中"迁移后必须复测"的一项：改 `crates/` 下的文件仍要能触发重编译与重启

### 文档同步（本轮已完成）

- [x] `AGENTS.md` §8 文档体系表加入 `docs/scope.md` 与 `docs/portable.md`；
      §3.1 加入**命名约定**；§6 加入计划规则 `no-ui-vocab-in-types`
- [x] `ROADMAP.md` **按新范围重写**（4 阶段 → 10 阶段；CI 平台矩阵提前到阶段 1；
      每个条目都带可执行验收标准）
- [x] **ADR 队列收敛为 3 份**：0001（切分与后端抽象）/ 0002（机密存储与可搬迁）/
      0003（SSH 栈与资源模型）。原 7~8 份的合并去向见 [`docs/adr/README.md`](./adr/README.md)。
      0002 / 0003 **不在现在写** —— 等真正要动那块代码之前再写，
      避免塞满"还没被代码验证过的细节"
- [x] `docs/scope.md` 全文 `tab` → `Session` 统一（40 处），并新增 §1.2 命名约定
- [x] **汇总类文档的纪律成文并做成检查**（起因：ROADMAP 会慢慢长实现细节）：
      `AGENTS.md` 新增 **§8.1**（三类不许出现的内容 + 搬运去向 + ROADMAP 硬预算）；
      搬运表与"三级粒度"进 [`docs/README.md`](./README.md)；
      **`just docs-check` 增加三个检查**：条目 ≤3 行、无代码块、反引号里无命令调用。
      已用三个负例分别验证（粘命令 / 条目 4 行 / 塞代码块 → 都如实报错）
- [x] `ROADMAP.md` 按新纪律**清掉 17 处细节泄漏**（验证手段与理由移出：
      `cargo metadata`、`introspect {...}`、`cargo tree 可证` 等 → 归 plan 的验收命令）
- [x] **`docs/scope.md` §7 下沉为 [`bitwarden.md`](./bitwarden.md)**（626 → **534 行**）：
      许可证条款原文、四条技术路的核实结果、条目字段结构与指纹推导搬走；
      `scope.md` 只留结论表 + 指针。这是 §8.1 那条"某一节超过约一屏就下沉"的首次应用
      （此前的 `scope.md` → `portable.md` 是同类，但那时还没成文）

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
- [ ] 托盘的 Linux 依赖 `libayatana-appindicator3` 已在 CI apt 列表里，需实测
- [ ] 单实例处理（第二个实例应唤起已有窗口，而不是各跑一套隧道）
- [ ] 动态转发（`-D`）需要自己实现 SOCKS5 服务端
- [ ] 托盘图标在 macOS/Linux 的尺寸与模板图标要求（UI 阶段）

## 结构现状（容易找错地方）

- 命令入口分两处：项目级在根 `justfile`，crate 级在 `src-tauri/justfile`。
  根只**转发**，命令体只有一处。**权威清单在 `docs/just.md` §2**，由 `just docs-check` 强制同步。
- CI 只有一个文件 `.github/workflows/ci.yml`（原 `victauri.yml` 已删除并合并进来）。

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
   详见 `docs/adr/0001` §2.2。若采纳决策一，这类问题会整体消失。
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
    ⚠️ 工作区切分迁移（plan-0001）后**必须停掉旧进程再重起**，
    否则旧进程的监听范围还是老的 `src-tauri`，会让迁移后的验收项失真。
16. **含反引号的 grep 模式在 justfile 配方里必须整体放进单引号** ——
    配方体由 bash 执行，裸反引号会被当**命令替换**跑掉。
    `docs-check` 的 ROADMAP 纪律检查踩过这一点。

## 环境

CachyOS（Arch 系）/ rustc 1.98.1 / cargo 1.98.1 / node 26.8.2 / pnpm 12.3.4 / mise 2026.9.1
