# AGENTS.md — akasha 项目规范

> 本文件是**人机共享的唯一规范入口**。`CLAUDE.md` 只是指向本文件的指针 + Victauri 自动生成块。
> 优先级：用户当次指令 > 本文件 > 代码中的既有约定。
> 修改本文件等于修改项目宪法：**必须单独提交**，不要混在功能 PR 里。

---

## 文件在哪（先看这里）

| 想知道 | 看 |
|---|---|
| **规则**：什么能做什么不能做、命令入口 | 本文件 |
| **下一步做什么** | [`ROADMAP.md`](./ROADMAP.md) |
| **现在到哪了、有哪些坑** | [`docs/STATUS.md`](./docs/STATUS.md) |
| **某个决定为什么这样定** | [`docs/adr/`](./docs/adr/) |
| **某个工作项怎么做** | [`docs/plans/`](./docs/plans/) |
| 文档索引与骨架 | [`docs/README.md`](./docs/README.md) |

> 本文件**只放规则**。状态与待办不写在这里 —— 规则的时效是"几乎不变"，
> 状态的时效是"每次会话"，混在一起两边都会烂。

---

## 0. 项目定位与硬性原则

- **产品**：`akasha` — Tauri 2 桌面终端应用（identifier `fans.cyrene.akasha-terminal`）。
- **栈**：Rust 后端（PTY / 进程 / VT 状态）+ React 19 + Vite 8 前端（xterm 渲染）。
- **架构原则**：
  1. **Rust 侧是唯一真相源** —— IPC 签名从 Rust 生成到 TS，不反向手写。
  2. **`src-tauri` 是薄壳** —— 纯逻辑下沉到不依赖 Tauri 的 `crates/`，可脱离 app 测试。
  3. **终端输出与用户按键都是不可信输入** —— 解析层永远假设输入带恶意 escape 序列。

### 绝对禁止（违反即视为"未完成"，不接受"下次再改"）

1. 前端裸调 `invoke("字符串命令名")` —— 必须走 `src/ipc/` 生成层。
2. PTY 字节当 `String` 跨 IPC 传，或逐字节 / 逐行 emit。
3. 在 `src-tauri/src/` 里写业务逻辑（应下沉到 `crates/`）。
4. `unwrap()` / `expect()` 出现在 command 边界或长驻任务中。
5. 用固定 `sleep` 等待异步完成 —— 用 Victauri `wait_for`（见 §7）。
6. 手改 `src/ipc/bindings.ts`（生成物）。
7. 未经 §7 DoD 验证就宣布功能完成。

---

## 1. 开发循环（先读这一节，能省掉最多无效劳作）

### 热重载的真相

**Tauri 没有 Rust 热重载。** `tauri dev` 本身就是开发主控进程，它只做两件事：

| 改动 | 行为 | 代价 |
|---|---|---|
| 前端 (`src/**`) | Vite HMR | 毫秒级，**不重启 app** |
| Rust (`src-tauri/**`, `crates/**`) | 增量重编译 + **重启 app** | 秒～分钟级，**唯一路径** |

**Victauri 不提供热重载。** 它是在*已运行*进程内嵌的 MCP 服务，35 个工具全部是
检查/驱动（DOM、IPC、后端状态、数据库、窗口）。它的价值不是"不用重启"，而是
**"不重启也能验证"**：不用加临时调试 UI、不用重跑场景，就能 `eval_js`、
`invoke_command`、`dom_snapshot` 直接观察并驱动运行中的 app。

### 由此推出的规则

- **常驻一个 `just dev`，不要每次手动重跑。** Rust 保存后 CLI 自动重编译 + 重启；
  重启后 Victauri bridge 会**重新发现端口**，MCP 无需重连 —— 继续调用即可。
- **Rust 侧提速的正解是"下沉 + 独立循环"，而不是等重启**：
  - 纯逻辑（PTY 抽象、VT 解析、状态机、布局）放 `crates/`，零 Tauri 依赖；
  - `just watch`（bacon）提供秒级 `check`/`test`，**全程不启动 app**；
  - `src-tauri` 只留 IPC 编组，改它的频率越低，重编译成本越低。
