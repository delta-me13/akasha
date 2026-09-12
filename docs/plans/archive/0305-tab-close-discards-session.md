# Plan 0305: 关闭终端标签页 = 立刻丢弃该 Session

- **关联**：ROADMAP 阶段 3 ·「关闭终端标签页 = 立刻丢弃该 Session」
- **前置**：plan 0204（显式 kill + wait 收尸）、0205（进程外兜底）—— 关闭路径的机制已经成立
- **状态**：已完成
- **影响面**：`src/**`（标签栏与多标签宿主）、`src-tauri/tests/**`、`src-tauri/justfile`（E2E 清单）、
  `docs/scope.md` §5、`AGENTS.md` §3.3

## 目标

1. 前端第一次出现**标签页**这种呈现（`App.tsx` 原先刻意不做，因为呈现方式未定型）。
   每个终端标签页对应**一个**后端 `Session`（`SessionKind::Terminal`）。
2. **关闭一个终端标签页 = 立刻丢弃它的 `Session`**：不弹确认、不留宽限期。
   **只丢它自己** —— 别的标签页的会话与进程不受影响。
3. 关掉**最后一个**标签页是**空状态**（界面空了、进程还在），**不是退出应用** ——
   「关标签页 ≠ 关窗口 ≠ 退出应用」（托盘与退出是阶段 3 另一条线的事）。
4. 把"哪一类标签页可关闭"写成规范：
   - **三大终端**（local / ssh / serial）的标签页：**有关闭按钮**，关掉 = 立刻丢弃该 `Session`；
   - **仅渲染的前端标签页**（SSH 转发 / 密码库 / 文件传输）：**没有关闭按钮**，前端只是客户端视图，
     **关前端不影响后端执行**（`docs/scope.md` §5.6）。

## 非目标

- 不做标签页重命名 / 拖拽排序 / 分屏 / OSC 标题同步（标题暂时就是序号）。
- **不给**转发 / 密码库 / 文件传输做标签页（阶段 4/6/7）：本步只把"它们没有关闭按钮"这条**规则**
  写进规范，并留一个 `kind` 位置等它们来。
- **不动**托盘与窗口行为（plan 0301–0303）：**关窗口 ≠ 关标签页**（关窗口默认收托盘，会话必须活着）。
- **不加**"只有 Terminal 能被关"这种后端限制：`close_session` 对任何 `Session` 都成立
  （阶段 6 的 plan 0606 正要用它关转发 `Session`）。本步约束的是**前端的按钮**，不是后端能力。

## 前置检查

```bash
grep -rn "close_session" src-tauri/src src/ipc | grep -v bindings   # 关闭路径已存在（0204）
grep -n "E2E_TARGETS" src-tauri/justfile                            # 新目标要加进清单
just test-e2e                                                        # 改之前的基线
```

## 步骤

1. **标签模型**（`src/App.tsx`）：`OpenTab { key, kind }`，`kind = "terminal"`（当前唯一变体，
   位置留给"仅渲染"的视图标签页）。`active` 只是一个 `key`，空列表 = 空状态。
2. **标签栏**（`src/tabs/TabStrip.tsx`）：新建（`+`）/ 切换 / 关闭（`×`）。
   `×` **只在 `kind === "terminal"` 时渲染** —— 这就是上面第 3 条规则在代码里的落点。
3. **多标签宿主**：所有标签页的 `TerminalPane` **保持挂载**，非活动的用 `visibility: hidden`
   叠在同一格里。
   - **不能 `display: none`**：FitAddon 量不到尺寸，切回去会一直停在旧行列数；
   - **更不能卸载**：卸载就是关会话（`attachTerminal` 的清理函数是唯一的关闭调用点）。
   - 于是"关标签页"**不需要第二条关闭路径**：从列表里移除 → React 卸载 → `close_session`。
4. **前端不等回收完成**：`void close()`。"立刻" = **没有宽限期**，不是"UI 阻塞到子进程收完"；
   真正的 wait 收尸在后端的 `Transport::shutdown()` 里（0204）。
5. **焦点交接**：切到某个标签页时把键盘焦点交给 xterm 的输入框（否则用户点完标签页按键没反应）。
   只碰焦点，不碰终端内容（§4.2 禁止的是绕开 xterm 改 DOM 文本）。
6. **E2E**（`src-tauri/tests/tab_close.rs`，接入 `E2E_TARGETS`）：真 app、真点击，判据见下。
7. **既有用例的一处收紧**：`terminal_render.rs` 取 textarea 从"第一个"改成"活动标签页里的那个" ——
   多标签之后"第一个"不再唯一，而探针 `__akashaTerminal` 指向**最后挂载**的那个面，两者必须指同一个终端。
8. **规范**：`docs/scope.md` §5.6（标签页分类）、`AGENTS.md` §3.3（事件表加一行 + 一条反直觉提醒）。

## 验收命令

```bash
just ready        # 6/6（fmt-check / lint / test / deny-offline / gen-types-check / docs-check）
just test-e2e     # 自包含：起 Vite + app → 逐个目标 → 收尾
```

`tab_close` 用例必须打印并满足（真 UI 点击，不是直接调命令）：

