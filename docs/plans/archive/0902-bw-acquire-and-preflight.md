# Plan 0902: `bw` 的获取与前置检查

- **关联**：ROADMAP 阶段 9 ·「`bw` 的获取与前置检查」
- **前置**：[ADR-0007](../adr/0007-bitwarden-cli-acquisition.md)（实现中）· 无代码依赖
- **状态**：已完成（2026-09-15）

## 目标

让"这台机器上有没有一个能用的 `bw`"这件事**有答案、且答案是用户能照着动的**。

| 要发生什么 | 由谁负责 |
|---|---|
| 两个轴（二进制来源 × CLI 状态目录）各自可切，默认都取 `host` | ADR-0007 D4（用户当次指令） |
| `host` 轴上找不到 `bw` → 说清是这一件事，并给出下载动作 | 本条 |
| `host` 轴上探到**专有**变体 → 给出许可证提示 | 本条（`bitwarden.md` §2.2 的判据） |
| 运行时下载：解析最新 `cli-v*` → 取 OSS 资产 → 算 SHA-256 → 解包 → 落盘 | 本条 |
| 两个轴的选择**记得住**（重启之后还在） | 本条（`config.json` 多一个对象） |

## 非目标

- **登录 / 解锁 / 锁定与界面**：plan 0905。本条只把 CLI 摆到"能调"的位置。
- 只读导入（plan 0903）与离线缓存（plan 0904）。
- **不做后台自动更新**：检查与下载都是用户动作（ADR-0007 D2 的代价由界面承担）。
- 不打包、不分发任何二进制（`scope.md` §10）。

## 前置检查（已完成的部分，实测记录见 [`../bitwarden.md`](../bitwarden.md)）

| 事实 | 出处 |
|---|---|
| 资产名是 `bw-oss-<os>[-<arch>]-<版本>.zip`，包里是单个可执行文件 | `bitwarden.md` §7 实测 |
| 变体判据 = `bw --help` 的命令表里有没有 `device-approval` | `bitwarden.md` §2.2 实测 |
| 上游自 `cli-v2025.6.0` 起**不发布 SHA-256 文件** | `bitwarden.md` §2 实测 |
| 依赖（`ureq` / `zip` / `sha2`）在本机缓存里，离线可解析与编译 | `cargo check -p akasha-bw --offline` 实测 |

## 步骤

1. **新 crate `src-tauri/crates/akasha-bw`**（零 Tauri 依赖，`AGENTS.md` §3.1）。
   模块边界：`location`（两个轴与落点）/ `variant`（变体判定，纯函数）/ `status`（解析，
   纯函数）/ `cli`（调用、环境、超时、失败分类）/ `session`（受保护页）/
   `acquire`（下载与安装）/ `testing`（进程内假上游，供本 crate 与 app 的 E2E 共用）。
   依赖：`akasha-store`（受保护页，与 `akasha-ssh` 同一个先例）、`ureq`（阻塞 HTTP，
   rustls + ring，与 `russh` 同一个 crypto 后端）、`zip`（只开 `deflate`）、`sha2`、
   `serde` / `serde_json` / `thiserror` / `tracing`。
2. **机密的两条通路**：主密码经 `--passwordenv`、session key 经 `BW_SESSION` 环境变量 ——
   两条都**不进 argv**（ADR-0007 D7 / D8）。`stdin` 接 `null` 且一律带 `--nointeraction`，
   否则缺凭据时子进程会走交互提问并一直等下去（而它手里攥着 app 的那把锁）。
3. **app 接线**（`src-tauri/src/bitwarden.rs`）：`Paths::new(data_dir)` + 两个轴 →
   `Located` → `Cli`；一条长驻的 `Mutex` 保证**同一时刻只有一个 `bw` 进程**（CLI 自己在 `data.json`
   上没有任何互斥）。
4. **配置持久化**：`config.json` 增一个可选对象 `bitwarden: {binary, appdata}`（取值
   `host` / `managed`），默认两个 `host`；**第一次写**才创建文件（`config.rs` 此前只读，
   写路径与本条一起落）。
5. **IPC**：`bw_cli_status`（两轴 + 解析结果 + 变体 + 许可证提示句）、
   `bw_cli_install`（下载，**async**：会拉 45 MB）、`bw_cli_settings(binary, appdata)`（切换）。
   probe `bitwarden` 报同一份读数。

## 验收命令