- **前端迭代不启动 app**：`just dev-web` + Tauri `mockIPC`，浏览器里跑 Vite HMR。
- **`.taurignore`** 排除不该触发 Rust 重建的路径（日志、fixtures、临时产物）。
- 跨平台差异交给 CI 矩阵，本地不必反复跑全平台。

### ⚠️ 一类会被误判成 bug 的失败：写工作区之外被沙箱拒绝

在受限环境（agent 沙箱 / 容器 / 只读家目录）里，**需要写工作区之外**的命令会失败，
而且失败信息看起来像工具或代码故障。已知两例：

| 命令 | 症状 | 想写的路径 |
|---|---|---|
| `just dev` | dconf 警告后 panic：`Failed to setup app: 只读文件系统 (os error 30)`。**发生在 `cargo build` 成功之后**，极易被误判成"编译过了但跑不起来" | `$HOME/.local/share/...`、`/run/user/1000/dconf` |
| `just deny` | `failed to acquire advisory database lock: ... failed to create parent directories` | `~/.cargo/advisory-dbs` |

**这些是环境权限，不是项目 bug** —— 同一份代码在完整权限下实测正常启动、正常检查
（基线见 `docs/STATUS.md`）。

**遇到时的正确动作（按顺序）**：

1. 先判断是否属于这一类：错误里出现「只读文件系统 / 无法创建目录 / 无法获取 lock」，
   且目标路径在**工作区之外**。
2. 确认属于这一类后，**直接对同一条命令申请提权重试**。不要当成项目 bug 去翻代码，
   不要只报错一次就收工，也不要换一条命令绕开。
3. **只在提权不可用**（被拒绝 / 无人审批）时，才退到替代方案（例如 `just dev` 的
   XDG 重定向 —— 它有副作用，见 `docs/just.md` §6）。

> 只在这条命令**确实被拒之后**才申请提权，不要预先提权：无差别的提权请求会被拒绝，
> 也会让「权限被拒」这个信号本身失去意义。

---

## 2. 工具分工（何时用哪个）

| 场景 | 首选 | 说明 |
|---|---|---|
| 结构化找代码（"所有 `.unwrap()` 调用"） | `ast-grep` / `sg` | 语法树匹配，不误伤注释与字符串 |
| 符号级理解、跳转、重构 | **rust-analyzer MCP** | 类型/引用级，跨 crate |
| 纯文本、日志、配置查找 | `grep` / `glob` | 非代码场景 |
| 运行中 app 的 DOM / IPC / 后端状态 | **Victauri MCP** | Tauri 的事不要用 CDP/Playwright |
| 运行中 app 的验收与回归 | `victauri-test` + `VICTAURI_E2E=1` | 见 §7 |
| 约束执行（把"禁止"变成机器检查） | `ast-grep scan` + `.ast-grep/rules/` | 见 §6 |

> `ast-grep` 在本项目有**双重身份**：既是搜索工具，也是**约束执行器**。
> 只把它当搜索用是浪费它的价值。

---

## 3. Rust 侧约束

### 3.1 分层

```
crates/akasha-pty/   # PTY 抽象：trait + portable-pty 实现；无 Tauri 依赖，可 mock
crates/akasha-vt/    # VT 解析 / 屏幕状态 / 回滚缓冲；纯函数式，可快照测试
crates/akasha-core/  # (按需) 会话模型、配置、布局
src-tauri/src/       # IPC 薄壳：command + Channel + 事件 + 状态注入
```

- 依赖方向**单向**：`src-tauri` → `crates/*`，反向依赖视为架构违规。
- `crates/*` 不得 `use tauri::*`。这条用 ast-grep 规则强制（§6）。
- 范围扩大后还会引入更多 crate（ssh / serial / sftp / store / 托盘与隧道），
  能力与切分见 `docs/scope.md`；**依赖方向规则同上，对新 crate 一律适用**。
