# AGENTS.md — akasha 项目规范

> 本文件是**人机共享的唯一规范入口**。`CLAUDE.md` 只是指向本文件的指针 + Victauri 自动生成块。
> 优先级：用户当次指令 > 本文件 > 代码中的既有约定。
> 修改本文件等于修改项目宪法：**必须单独提交**，不要混在功能 PR 里。

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
- ⚠️ **本节的切分（尤其 `akasha-vt` 是否必要）尚未定案**，见
  `docs/adr/0001-crate-split-and-pty-abstraction.md`。ADR 接受前按现状执行。

### 3.2 数据流与背压（终端应用的成败点）

- PTY 读出的字节流**必须走二进制通道**：`tauri::ipc::Channel<Vec<u8>>` 或 raw body。
  默认 JSON IPC 会把字节流序列化成数组/字符串，吞吐直接崩。
- **合批后再发**：read loop 按「≥16ms 或 ≥64KiB」聚合一次，禁止逐字节 / 逐行 emit。
- **绝不假设 UTF-8**：字节流可以被切在任意多字节序列中间。解码只能在 VT 层做，
  且必须容忍"半个字符"；跨 IPC 只传 `&[u8]`。
- 前端渲染必须进 xterm 的写入缓冲，**禁止绕开 xterm 直接改 DOM**。

### 3.3 进程生命周期

- 每个 PTY 子进程必须**显式 kill + wait 收尸**；drop 不能代替。
- 三条路径都要覆盖：**窗口关闭 / app 重载 / panic**。
- 验收方式：用 Victauri `introspect { action: "processes" }` 确认无残留子进程（§7）。
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
- `just lint` 包含 `ast-grep scan`；pre-commit 与 CI 都跑。
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

### DoD 检查表（功能宣布完成前逐项确认）

- [ ] `just fmt && just lint && just test` 全绿。
- [ ] `ast-grep scan` 无新增违规。
- [ ] 改过 command/event → 跑过 `just gen-types`，产物差异已提交。
- [ ] 真实路径走通：Victauri `invoke_command`（或 UI 交互）触发 → `wait_for` 等到
      **真正结束** → `verify_state` 确认前后端状态一致。**禁止用 sleep 代替**。
- [ ] 新 command 在 `get_registry` 中可见；`detect_ghost_commands` 无新增
      `confirmed_ghosts`。
- [ ] 涉及 PTY / 子进程 → `introspect { action: "processes" }` 确认关闭后无残留。
- [ ] 涉及终端输出解析 → 新增对应 `insta` 快照。

### Victauri 使用纪律

- 调用任何工具前先 `get_plugin_info` 确认 `app.identifier == fans.cyrene.akasha-terminal`。
- 异步后端命令是 fire-and-forget：`invoke_command` 后用 `wait_for`
  （`condition: "expression"` 轮询状态，或 `condition: "event"` 等事件）。
- `query_db` **只读**；需要造数据就通过 app 自己的 command（尊重业务不变量）。
- 需要观察的后端内部状态，**注册 `probe`** 用 `app_state` 读，
  而不是靠 grep 日志反推。

---

## 8. 文档与决策

- 架构决策写 `docs/adr/NNNN-<slug>.md`（背景 / 选项 / 决策 / 后果）。
- 终端领域选型（PTY 库、VT 解析器、渲染器、序列化协议）**必须**有 ADR：
  这类决定日后被反复推翻的成本最高。
- ADR 一经接受不修改，只追加"被 NNNN 取代"。

---

## 9. 提交与版本控制

- 约定式提交：`feat|fix|refactor|perf|test|docs|chore|build(scope): 摘要`。
- 一个提交一件事；**规范文件、CI、格式化等大范围改动单独提交**。
- pre-commit：`fmt --check` + `clippy -D warnings` + `ast-grep scan`（装了 `just` 后走 `just lint`）。
- 不要提交：`node_modules/`、`dist/`、`target/`、生成的 `gen/schemas`。
- **要提交**：`Cargo.lock` / `pnpm-lock.yaml`（这是应用不是库，锁文件必须进仓库）。
- 大文件（图标除外）不进 git。

---

## 10. 依赖状态与剩余待办

### 已就绪（已实测）

| 类别 | 已装 |
|---|---|
| Rust CLI | `just 1.58.0`、`bacon 3.25.0`、`cargo-nextest 0.9.144`、`sccache 0.17.0`、`cargo-deny 0.20.2` |
| 前端 | `@xterm/xterm 6` + webgl/canvas/fit/search/serialize/unicode11、`vitest 5`、`@biomejs/biome 2.5` |
| Rust 依赖 | `portable-pty 0.9`、`vte 0.15`、`thiserror 2`、`tracing`、`tauri-plugin-log`、`tauri-specta 2.0.0-rc.25` + `specta` + `specta-typescript`、`insta`、`criterion` |
| 缓存 | `.cargo/config.toml` 已接 sccache（`just check` 后 `sccache --show-stats` 验证） |

### 剩余待办

1. 🔴 **阻塞构建 —— 缺 WebKit2GTK 系统库。**
   本机是 **CachyOS（Arch 系，用 `pacman`）**，不是 Debian 系，**不要用 `apt-get`**：

   ```bash
   sudo pacman -S webkit2gtk-4.1
   ```

   现状证据：`just check` 退出码 101，`javascriptcore-rs-sys` 的构建脚本报
   `Package 'javascriptcoregtk-4.1' was not found`。验收标准 = `just check` 退出码 0。
2. 🔴 **`.github/workflows/victauri.yml` 当前是坏的**：它在仓库根跑 `cargo build`
   与 `cargo metadata`，但根目录没有 `Cargo.toml`。ADR-0001 若采纳根工作区会自动修好；
   否则必须显式改成 `--manifest-path src-tauri/Cargo.toml` 并写死 bin 名。
3. `cargo deny init` 同样因为根目录无 `Cargo.toml` 而失败（退出码 1），
   `deny.toml` 未生成 → `just deny` 暂时无意义。同样依赖 ADR-0001 的结论。
4. 工作区切分与 PTY 抽象待 `docs/adr/0001` 定案。定案后需同步：§3.1 的分层图、
   `justfile` 的 `MANIFEST`、根 `.gitignore`（`target/` 位置）、CI。
5. `tauri-specta` 目前是 `2.0.0-rc.25`（预发布）。接入时决定：锁 rc 还是等正式版。
