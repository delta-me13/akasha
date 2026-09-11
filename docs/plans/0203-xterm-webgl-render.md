# Plan 0203: 前端 xterm + WebGL 渲染

- **关联**：ROADMAP 阶段 2 ·「前端 xterm + WebGL 渲染，字节流不进 React state」
- **前置**：plan 0202（IPC 二进制通道已能把批次送进来）
- **状态**：未开始
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

（边做边追加：记录 canvas 数量、探测到的 renderer 类型、大流量下的 heap 表现。）