```bash
# 1) crate 层：变体判定、状态解析、下载与解包、失败分类、超时、机密走环境不走 argv
cargo nextest run --package akasha-bw
# 预期：46 passed（39 单测 + 7 假 CLI 集成），0 failed

# 2) 门禁
just ready
# 预期：✅ just ready 全绿（6/6）

# 3) 真实路径（Victauri，app 由 just dev 常驻）：
#    前置：`which bw` 为空 —— 这台机器上确实没有宿主机那一份
#    invoke_command: bw_cli_status   {binary: "host"}
#      → problem 说的是"PATH 里没有 bw"，而不是"还没下载"（两件事分得开）
#    invoke_command: bw_cli_settings {binary: "managed", appdata: "managed"}
#    invoke_command: bw_cli_install
#      → 真的从上游拉一份 bw-oss，返回版本与 SHA-256（**这条要联网**）
#    invoke_command: bw_cli_status
#      → version = 上游最新版、variant = oss、licenseNotice 为空
```

## 回滚

- 整个功能下线：`src-tauri/src/bitwarden.rs` 的三条命令与 probe 摘掉。
  ⚠️ `config.json` 的 `bitwarden` 字段要**一起处理**：`File` 是 `deny_unknown_fields` 的，
  老版本遇到这个字段会把整份配置判为非法（表现为"回到默认值 + 一条 warn"）。
- 只回滚默认值：把 `Settings::default()` 改成别的取值即可；磁盘上已下载的版本目录不动。

## 实施记录

### 落地时定下的四件事

1. **只拼 `bw-oss-` 的资产名**：没有"下哪一版"这个开关（ADR-0007 D1）。两份资产的文件名
   只差 `oss` 一段，写错一段就是许可证问题，所以拼法只有一处（`asset_name`），
   并被单测按平台 / 架构逐个钉住（含"Windows arm64 没有资产"这条反例）。
2. **SHA-256 是记录，不是校验结论**：上游不发布校验文件（实测），拿到的是"这次下载的那些
   字节的哈希"，进日志与界面供事后核对；**不拿上一次的哈希去拦这一次** —— 那会把一次上游
   更新变成一次"校验失败"。
3. **变体判据读 `--help` 的命令表**，且**判不出来时报"判不出"**：默认当 OSS 等于让许可证
   提示静默消失。诱饵用例（`device-approval` 只出现在其它段落）与"命令表为空"两态都钉住。
4. **`bw` 的启动慢**（约 140 MB 的 Node SEA）：纯本地命令也给 20 秒，`login` / `unlock`
   给 120 秒；超时路径**杀 + 收**（`kill` 之后 `wait`），不留僵尸。

### 验收命令的实际输出

```console
$ cargo nextest run --package akasha-bw
46 tests run: 46 passed（39 单测 + 7 条假 `bw` 的集成用例）

$ just ready
✅ just ready 全绿（6/6）

$ cargo test -p akasha-bw --test probe_real_upstream -- --nocapture   # 一次性探针，读完数即删
PROBE 上游最新 cli 版本 = 2026.8.0
PROBE 落点 = /tmp/akasha-bw-real-70/bitwarden/bw-2026.8.0/bw
PROBE sha256 = d8bbc213d3dbdb701709386af6ea3bd763482f922ae0b538fdc3fdb88b1b1704
PROBE 体积 = 141819984 字节
PROBE crate 报的版本 = 2026.8.0
PROBE 变体 = Oss
PROBE status --raw = Ok((Unauthenticated, None))
# 与 `sha256sum` 直接算上游那个 zip 的结果**逐字符相同**；24.63 s
```

### 与计划的差异

- **"真实路径走 app + Victauri MCP"这条在本沙箱里做不到**：每次 bash 调用都是独立的 bwrap
  （问题 #33），app 把 Victauri 的发现目录写在它**自己那次调用的私有 `/tmp`** 里，
  外面的 bridge 看不到它。于是改成**两半各自走真**：下载那一半用上面那条一次性探针
  （真上游），app 接线那一半用 `just test-e2e` 里的 `bitwarden_login`（真实 app、真实命令、
  假 CLI）。两条**不是同一次运行**，这一点如实记在 `docs/STATUS.md` 的待验证里。
- **探针里那次"直接执行不设 `BITWARDENCLI_APPDATA_DIR`"输出为空**：这个沙箱的家目录只读，
  `bw` 建不出它自己的 `data.json`。`managed` 那一轴因此不只是"便携性"，也是
  "家目录不可写时唯一能用的一档"。

### 留下的

- `config.json` 的写路径没有"写一次再重启"的用例（单测覆盖了坏文件与只换那一段；
  E2E 每次运行都会重写它并被 app 启动时读回，但没有专门断言"重启之后仍是上次选的那一个"）。
- 界面里没有自签证书（`NODE_EXTRA_CA_CERTS`）那一栏 —— crate 的 `with_extra_ca` 已就位。