- **命名：后端类型名不得编码 UI 呈现方式。** 前端把 `Session` 渲染成标签页 / 面板 /
  分屏 / 独立窗口都行，后端只按语义命名。词汇表与理由见 `docs/scope.md` §1.2 ——
  要点：**`Session`** = 资源的归属单位（用户打开的一个工作单元）、
  **`Transport`** = 字节载体（PTY / SSH shell 通道 / 串口）、
  **`Connection`** = 一条 SSH 连接。不要用 `Tab` / `Pane` / `View`，
  也不要用 `Workspace`（本仓库已指 Cargo workspace）。
- 本节的切分以 **ADR-0001 为准**，其状态为**已接受**（2026-09-11）。
  决策二已裁定：`akasha-vt` **维持延后**，若确有必要则建于
  `crates/akasha-vt/`，**不在仓库根平铺**。见该 ADR §0 补记。

### 3.2 数据流与背压（终端应用的成败点）

- PTY 读出的字节流**必须走二进制通道**：`tauri::ipc::Channel<Vec<u8>>` 或 raw body。
  默认 JSON IPC 会把字节流序列化成数组/字符串，吞吐直接崩。
- **合批后再发**：read loop 按「≥16ms 或 ≥64KiB」聚合一次，禁止逐字节 / 逐行 emit。
- **绝不假设 UTF-8**：字节流可以被切在任意多字节序列中间。解码只能在 VT 层做，
  且必须容忍"半个字符"；跨 IPC 只传 `&[u8]`。
- 前端渲染必须进 xterm 的写入缓冲，**禁止绕开 xterm 直接改 DOM**。

### 3.3 进程生命周期

⚠️ **窗口关闭 ≠ 进程退出。** 默认行为是**收到系统托盘**（见 `docs/scope.md` §5），
因此"窗口关闭"**不是**回收时机 —— PTY 与 SSH 隧道必须存活，否则托盘就没有意义。

| 事件 | PTY / 隧道 | 何时回收 |
|---|---|---|
| 窗口关闭（默认：收托盘） | **必须存活** | 不回收 |
| 窗口关闭（配置为"直接退出"） | 回收 | 立即 |
| **真正退出**（托盘退出 / app 重载 / panic） | 回收 | **零残留** |

- 每个 PTY / SSH 子进程与隧道必须在上述回收路径上**显式 kill + wait 收尸**；
  drop 不能代替。**窗口关闭默认不在其中。**
- 验收方式：用 Victauri `introspect { action: "processes" }` 在**真正退出之后**确认零残留。
  在"收托盘"状态下**存在子进程是预期行为，不是泄漏** —— 不要把存活误报成 bug。
- 托盘行为**可配置**，所以回收逻辑必须**读配置**，不能硬编码"关窗即杀"。
- 默认不继承不必要的 fd / 环境变量；shell 启动参数集中管理，不散落。

### 3.4 错误、日志、unsafe

- 错误：`thiserror` 分域定义；command 一律返回 `Result<T, E>`，`E: Serialize`。
- 日志：`tracing` 结构化日志 + `tauri-plugin-log` 文件轮转。
  **禁止 `println!` / `eprintln!`**（ast-grep 强制）。
- 敏感内容（用户键入的终端输入）**默认不落盘**，debug 级需显式开关。
- `unsafe`：默认禁止；确有必要时须 `// SAFETY:` 注释 + 单测覆盖。

---

## 4. 前端侧约束

### 4.1 渲染选型（已定，勿反复推翻）

- `@xterm/xterm` + `@xterm/addon-webgl`（不可用时回退 `addon-canvas`）。
  **DOM renderer 在高吞吐下必卡**，不作为默认。
- 配套：`addon-fit`（尺寸）、`addon-search`（搜索）、`addon-serialize`（会话恢复）、
  `addon-unicode11`（宽字符）。