```
标签页数：1 → + → 2 → 关闭第 1 个 → 1
探针 A(…) 已随标签页消失（… ms）      ← 关闭 = 立刻丢弃，且不需要第二次点击
探针 B(…) 仍在                        ← 只丢它自己
✅ 关闭终端标签页 = 立刻丢弃该 Session，且只丢它自己
```

手工复核（可选，`just dev`）：

```bash
just dev
# 1) 开一个终端，跑 `seq 1 200`；2) 点 + 开第二个；3) 切回第一个 —— 回滚内容必须还在
# 4) 点第一个标签页的 ×；5) pgrep -af 'sleep 600' —— 第一个会话里的东西必须不见
```

## 回滚

删掉 `src/tabs/`，`App.tsx` 回到单终端（`<TerminalPane />` 一个），E2E 目标从 `E2E_TARGETS` 移除。
后端**没有改动**，所以回滚不涉及 Rust 侧。

## 实施记录

| 命令 | 结果 |
|---|---|
| `just ready` | **6/6 全绿** |
| `just test` | **66 tests run: 66 passed**（Rust 侧本轮**一行没改**） |
| `just test-e2e` | 退出码 **0**，**10 个用例全绿**（新增 `tab_close` 1 条） |
| `pnpm build`（tsc + vite build） | 退出码 0；产物 841 kB / gzip 230 kB；生产包里 `akashaTerminal` / `activateProbe` / `mockIPC` 命中数 **0**（探针与模拟后端仍被整段摇掉） |

真 app 上的输出（两个标签页各起一个忽略 SIGHUP 的探针；单独跑与在 `test-e2e` 里跑各验一遍）：

```
已把标签页 1 的 WebGL 上下文丢掉（退到 canvas）      ← 见下面的 1.
标签页 1：探针 A = 1279；标签页 2：探针 B = 1377
标签页数：1 → + → 2 → 切换（当前「终端 1」）→ 两个探针都还在
探针 A(1279) 已随标签页消失（83 ms）                 ← 关闭 = 立刻丢弃，不需要第二次点击
探针 B(1377) 仍在，且它的屏幕内容还在                ← 只丢它自己
关掉最后一个标签页：空状态（app 仍在），探针 B(1377) 也随会话被丢弃
✅ 关闭终端标签页 = 立刻丢弃该 Session，且只丢它自己（关掉最后一个也不退出应用）
```

**实现**：`src/tabs/TabStrip.tsx`（新；`×` 按 `kind` 渲染）、`src/App.tsx`（标签模型 + 多面宿主）、
`src/terminal/TerminalPane.tsx`（活动面交焦点 / 交探针）、`src/terminal/surface.ts`（`activateProbe`
+ 拆面兜底）、`src/terminal/attach.ts`（清理顺序）。**Rust 侧没改** —— `close_session` 本来就是
"立刻丢弃"（plan 0204）。

**这条 E2E 第一次跑就红，抓到的是一个真 bug**（不是测试写错）：

1. **`term.dispose()` 会抛，而它跑在 React 的 effect 清理函数里。** 触发状态：终端的 WebGL
   上下文丢过、已经退到 canvas（`terminal_render` 的降级用例正好造成这个状态）。实测栈：
   `dispose@surface.ts` → `dispose@xterm` → `TypeError: undefined is not an object
   (evaluating 'this._linkifier2.onShowLinkUnderline')`。后果是**双重的**：
   - 抛在 effect 清理里 = React 卸载整棵树 → **关一个标签页，整个界面变空白**；
   - 它排在"关会话"前面 → 那一句**永远执行不到** → 标签页没了、PTY 与里面的作业留在机器上。
   修法两条：`surface.dispose()` 兜住异常（只记账），`attach.ts` 的清理**先交会话、后拆面**。
   用例因此**故意**先把那个状态造出来（`LOSE_WEBGL`），红-绿都实测过。
2. **多标签之后 `.xterm-helper-textarea` 不再唯一**：`terminal_render` / `exit_residue` 的输入
   helper 从"第一个 / 最后一个"改成**活动面**里的那个（`.tab-pane.is-active …`）—— 否则敲的命令
   可能落进一个隐藏的终端。
3. **探针原本是"最后挂载的那个面"**，多标签下会与"用户正在看的那个面"分叉（切回去、关掉别的
   标签页都会）。新增 `activateProbe(host)`，由活动标签页在拿到焦点时调用。

**未覆盖**

- 标签页重命名 / 拖拽排序 / 分屏 / OSC 标题同步（本步非目标）。
- 转发 / 密码库 / 文件传输的**视图标签页**（阶段 4/6/7）—— 本步只立了 `kind` 这个位置与规范。
- **前端类型检查不在 `just ready` 里**（它只覆盖 Rust + 文档）：本步靠 `pnpm build` 手动跑 + E2E
  真跑兜着。要不要把 `tsc` 纳入门禁（牵涉 CI 那条 job 装不装前端依赖）是**另一件事**，记在 `STATUS.md`。
- 每个面各有一份探针，但只有**活动面**能从 `window.__akashaTerminal` 读到；非活动面的屏幕要另开
  入口才能看（本步用不着）。
