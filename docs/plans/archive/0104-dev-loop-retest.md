# Plan 0104: 迁移后复测开发循环

- **关联**：ROADMAP 阶段 1 ·「迁移后复测开发循环」
- **前置**：plan 0101（迁移完成）+ plan 0103（**必须先有 `crates/` 成员**，否则监听范围无从测）
- **状态**：已完成（2026-09-11）——**监听范围确实失效过，已修复**
- **影响面**：无（纯验证）。若监听范围失效，改动落在 `src-tauri/Cargo.toml` 或 `.taurignore`

## 目标

确认根 workspace 布局下，`just dev` 的重编译 / 重启 / **监听范围**与迁移前基线一致。

**最大的隐性风险是监听范围**：CLI 打印的监听路径原本是 `src-tauri`，
迁移后 `crates/*` 落在它之外。若 crate 改动不再触发重启，开发循环会**静默失效** ——
`cargo check` / `just ready` 全绿，但你改了代码看不到效果，而没有任何门禁会告诉你。

## 非目标

- 不改业务代码（本 plan 只复测，必要时只动 watcher 配置）
- 不重复 plan 0101 的验收（结构是否正确已在那里验过）

## 前置检查

```bash
pgrep -af 'tauri dev|target/debug/akasha' || echo "无遗留 just dev（预期）"
```

⚠️ 若有遗留进程**先停掉**：它的监听范围是旧布局，会让复测结果失真（坑 #15 ——
那种状态下"端口在听、HTTP 200，但 app 没在运行"极具迷惑性）。

## 步骤

1. 从仓库根起 `just dev`，记录：二进制落点、冷编译耗时、dev server 端口、CLI 打印的监听路径。
2. 按下表逐项对比迁移前基线。
3. **监听范围实测**：改 `crates/akasha-core/src/lib.rs` 里的一行注释，
   观察 CLI 是否重编译并重启 app。
4. 若失效：在 `src-tauri/Cargo.toml` / `.taurignore` 层面处理（把 `crates/` 纳入 watch 范围），
   **不要**接受"静默失效"。处置写进「实施记录」。
5. 顺带走一遍 IPC 与 Victauri：`just doctor` 确认连的是本项目；打一次已有 command。

## 基线对照表（迁移前实测，出处 `docs/STATUS.md`）

| 项 | 迁移前 | 迁移后应为 |
|---|---|---|
| 二进制落点 | `src-tauri/target/debug/akasha` | `<root>/target/debug/akasha` |
| 冷编译 / 增量 | 47.53s / 6.09s | 同量级（不应变慢） |
| dev server | Vite 1420、HTTP 200 | 同上 |
| **CLI 监听范围** | `Watching .../src-tauri for changes` | **改 `crates/` 下的文件仍触发重编译 + 重启** |
| IPC 端到端 | 已有 command 返回预期结果 | 同样返回 |
| Victauri | 连上、identifier 为 `fans.cyrene.akasha-terminal` | 同样连上 |

## 验收命令

```bash
just dev                       # 一个终端常驻；期望能起窗口
# 另开一个终端触发一次 crate 改动：
printf '\n// watch-probe\n' >> crates/akasha-core/src/lib.rs
#   期望：just dev 的 CLI 输出立刻出现重编译 + 重启
git checkout -- crates/akasha-core/src/lib.rs     # 已提交文件，checkout 安全
just doctor                    # 期望识别到本项目
```

> `git checkout` 只对**已提交**的文件安全；未提交的改动用它还原会静默丢失（坑 #12）。

## 回滚

纯验证，无改动。若为此改了 watcher 配置，`git revert <commit>`。

## 实施记录

### 结论先行：监听范围**确实失效了**，这不是假想风险

迁移后第一次 `just dev` 的 CLI 输出：

```
Info Watching /home/lycurgus/akasha/src-tauri for changes...
```

**只有 `src-tauri`。** 随后实测（改两次 `crates/` 下的文件、等 35 秒）：dev 输出**一行都没有** ——
不重编译、不重启。阳性对照（改 `src-tauri/src/lib.rs`）立刻有
`Info File src-tauri/src/lib.rs changed. Rebuilding application...`。
即"监听器是活的，只是没看 `crates/`"——正是本 plan 要抓的那种静默失效。

