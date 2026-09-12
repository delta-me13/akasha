# Plan 0203: 前端 xterm + WebGL 渲染

- **关联**：ROADMAP 阶段 2 ·「前端 xterm + WebGL 渲染，字节流不进 React state」
- **前置**：plan 0202（IPC 二进制通道已能把批次送进来）
- **状态**：已完成
- **影响面**：`package.json`（前端依赖）、`src/**`（终端组件与 `src/ipc/` 包装）

## 目标

用 `@xterm/xterm` + `@xterm/addon-webgl` 渲染（WebGL 不可用时回退 `addon-canvas`），
配套 `addon-fit` / `addon-search` / `addon-serialize` / `addon-unicode11`（`AGENTS.md` §4.1）。

**字节流不进 React state**：终端写入走 `ref` + 命令式 API，React 只管理壳层 UI 状态
（`AGENTS.md` §4.2）。

## 非目标

- **不**实现多标签 / 分屏（`Session` 的呈现方式尚未定型，见 `scope.md` §1.2）
- **不**做 DOM renderer 兜底（高吞吐下必卡，不作为默认；WebGL 不可用只退到 canvas）
- **不**做 OSC 52 / OSC 8 白名单之外的处理（安全项见 `AGENTS.md` §4.3，按需另开 plan）

## 前置检查

```bash
node -e 'const p=require("./package.json");console.log(Object.keys({...p.dependencies,...p.devDependencies}))'
npx tauri --version >/dev/null 2>&1; echo "tauri cli: $?"
```

## 步骤

1. `pnpm add @xterm/xterm @xterm/addon-webgl @xterm/addon-canvas @xterm/addon-fit @xterm/addon-search @xterm/addon-serialize @xterm/addon-unicode11`。
2. 终端组件：`useRef` 持有 `Terminal` 实例；`onData` → 后端写入；后端批次 → `term.write(bytes)`。
   **`term.write` 的调用点只有一处**（便于日后接背压），且**不在 React 渲染路径里**。
3. WebGL 初始化 + 能力探测：失败时 `loadAddon(canvas)`；两者都失败时**明确报错**，不静默退回 DOM。
4. `addon-fit` + `ResizeObserver`：尺寸变化 → `fit()` → 把新行列数回传后端（`resize` 是"尽力"语义）。
5. 验证"不进 state"：`grep -rn 'setState\|useState' src/` 的命中**不得出现在写入路径**上。

## 验收命令

```bash
just dev-web      # 浏览器里迭代（不启动 app）
#   期望：终端由 canvas 渲染 —— 打开 devtools 执行：
#   document.querySelectorAll('.xterm-screen canvas').length   // 期望 ≥1

just dev          # 再在真 app 里确认一次（Victauri 能看到真实 webview）
just doctor       # 期望识别到本项目

pnpm build        # 期望前端能构建（类型与依赖都齐）
```

Victauri 侧（app 运行中）：
- `dom_snapshot` 确认终端节点存在、无异常结构；
- 连续输出大流量后 `get_performance` 看 JS heap 是否线性增长（应回落到稳态）。

## 回滚

删除终端组件与依赖，回到 plan 0202 的"能收字节但没有渲染"状态。

## 实施记录

**代码**（`src/terminal/` 三个文件 + `src/ipc/mock.ts`）：

- `surface.ts`：xterm + 四个 addon；渲染器 WebGL → canvas → **明确报错**三档，
  选定结果写进 `host.dataset.renderer` 供验收读；`term.write` 是**全工程唯一调用点**；
  WebGL 丢上下文（`onContextLoss`）时 dispose 后装 canvas。
- `attach.ts`：命令式接线（会话 ↔ 渲染面）。`ResizeObserver` 按帧合并，
  会话刚开时 `force` 补发一次真实行列数（载体是按默认尺寸起的）；
  **StrictMode 双挂载**时把没人要的会话显式关掉（否则每挂载一次多留一个真 PTY）；
  挂载失败折成报错，**不让异常冒到 React** —— 那会卸载整棵树，用户看到的是白屏。
- `TerminalPane.tsx`：React 只管宿主 DOM + 壳层状态。`useState` 命中只有两处
  （连接状态、渲染器种类），**写入路径上没有任何 state**（`grep` 可复核）。
- `mock.ts`：`just dev-web` 的模拟后端（`mockIPC` + 假 shell + `big` 灌流），
  模拟的是**线上格式**（`__CHANNEL__:<id>` + `ArrayBuffer`），所以浏览器里走的解码
  路径与真后端一致。只在 `import.meta.env.DEV && 没有 __TAURI_INTERNALS__` 时动态
  import；生产包里 `模拟后端` / `mockIPC` / `__akashaTerminal` 命中数**均为 0**（实测）。

**踩到的坑**（已进 STATUS）：`addon-unicode11` 的 `term.unicode` 是 **proposed API**，
不开 `allowProposedApi` 就在 effect 里抛 —— 症状是**整个界面白屏**；而 Victauri
读不到 webview console，是靠临时往 `index.html` 塞 `window.onerror` 钩子才定位到的。

**实测**（`VICTAURI_E2E=1 cargo test --test terminal_render -- --test-threads=1`，真 app）：

| 项 | 结果 |
|---|---|
| 渲染器 | **webgl**（WebKitGTK/MESA 软件栈下仍是 WebGL2）；`canvas` 2 块；DOM 行容器 **0 个** |
| 按键来回 | `printf 'akasha-probe-%s\n' 42` → 屏幕出现 `akasha-probe-42`（**求值结果**，不是命令行回显） |
| 8 MB 灌流 | 135–141 批送达；排空哨兵出现即队列归零；**之后还能继续敲命令**（"暂停"而非"卡死"） |
| WebGL 丢上下文 | `WEBGL_lose_context` → 落到 **canvas**，屏幕内容**保留**（不是重建终端） |
| console | **零 error**；只有 xterm 自己的 2 条 `warn`（`task queue exceeded allotted deadline`，本机软件渲染下出现） |

**CSP**（`STATUS.md` 待办里点名属于本 plan）：`csp` 与 `devCsp` 都从 `null` 换成最小放行，
两者只差 `connect-src` 里的 `ws://localhost:1420`（Vite HMR）。把 `devCsp` 临时设成与
`csp` **完全相同**也真跑过一轮：渲染 / IPC / 灌流全部照常、零 console error ——
所以生产那条字符串**是被跑过的**，不是纸面推测。`style-src 'unsafe-inline'` 必需
（xterm 自己注入 `<style>`）。

**未覆盖（照实记）**：

- `just dev-web` 的模拟后端只做了 `pnpm build` + 代码审查，**没在真浏览器里点过**
  —— 本环境没有可用浏览器。
- plan 里写的"大流量后看 `get_performance` 的 JS heap 是否回落"**没有取数**：
  只验到"8 MB 灌完队列归零、界面仍可交互"，没拿 heap 数字。