### 4.2 状态与性能

- **终端字节流不进 React state**。用 `ref` + 命令式 API；React 只管理壳层 UI 状态。
- 避免每次输出触发组件重渲染；订阅走 ref，不做 per-chunk `setState`。
- 长列表 / 大量 span 一律虚拟化。

### 4.3 安全

- **OSC 52（写剪贴板）、OSC 8（超链接）、设备查询等必须走白名单 / 二次确认**，
  默认拒绝。恶意序列可窃取剪贴板或触发外链。
- `tauri.conf.json` 的 `csp` **不得为 `null`**；按需最小化放行。
- `capabilities/*.json` 遵循最小权限：**每加一条 permission 都要说明理由**。
- 渲染到 DOM 的任何终端衍生文本都要转义，禁止 `dangerouslySetInnerHTML`。

---

## 5. 类型边界（IPC 单一真相源）

- 用 `tauri-specta`（+ `specta-typescript`）从 Rust command/event **生成**
  `src/ipc/bindings.ts`；该文件为生成物，**禁止手改**。
- 任何新增/修改 command 或 event 后**必须**跑 `just gen-types` 并提交产物差异。
- 前端只允许通过 `src/ipc/` 的包装函数调后端，不出现裸命令名字符串。
- CI 校验：重跑 `just gen-types` 后 `git diff --exit-code` 必须为空。

---

## 6. 结构护栏（ast-grep 从"搜索"升级为"约束执行"）

- 配置：根目录 `sgconfig.yml`，规则目录 `.ast-grep/rules/`。
- `just lint` 包含 `ast-grep scan`；本地与 CI 都跑（CI 见 `.github/workflows/ci.yml`）。
- **规则与它守护的代码同 PR 落地**：规则先红、代码补上后转绿。
- 计划的规则清单（按需逐条添加）：

| 规则 | 拦截 |
|---|---|
| `no-bare-invoke` | 前端 `invoke("...")` 裸调用 |
| `no-println` ✅ 已落地 | Rust `println!` / `eprintln!` |
| `no-unwrap-in-commands` | command / 长驻任务中的 `unwrap()` |
| `no-tauri-in-core-crates` | `crates/**` 里 `use tauri::` |
| `no-std-command-bypass` | 绕过 `akasha-pty` 直接用 `std::process::Command` |
| `no-string-pty-channel` | PTY 字节流走 `Channel<String>` 而非 `Channel<Vec<u8>>` |
| `no-ui-vocab-in-types` | `crates/**` 与 `src-tauri/src/**` 类型名中的 `Tab`/`Pane`/`Window`/`View`（见 §3.1 命名规则） |

> 现阶段这些规则尚**未全部创建** —— 每条规则应与它守护的代码一起落地，
> 否则只是噪音。新增规则时同步更新上表。

---

## 7. 测试与验收（Definition of Done）

### 三层测试

| 层 | 工具 | 范围 | 是否需要 app |
|---|---|---|---|
| 单元 / 属性 | `cargo-nextest`（+ `proptest` 按需） | `crates/*` 纯逻辑 | 否 |
| 快照 | `insta` | VT 解析输出、屏幕状态 | 否 |
| 性能基线 | `criterion` | 解析与写路径吞吐 | 否 |
| 集成 / E2E | `victauri-test` + `VICTAURI_E2E=1` | IPC 契约、前后端一致性 | **是** |

### DoD：一条命令 + 两件机器查不了的事

```bash
just ready   # fmt-check + lint(clippy -D warnings + ast-grep scan) + test + deny-offline + docs-check
```

`just ready` 就是**可执行的 DoD**。能在命令里表达的验收标准，不要写成散文 ——
散文规则不会被强制执行。它覆盖不了、必须另外确认的只有两件：

- [ ] **真实路径走通**：Victauri `invoke_command`（或 UI 交互）触发 → `wait_for` 等到
      **真正结束** → `verify_state` 确认前后端状态一致。**禁止用 sleep 代替**。
