# just 快速上手（给人看的）

> **30 秒版**：忘了有啥命令就 `just --list`；改完代码提交前跑 `just ready`。
>
> 本文件是「我想做什么 → 跑什么」的任务视角，**并且就是命令的权威清单**（见下面 §2）。
> `AGENTS.md` §11 只讲归属原则，不重复这张表。

## 1. 最常用的五条

| 我想… | 命令 | 说明 |
|---|---|---|
| 开始干活 | `just dev` | 起 app。前端改动走 Vite HMR；**Rust 改动自动重编译并重启**。起一次就别关了 |
| 只改 Rust 逻辑 | `just watch` | bacon 秒级 clippy，**完全不启动 app** —— 这是本项目最快的反馈循环 |
| 只改界面 | `just dev-web` | 浏览器里跑 Vite，不启动 app |
| 提交前 | `just ready` | 一条命令跑完全部门禁 |
| 忘了有啥 | `just --list` | 列出全部配方及其一句话说明 |

## 2. 全部命令（权威清单）

> **这是全部配方的权威清单。** 由 `just docs-check` 强制保证：**每个配方都必须出现在下面这张表里**
> （只认这张表，不认别处的顺带提及），且 `AGENTS.md` / `ROADMAP.md` / `docs/**/*.md`
> 里提到的每个命令都必须真实存在。`AGENTS.md` §11 只讲归属原则，不再重复这张表。

「归属」列的含义：**根** = 直接实现在根 `justfile`；**转发** = 实现在 `src-tauri/justfile`、
根只转发；**根组合** = 跨根与 crate 的组合，只能在根定义。

| 命令 | 作用 | 归属 |
|---|---|---|
| `just dev` | 起 app：前端 HMR + Rust 改动自动重编译并重启（workspace 就在 `src-tauri/` 内，默认监听已覆盖全部成员）。**起一次就别关** | 根 |
| `just dev-web` | 只跑前端，不启动 app（浏览器里迭代界面） | 根 |
| `just watch` | bacon 秒级反馈循环，不启动 app —— 改纯逻辑时最快 | 转发 |
| `just ready` | **提交前跑这一个**：全部门禁（安静聚合，失败才倾倒） | 根组合 |
| `just check` | 类型检查（含 tests / benches），**workspace 全成员** | 转发 |
| `just clippy` | clippy，警告即错误，**workspace 全成员** | 转发 |
| `just lint` | clippy + `ast-grep scan`（结构性护栏） | 根组合 |
| `just fmt` | rustfmt 格式化（`--all` = workspace 全成员） | 转发 |
| `just fmt-check` | 只检查格式，不改文件 | 转发 |
| `just test` | 单元测试（cargo-nextest），**workspace 全成员** | 转发 |
| `just test-e2e` | E2E，需要 app 正在运行（先 `just dev`） | 转发 |
| `just deny` | 依赖门禁：许可证 / 漏洞 / 来源（需联网） | 转发 |
| `just deny-offline` | 同上，跳过需要联网的 advisories | 转发 |
| `just gen-types` | Rust command/event → `src/ipc/bindings.ts`（待接入 tauri-specta） | 转发 |
| `just doctor` | 确认 Victauri 连的是本项目，而不是别的实例 | 根 |
| `just syscheck` | 检查系统库是否齐（缺 webkit2gtk 会提前报错） | 根 |
| `just tools` | 按 `mise.toml` 装齐全局 CLI 工具 | 根 |
| `just tools-ls` | 看工具版本与来源 | 根 |
| `just docs-check` | 文档纪律：① 命令未漂移 ② ROADMAP 没长细节（每条 ≤3 行、无代码块、无命令调用）③ plan 预算（≤200 行）+ 索引一致 + 骨架不许开工 | 根 |
| `just ci-check` | CI 双 forge（Gitea + GitHub）可移植性：无 `runner` 上下文 / 非 Linux 分支有 `github.server_url` 门 / 三平台齐全 / 无 `.gitea/workflows/` 抢占目录 | 根 |

## 3. 两个 justfile 是什么关系

```
justfile                 ← 你在这里敲命令（项目级 + 转发）
├── dev / dev-web / tools / syscheck / doctor
├── lint / ready / docs-check        ← 跨根与 crate 的组合
└── check / fmt / test / deny / ... → 转发 ──┐
                                            ↓
src-tauri/justfile       ← crate 级命令真正实现的地方
    check / clippy / fmt / fmt-check / watch
    test / test-e2e / deny / deny-offline / gen-types
```

**为什么要分两个**：just 用 **justfile 所在目录**作为配方的工作目录。
把 crate 级命令放在 `src-tauri/` 里，`cargo` 就天然找得到 manifest ——
不需要在每条命令后面跟 `--manifest-path`（少一类需要记住的纪律）。