### 修复：写进 `tauri.conf.json`，不是写在配方里

tauri CLI 有这个开关：`--additional-watch-folders <paths>`。先试了命令行形式，踩到两个坑：

1. **路径基准是 `src-tauri/`，不是 cwd** —— 传 `crates` 得到
   `Warn Additional watch folder '/home/lycurgus/akasha/src-tauri/crates' not found, ignoring`。
   这是**警告后继续**，不读警告就会以为什么都没发生。
2. 配置 schema 里本来就有 `build.additionalWatchFolders`（`config.schema.json` 的 `BuildConfig`）。
   最终落在这里而不是 `justfile`：**这样直接跑 `pnpm tauri dev` 也有效** ——
   监听范围是项目的性质，不是某条命令的性质。

修复后 CLI 输出两行，且 `../crates` 被正确解析（无 "not found" 警告）：

```
Info Watching /home/lycurgus/akasha/src-tauri for changes...
Info Watching /home/lycurgus/akasha/crates for changes...
```

### 端到端复测（把"真的重编译了"证到底）

这里有一个**会骗过人的混淆变量**：`src-tauri` 目前还不依赖任何 `crates/*`，
所以"监听触发了重建"并不等于"改的东西被编译进去了"（cargo 无事可做，0.29s 就结束）。
为排除它，临时给 `src-tauri` 加上 `akasha-core` 依赖再测一次，然后还原：

```
Info File crates/akasha-core/src/lib.rs changed. Rebuilding application...
Compiling akasha-core v0.1.0 (/home/lycurgus/akasha/crates/akasha-core)
Compiling akasha v0.1.0 (/home/lycurgus/akasha/src-tauri)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.18s
```

### 与迁移前基线逐项对照（`docs/STATUS.md` 的预跑基线）

| 项 | 迁移前 | 迁移后实测 |
|---|---|---|
| 二进制落点 | `src-tauri/target/debug/akasha` | **`/home/lycurgus/akasha/target/debug/akasha`** ✅ |
| 增量重编译 | 6.09s | **6.09 / 6.18 / 6.26s**（同量级，未变慢）✅ |
| dev server | Vite 1420 | 1420 就绪（HTTP 在听）✅ |
| Rust 监听范围 | 仅 `src-tauri` | **`src-tauri` + `crates`**（修复后）✅ |
| `just doctor` | 连上、35 个工具 | **13/13 passed**，`Connected to Victauri server`，端口 7373 ✅ |
| IPC 端到端 | `greet` → `Hello, preflight! …` | **同一字符串，逐字相同** ✅ |

（首次编译 31.34s 不用于对照：那次复用了移动过来的构建缓存，与"冷编译 47.53s"不同口径。）

### 两处必须交代清楚的环境限制

- **宿主 MCP 连不到沙箱内运行的 app。** agent 的每次 bash 调用都在 bwrap 里跑
  （`--ro-bind / /`、`--tmpfs /tmp`、`--unshare-pid`），Victauri 的发现文件
  （`/tmp/victauri/<pid>/{port,token}`）落在**该沙箱私有的 /tmp**，宿主侧的 MCP bridge 看不见。
  因此上面的 `doctor` 与 IPC 检查是**在同一个沙箱内**完成的
  （`just dev` 后台起 → 同脚本里跑 `just doctor` → 沿 Victauri 的 REST 回退路径打 `greet`）。
  **这是环境结构，不是项目问题**；在无沙箱环境下 MCP 直接可用。
- `dconf-CRITICAL`（写 `/run/user/1000/dconf/user`）与 WebKit 缓存的
  `Failed to create hard link` 都是同一类只读文件系统告警，**app 仍然正常起窗口**。

### 留给阶段 2 的一条复验

监听范围已覆盖 `crates/`，但"改了 crates 的代码，app 行为真的跟着变"目前只能靠
**临时依赖**证明（真实的 `src-tauri → crates/*` 依赖要到 plan 0201 / 0202 才建立）。
接上真实依赖后应顺手再改一次 `crates/` 文件确认一遍 —— 已记入 `docs/STATUS.md` 的待办。