- [ ] **涉 PTY / 子进程**：`introspect { action: "processes" }` 在**真正退出之后**确认无残留。
      ⚠️ "收托盘"状态下**存在子进程是预期行为**（见 §3.3），不是泄漏。

改过 command/event 时额外一条：新 command 应在 `get_registry` 中可见，
`detect_ghost_commands` 无新增 `confirmed_ghosts`；`just gen-types` 后 `git diff` 为空
（接入 tauri-specta 后纳入 `ready`，见 ROADMAP 阶段 3）。
涉及终端输出解析时，补一个 `insta` 快照。

### Victauri 使用纪律

- 调用任何工具前先 `get_plugin_info` 确认 `app.identifier == fans.cyrene.akasha-terminal`。
- 异步后端命令是 fire-and-forget：`invoke_command` 后用 `wait_for`
  （`condition: "expression"` 轮询状态，或 `condition: "event"` 等事件）。
- `query_db` **只读**；需要造数据就通过 app 自己的 command（尊重业务不变量）。
- 需要观察的后端内部状态，**注册 `probe`** 用 `app_state` 读，
  而不是靠 grep 日志反推。

---

## 8. 文档体系（时效性各不相同，不要混）

| 文件 | 回答什么 | 时效 |
|---|---|---|
| `AGENTS.md`（本文件） | 规则 | 几乎不变；**只放规则**，状态/进度/细节一律在别处 |
| `ROADMAP.md` | 去哪 | 偶尔变，**只勾复选框** |
| `docs/scope.md` | **产品预备有什么**：能力清单、非目标、已识别风险、**命名约定** | 偶尔变（只增删能力条目） |
| `docs/portable.md` | **可搬迁性怎么落地**：要求、数据目录、验证方法 | 随实测变 |
| `docs/STATUS.md` | 现在在哪 | **每次会话覆盖写，不追加** |
| `docs/adr/NNNN-*.md` | 为什么这样定 | **不可变**，只追加"被 NNNN 取代" |
| `docs/plans/NNNN-*.md` | 这次怎么做 | 随实现更新，就地修改 |

- **不要把状态、进度、待办写进本文件** —— 那会让本文件每天都要改，
  而后人无法分辨哪条还是现行规则。
- **本文件已经偏长**（超过 300 行）。再要往里加东西时，先问："这是规则，还是参考资料？"
  参考资料（如命令的详细用法、排错步骤）应下沉到 `docs/` 并在本文件留一句指针。
- 终端领域选型（PTY 库、VT 解析器、渲染器、序列化协议）**必须**有 ADR：
  这类决定日后被反复推翻的成本最高。
- plan 与 ADR 各自独立编号，用 plan 头部的 `关联：ADR-XXXX` 建立关系。
- 索引、plan 骨架、为什么暂时不建 `specs/` —— 见 `docs/README.md`。

### 8.1 汇总类文档的纪律（防止实现细节泄漏）

**汇总类 = `ROADMAP.md` / `docs/STATUS.md` / `docs/scope.md`。**
它们只回答"**有什么 / 到哪了 / 去哪**"，**不回答"怎么做"**。

| 想写的 | 它其实是 | 该去哪 |
|---|---|---|
| 步骤、编号子步骤、代码块 | 怎么做 | `docs/plans/NNNN-*` |
| "因为…"、"否则会…"、"之所以" | 为什么 | ADR；或 `scope.md` 能力条目的理由列 |
| "用 `cargo tree` 可证"、具体 flag、测试内部结构 | 验证手段 | plan 的「验收命令」 |
| 踩过的坑、排错步骤 | 参考资料 | `STATUS.md` 的坑；或专门文档（如 `portable.md`） |

**`ROADMAP.md` 的硬预算**（由 `just docs-check` 强制）：

- 每个条目 **≤ 3 行**（1 行标题 + 至多 2 行续行）
- **不出现代码块**；**不把命令调用粘进来**（反引号里不得有 `--flag` 或 `{...}`）
- 条目里只有两样：**一句"做完的样子"** + **一个可选的 plan 指针**