副作用（好的那种）：`cd src-tauri && just check` 也能独立用。

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

**改 IPC（command / event）**
1. 先 `just dev`（app 得跑着，Victauri 才能连）
2. 改完跑 `just gen-types`，**提交生成的类型差异**
3. 用 Victauri 走一遍真实路径（见 `AGENTS.md` §7）

**加依赖**
```bash
cd src-tauri && cargo add <crate>      # Rust 依赖（没有根 Cargo.toml，所以要进目录）
pnpm add <pkg>                          # 前端依赖（在仓库根）
just deny-offline                       # 新依赖的许可证要过门禁
```

## 5. `just ready` 的输出约定

```
→ fmt-check     ✅ 1s
→ lint          ✅ 3s
→ test          ✅ 2s
→ deny-offline  ✅ 6s
→ docs-check    ✅ 0s
✅ just ready 全绿（5/5）
```

**成功时只有这些**。失败时会把**那一步**的完整输出倒出来（超过 80 行则首尾各 40 行，
完整内容落在 `.just-ready-fail.log`）。

为什么这么设计：`deny-offline` 对 Tauri 这种依赖树会打 **5000+ 行**「重复版本」警告，
而它是 warn 级、永远不会让门禁失败 —— 在成功的运行里那全是噪音，会把真正的错误淹掉。

想看细节就**单跑那一步**（`just deny-offline` / `just lint` / `just test`），输出是完整的。

## 6. 出问题了

| 症状 | 原因 / 处理 |
|---|---|
| `just: command not found` | 工具没装。`just tools`（走 `mise.toml`），或 `cargo install just` |
| 提示找不到 `Cargo.toml` | 你在根目录跑了 crate 级命令。用根转发（`just check`），或 `cd src-tauri` |
| 缺 webkit2gtk 之类的系统库 | `just syscheck` 会指出来。Arch 系：`sudo pacman -S webkit2gtk-4.1` |
| app 起不来，或 Victauri 连不上 | app 必须先跑（`just dev`）；再用 `just doctor` 确认连的是本项目 |
| `just ready` 有一步红了 | 看结尾提示的那一步，或 `.just-ready-fail.log` |
| 改了配方但 `just --list` 没显示 | 检查缩进（配方体必须是 tab 或统一缩进），以及是否写在了对的 justfile 里 |
| 改了 `src-tauri/crates/` 下的文件，app 却不重编译 | 先确认它**确实在 `src-tauri/` 里面**（tauri CLI 默认只监听 `src-tauri`）。成员若被放到它外面（例如仓库根的 `src-tauri/crates/`），必须另配监听范围，否则开发循环**静默失效** —— 见 `STATUS.md` 坑 #21 |
| Gitea 上 CI 不跑 / 一直排队 | 三个已知原因：存在 `.gitea/workflows/`（它只读第一个存在的目录）、用了 `runner` 上下文、非 Linux 分支没有 `github.server_url` 门。跑 `just ci-check` 一次全查 |

**受限环境里跑 app**（容器 / agent 沙箱 / 无写权限的家目录）：
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
> 能提权就直接提权，别为了绕开权限去付这份代价。

> 同一类问题还有 `just deny`：它要写 `~/.cargo/advisory-dbs`，受限环境下会报
> `failed to acquire advisory database lock ... failed to create parent directories`。
> 处理方式相同：**先提权**。

## 7. 想加一条新命令

1. **先判断归属**：碰 cargo / Rust → 写进 `src-tauri/justfile`；
   前端、环境、跨仓库的组合 → 写进根 `justfile`。
2. 根 justfile 里给 crate 级命令写**转发**，别复制命令体。
3. **在本文 §2 的表格里加/改一行** —— 那是唯一权威清单，`AGENTS.md` §11 不放表。
4. 跑 `just docs-check` 验证（它已纳入 `just ready` 和 CI，不同步会直接红）。

## 8. 和 CI 的关系

CI（`.github/workflows/ci.yml`）跑的就是这两条：

```bash
just ready
just docs-check
```

**所以本地绿 ≈ CI 绿** —— 门禁只有一处定义，不存在"本地过了 CI 挂"的两套标准。
另有一个独立的 E2E job 会在 xvfb 下真的把 app 跑起来。

## 9. 关于 `mise.toml`

全局 CLI 工具（just / bacon / cargo-nextest / sccache / cargo-deny）记在 `mise.toml`。
装了 mise 并激活 shell 后，**进入本目录会自动装齐缺失的工具**（`just tools` 是同一件事的手动版）。
不想让 mise 管、继续用 `~/.cargo/bin` 里那套，删掉 `mise.toml` 即可。
