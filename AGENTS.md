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
  2. **`src-tauri` 是薄壳** —— 纯逻辑下沉到不依赖 Tauri 的 `src-tauri/crates/`，可脱离 app 测试。
  3. **终端输出与用户按键都是不可信输入** —— 解析层永远假设输入带恶意 escape 序列。

### 绝对禁止（违反即视为"未完成"，不接受"下次再改"）

1. 前端裸调 `invoke("字符串命令名")` —— 必须走 `src/ipc/` 生成层。
2. PTY 字节当 `String` 跨 IPC 传，或逐字节 / 逐行 emit。
3. 在 `src-tauri/src/` 里写业务逻辑（应下沉到 `src-tauri/crates/`）。
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
| Rust (`src-tauri/**`，含 `crates/`) | 增量重编译 + **重启 app** | 秒～分钟级，**唯一路径** |

**Victauri 不提供热重载。** 它是在*已运行*进程内嵌的 MCP 服务，35 个工具全部是
检查/驱动（DOM、IPC、后端状态、数据库、窗口）。它的价值不是"不用重启"，而是
**"不重启也能验证"**：不用加临时调试 UI、不用重跑场景，就能 `eval_js`、
`invoke_command`、`dom_snapshot` 直接观察并驱动运行中的 app。

### 由此推出的规则

- **常驻一个 `just dev`，不要每次手动重跑。** Rust 保存后 CLI 自动重编译 + 重启；
  重启后 Victauri bridge 会**重新发现端口**，MCP 无需重连 —— 继续调用即可。