> ⚠️ 「每个条目必须有可执行的验收标准」（`ROADMAP.md` 头部）**不等于**把命令抄进去。
> ROADMAP 的验收是**行为判据**（"搬走文件夹后数据还在"）；plan 的才是
> **可粘贴的命令 + 预期输出**。两处都写同一件事，必然漂移。
>
> **诊断法**：条目数没变而文件在变长 ⇒ 细节正在泄漏。
> 「每条 ≤ 3 行」的含义正是：**总量只应随能力条目数增长**，不随细节增长。

搬运表与更多例子见 [`docs/README.md`](./docs/README.md)。

---

## 9. 提交与版本控制

- 约定式提交：`feat|fix|refactor|perf|test|docs|chore|build(scope): 摘要`。
- 一个提交一件事；**规范文件、CI、格式化等大范围改动单独提交**。
- 提交前跑 `just ready`（fmt-check + lint + test + deny-offline + docs-check）—— 见 §7。
- 不要提交：`node_modules/`、`dist/`、`target/`、生成的 `gen/schemas`。
- **要提交**：`Cargo.lock` / `pnpm-lock.yaml`（这是应用不是库，锁文件必须进仓库）。
- 大文件（图标除外）不进 git。

---

## 10. 依赖与工具：清单放哪个文件

### 10.1 四类清单的归属

| 类别 | 文件 | 安装方式 |
|---|---|---|
| 编译进产物的 Rust 依赖 | `src-tauri/Cargo.toml` | `cargo add` |
| 前端依赖 | `package.json` | `pnpm add` |
| **全局 CLI 工具**（不进产物） | **`mise.toml`** | **`just tools`** |
| Rust 工具链本身（rustc/cargo/clippy/rustfmt） | 暂无固定 —— 跟随 rustup `stable` | `rustup` |
| 系统库（webkit2gtk 等） | 无（发行版包管理器） | `pacman -S`，**cargo/mise 都管不了** |

`mise.toml` 是"不属于 Cargo.toml 的工具"的唯一来源。里面只有 `just`、`sccache`
有 aqua 预编译配方，`bacon` / `cargo-nextest` / `cargo-deny` 必须显式写
`"cargo:xxx"` 后端，首次安装会从源码编译（较慢）。

> 注意：这些工具目前已通过 `cargo install` 装在 `~/.cargo/bin`。`just tools`
> 会用 mise 再装一份并让 shim 优先。若不想装两份，删掉 `mise.toml` 即可 ——
> 它退化为一份文档，不影响现有工具可用性。

> **当前装了什么、哪些门禁是绿的、还剩哪些待办 —— 见 [`docs/STATUS.md`](./docs/STATUS.md)。**
> 状态不写在本文件里（见 §8）。

---

## 11. 命令入口

**命令体只写一处**，按归属分两个文件：

- **项目级**（dev / ready / lint / 环境检查）→ 根 `justfile`
- **crate 级**（cargo / nextest / bacon / cargo-deny）→ `src-tauri/justfile`
  —— just 用 **justfile 所在目录**作为配方工作目录，所以那里 `cargo check` 天然找得到
  manifest，**不需要任何 `--manifest-path`**
- 根 `justfile` 对 crate 级命令**只做转发**，不复制命令体

**完整命令清单（全部 19 个配方 + 用途 + 典型工作流 + 排错）见
[`docs/just.md`](./docs/just.md) §2。** 新增或改名配方时必须同步那里 ——
`just docs-check` 强制要求：**每个配方都必须在 `docs/just.md` 里出现**，
且两份文档提到的命令都必须真实存在。该校验已纳入 `just ready` 与 CI。

> 设这个校验的原因很具体：agent 最容易犯的错就是照着一份**过期的规则**
> 去用一个已经不存在的旧命令，而这类错误在类型检查里看不出来。