- **Rust 侧提速的正解是"下沉 + 独立循环"，而不是等重启**：
  - 纯逻辑（PTY 抽象、VT 解析、状态机、布局）放 `src-tauri/crates/`，零 Tauri 依赖；
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
src-tauri/crates/akasha-pty/   # PTY 抽象：trait + portable-pty 实现；无 Tauri 依赖，可 mock
src-tauri/crates/akasha-vt/    # VT 解析 / 屏幕状态 / 回滚缓冲；纯函数式，可快照测试
src-tauri/crates/akasha-core/  # (按需) 会话模型、配置、布局
src-tauri/src/       # IPC 薄壳：command + Channel + 事件 + 状态注入
```

- 依赖方向**单向**：`src-tauri` → `src-tauri/crates/*`，反向依赖视为架构违规。
- `src-tauri/crates/*` 不得 `use tauri::*`。这条用 ast-grep 规则强制（§6）。
- 范围扩大后还会引入更多 crate（ssh / serial / sftp / store / 托盘与隧道），
  能力与切分见 `docs/scope.md`；**依赖方向规则同上，对新 crate 一律适用**。
- **命名：后端类型名不得编码 UI 呈现方式。** 前端把 `Session` 渲染成标签页 / 面板 /
  分屏 / 独立窗口都行，后端只按语义命名。词汇表与理由见 `docs/scope.md` §1.2 ——
  要点：**`Session`** = 资源的归属单位（用户打开的一个工作单元）、
  **`Transport`** = 字节载体（PTY / SSH shell 通道 / 串口）、
  **`Connection`** = 一条 SSH 连接。不要用 `Tab` / `Pane` / `View`，
  也不要用 `Workspace`（本仓库已指 Cargo workspace）。
- 本节的**分层与命名**以 **ADR-0001 为准**（已接受）；**workspace 的物理位置**
  （root 在 `src-tauri/`、成员在其 `crates/` 下、仓库根不放 Rust 成员）以
  [`docs/adr/0004`](./docs/adr/0004-rust-workspace-under-src-tauri.md) 为准 ——
  它取代了 ADR-0001 的决策一。
- ADR-0001 决策二仍有效：`akasha-vt` **维持延后**，若确有必要则建于
  `src-tauri/crates/akasha-vt/`，**不在仓库根平铺**。

### 3.2 数据流与背压（终端应用的成败点）

- PTY 读出的字节流**必须走 raw 通道**：`Channel<InvokeResponseBody>` +
  `InvokeResponseBody::Raw(bytes)`（JS 侧收到 `ArrayBuffer`）。
  ⚠️ **`Channel<Vec<u8>>` 不是二进制通道** —— tauri 有
  `impl<T: Serialize> IpcResponse for T` 这条 blanket impl，所以它发出去的是
  "六万多个数字的 JSON 数组"，与默认 JSON IPC 是同一条慢路。
  由 `.ast-grep/rules/no-string-pty-channel.yml` 强制（§6）。
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
| **真正退出**（托盘退出 / app 重载 / panic / `kill -9`） | 回收 | **零残留** |

- 每个 PTY / SSH 子进程与隧道必须在上述回收路径上**显式 kill + wait 收尸**；
  drop 不能代替。**窗口关闭默认不在其中。**
- 有一条路径**进程里没有任何代码会执行**（`tauri dev` 重载的 SIGKILL、`kill -9`），
  所以还有**进程外**的一道兜底：伴生看门狗读一条管道，EOF 就收掉登记过的会话。
  决定与边界见 [ADR-0005](./docs/adr/0005-sigkill-exit-watchdog.md)。
- **新增载体必须回答 `Transport::session_leader()`**（没有本地进程就 `None`）——
  看门狗靠它认会话。答错/漏答的表现是"这条路径上收不掉"，而其它三条路径照样绿。
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
- 任何新增/修改 command 或 event 后**必须**跑 `just gen-types` 并提交产物差异 ——
  **`just ready` 里的 `gen-types-check` 会比对生成物是否已提交**（没提交就红）。
- 前端只允许通过 `src/ipc/` 的包装函数调后端，不出现裸命令名字符串。
- ⚠️ **一处刻意的手写**：raw 字节通道那条命令的参数在生成物里只能是 `string`
  （`InvokeResponseBody` 没有 `specta::Type`，生成器写不出它的 TS 类型），
  所以"怎么建频道、怎么把 `ArrayBuffer` 变成字节"留在 `src/ipc/session.ts`。
  手写的**只有这一段**，签名仍来自生成物 —— 别把它当成"可以手写第二份签名"的先例。

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
| `no-tauri-in-core-crates` ✅ 已落地 | `src-tauri/crates/**` 里 `use tauri::` |
| `no-std-command-bypass` | 绕过 `akasha-pty` 直接用 `std::process::Command` |
| `no-string-pty-channel` ✅ 已落地 | PTY 字节流走 `Channel<Vec<u8>>` / `Channel<String>`（其实是 JSON 数组）而不是 raw 通道（§3.2） |
| `no-ui-vocab-in-types` ✅ 已落地 | `src-tauri/crates/**` 与 `src-tauri/src/**` 类型名中的 `Tab`/`Pane`/`Window`/`View`（见 §3.1 命名规则） |

> 现阶段这些规则尚**未全部创建** —— 每条规则应与它守护的代码一起落地，
> 否则只是噪音。新增规则时同步更新上表。
>
> ⚠️ **规则必须用负例验证过**才算落地：只跑一次"全绿"分不清"规则在工作"与
> "规则写错了、什么都没匹配到"。负例要一对 —— 一个应当命中、一个诱饵应当**不**命中
> （例如 `no-ui-vocab-in-types` 命中 `NegTabProbe` 但不命中 `Previewer`）。
> 验证后删掉探针文件，别留在仓库里。
>
> ⚠️ **改了规则的 `files:` / `ignores:` 之后要重跑一次负例**（目录搬家、crate 改名都算）：
> 路径写错的表现是"不匹配任何文件"，即**静默失效** —— `ast-grep scan` 照样退出码 0。

---

## 7. 测试与验收（Definition of Done）

### 四层测试

| 层 | 工具 | 范围 | 是否需要 app |
|---|---|---|---|
| 单元 / 属性 | `cargo-nextest`（+ `proptest` 按需） | `src-tauri/crates/*` 纯逻辑 | 否 |
| 快照 | `insta` | VT 解析输出、屏幕状态 | 否 |
| 性能基线 | `criterion` | 解析与写路径吞吐 | 否 |
| 集成 / E2E | `victauri-test` + `VICTAURI_E2E=1` | IPC 契约、前后端一致性 | **是** |

> **E2E 的入口是 `just test-e2e`** —— 自包含：已有 app（`just dev`）就复用，没有就自己起
> Vite + app，跑完收掉。不要手写 `VICTAURI_E2E=1 cargo test …` 那一串：目标清单、串行、
> 平台能力跳过与收尾都在配方里。**新增 E2E 目标必须加进配方的 `E2E_TARGETS`** ——
> 漏了它会直接红（那正是"生在门禁外、于是没人跑"的教训）。
> 平台跑不了的用例要**显式跳过并写明原因**（例如 Wayland 下拿不到原生窗口句柄），
> 多平台覆盖面交给 CI 矩阵。

> **性能基线不是门禁**：MB/s 随机器、编译器版本与是否插电而变，拿它当通过条件
> 只会得到一条随机红、且很快没人信的红线。基线**数字**记进 plan 与 `docs/STATUS.md`，
> 用途是改动前后对比（入口 `just bench`）。

### DoD：一条命令 + 两件机器查不了的事

```bash
just ready   # fmt-check + lint(clippy + ast-grep scan) + test + deny-offline
             # + gen-types-check + docs-check
```

`just ready` 就是**可执行的 DoD**。能在命令里表达的验收标准，不要写成散文 ——
散文规则不会被强制执行。它覆盖不了、必须另外确认的只有两件：

- [ ] **真实路径走通**：Victauri `invoke_command`（或 UI 交互）触发 → `wait_for` 等到
      **真正结束** → `verify_state` 确认前后端状态一致。**禁止用 sleep 代替**。
- [ ] **涉 PTY / 子进程**：`introspect { action: "processes" }` 在**真正退出之后**确认无残留。
      ⚠️ "收托盘"状态下**存在子进程是预期行为**（见 §3.3），不是泄漏。

改过 command/event 时额外一条：新 command 应在 `get_registry` 中可见，
`detect_ghost_commands` 无新增 `confirmed_ghosts`；生成物必须已提交
（`gen-types-check` 已纳入 `ready`，见 §5）。
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
| `docs/bitwarden.md` | **Bitwarden 集成的展开**：许可证、条目字段、指纹语义 | 随上游版本与实测变 |
| `docs/STATUS.md` | 现在在哪 | **每次会话覆盖写，不追加** |
| `docs/adr/NNNN-*.md` | 为什么这样定 | **不可变**，只追加"被 NNNN 取代" |
| `docs/plans/TTxx-*.md` | 这次怎么做（一个工作项一个文件） | 进行中就地修改；**完成后整份移入 `docs/plans/archive/`** |

- **不要把状态、进度、待办写进本文件** —— 那会让本文件每天都要改，
  而后人无法分辨哪条还是现行规则。
- **本文件已经偏长**（超过 300 行）。再要往里加东西时，先问："这是规则，还是参考资料？"
  参考资料（如命令的详细用法、排错步骤）应下沉到 `docs/` 并在本文件留一句指针。
- 终端领域选型（PTY 库、VT 解析器、渲染器、序列化协议）**必须**有 ADR：
  这类决定日后被反复推翻的成本最高。
- plan 与 ADR 各自独立编号，用 plan 头部的 `关联：ADR-XXXX` 建立关系。
- **plan 的生命周期有硬预算**（详见 [`docs/plans/README.md`](./docs/plans/README.md)）：
  一个工作项一个文件，**单文件 ≤ 200 行**，超了要**拆成两份**；
  完成后**整份移入 `docs/plans/archive/`**，索引保留一行 ——
  **永不把多份 plan 拼接成一份汇总**（那正是"一个文件太大"的来源）；
  没有「验收命令」的骨架 plan **不许开工**。以上由 `just docs-check` 强制。
- 索引、plan 骨架、为什么暂时不建 `specs/` —— 见 `docs/README.md` 与 `docs/plans/README.md`。

### 8.1 汇总类文档的纪律（防止实现细节泄漏）

**汇总类 = `ROADMAP.md` / `docs/STATUS.md` / `docs/scope.md`。**
它们只回答"**有什么 / 到哪了 / 去哪**"，**不回答"怎么做"**。

| 想写的 | 它其实是 | 该去哪 |
|---|---|---|
| 步骤、编号子步骤、代码块 | 怎么做 | `docs/plans/TTxx-*` |
| "因为…"、"否则会…"、"之所以" | 为什么 | ADR；或 `scope.md` 能力条目的理由列 |
| "用 `cargo tree` 可证"、具体 flag、测试内部结构 | 验证手段 | plan 的「验收命令」 |
| 踩过的坑、排错步骤 | 参考资料 | `STATUS.md` 的坑；或专门文档（如 `portable.md`） |
| `scope.md` 里**某一节越写越长**（超过约一屏） | 参考资料 | 拆成专门文档（`portable.md` / `bitwarden.md` 就是这么来的），原处只留结论 + 指针 |

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
- 提交前跑 `just ready`（fmt-check + lint + test + deny-offline + gen-types-check + docs-check）—— 见 §7。
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
- **非临时脚本一律做成 just 配方**，不要在仓库里散落 `.sh`：配方是唯一被 `docs-check`
  强制登记的入口（每个配方都必须出现在 `docs/just.md` §2，且 `just --list` 里可见），
  散落的脚本没有任何门禁照看。例外只有在 CI 里跑一次的安装步骤（它们不服务于本地工作流）。
- ⚠️ crate 级配方**必须显式带 `--workspace`**：cargo 在成员目录里**只选当前包**，
  漏了会让 `crates/*` 的 check / clippy / test **完全不被执行**，而 `just ready` 照样全绿
  （坑 #20）。`cargo fmt --all` 是例外（`--all` 本来就指全 workspace）。

**完整命令清单（全部 21 个配方 + 用途 + 典型工作流 + 排错）见
[`docs/just.md`](./docs/just.md) §2。** 新增或改名配方时必须同步那里 ——
`just docs-check` 强制要求：**每个配方都必须在 `docs/just.md` 里出现**，
且两份文档提到的命令都必须真实存在。该校验已纳入 `just ready` 与 CI。

> 设这个校验的原因很具体：agent 最容易犯的错就是照着一份**过期的规则**
> 去用一个已经不存在的旧命令，而这类错误在类型检查里看不出来。

---

## 12. CI：只维护 GitHub Actions 一份

`.github/workflows/ci.yml` 是**唯一**的工作流文件（三个 job：Linux 完整门禁 /
Windows + macOS 类型检查 / **三平台 E2E 矩阵**）。**不要为别的 forge 加兼容层** ——
曾做过"一份工作流同时喂 Gitea 与 GitHub"，代价是整份工作流被压在两边**共有的子集**里；
2026-09-11 评估后放弃，那份约束清单与放弃理由见
[`docs/plans/0102`](./docs/plans/0102-ci-platform-matrix.md)。

- **门禁只有一处定义**：CI 里跑的必须**就是**本地那一条 `just ready`，不要在 workflow 里
  另写 cargo 命令 —— 两处必然分叉，而分叉的方向总是"CI 比本地松"。
- **完整门禁只在 Linux 跑一次**（fmt / clippy / docs-check 的结论与平台无关）：
  矩阵跑三遍只是把时间乘三。**类型检查**在 Windows / macOS 再跑一份，用来挡 cfg 分支错误；
  **E2E 反过来要三平台都跑** —— 平台差异（原生窗口句柄、进程判活与收尾）正是它的对象。
  **出包不在 CI 的目标内**（需要真实主机：WiX / NSIS / WebView2 bootstrapper 都不行）。
- **E2E 只 `needs` Linux 那条 job**：平台类型检查与 E2E 是**互相独立**的信号，
  串成一条链只会让"Windows 红了"顺带吃掉 E2E 的结论。
- **系统依赖列表只有 `env.APT_DEPS` 一处**（Tauri 官方列表；注意是
  `libayatana-appindicator3-dev`，不是已消失的旧名 `libappindicator3-dev`）。
- **放手用 GitHub 专属能力，并且优先选省钱的**：`concurrency` + `cancel-in-progress`
  取消同一分支上被取代的运行（`main` 除外 —— 合并后的结论不该被掐断）、
  `permissions: contents: read`、`defaults.run.shell`、`${{ runner.* }}` 上下文与任意
  表达式函数。工具安装统一走 `taiki-e/install-action`（预编译产物 + SHA256/attestation 校验），
  **不要再手写"按平台选资产 + curl + 追加 `GITHUB_PATH`"的脚本** —— 那是兼容层的遗产，
  它的存在理由（"不能用 `${{ runner.arch }}`"）已经消失。

> 改了 workflow 先在本机跑 `just ready` —— 但它只证明"命令链是通的"：
> **CI 的真实行为以 runner 上的实跑为准**（状态见 `docs/STATUS.md`）。
