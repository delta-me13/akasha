# Plan 0306: 会话自身结束 = 回收该会话 + 关闭对应标签页

- **关联**：ROADMAP 阶段 3 ·「会话自己结束（敲 exit）= 回收该会话 + 关闭标签页」
- **前置**：plan 0305（标签页与会话一一对应）、0204（显式 kill + wait 回收子进程）、0205（撤销兜底登记）
- **状态**：已完成
- **影响面**：`src-tauri/src/session.rs`、`src-tauri/src/bindings.rs`、`src-tauri/src/lib.rs`、
  `src/ipc/**`、`src/terminal/**`、`src/App.tsx`、`src-tauri/tests/tab_close.rs`、
  `docs/scope.md`、`AGENTS.md` §3.3 / §4

## 目标

**用户修掉的 bug**：在终端里敲 `exit`（shell 自己退出）之后，那个**标签页还开着**，
而且后端那张会话表里还留着它 —— 一个已经结束的会话**没有任何路径回收它**（它只是"看起来还在"）。

1. **后端当场收尾**：载体的输出流结束（shell 退出 / PTY 关闭）时立刻
   `kill + wait 收尸`、注销注册、`SessionRegistry::close`、**撤销看门狗登记** ——
   与"用户关标签页"走的是同一套收尾，只是触发者不同。
2. **发一个事件**告诉前端"这个会话自己结束了"（`SessionEnded`，按 handle 寻址）。
3. **前端关闭那个标签页**：标签页与会话**同生命期**（0305 立的一一对应），
   两个方向都要成立 —— 用户关标签页 → 丢弃会话；会话自己走 → 标签页跟着走。

## 非目标

- **不做"已退出"的中间态**（灰掉的标签页、`[进程已结束]` 之类的提示）—— 直接关闭。
  要展示什么、怎么展示，等**正式 UI 设计稿**（`docs/scope.md` §1.3：当前前端只是验证壳层）。
- **不改版式 / 视觉 / 交互**：本步只加一条"会话结束"的数据流。
- 不做 SSH / serial 的会话结束（阶段 5/8）：它们将来接的是同一个入口（`Transport` 的输出流
  结束 = 会话结束），本步只有本地 PTY 能真正运行。
- 不引入 `tauri-specta` 的 `derive` 特性（那会多一个 proc-macro 依赖）：事件类型只有一个
  常量要写，手写 3 行 `impl` 更省（见代码注释）。

## 前置检查

```bash
grep -n "forward(\|spawn_batcher" src-tauri/src/session.rs        # 输出流结束 → forward 线程结束
grep -rn "events\b" src/ipc/bindings.ts | head                    # 目前**没有**事件（本步是第一个）
just ready                                                        # 改之前的基线
```

## 步骤

1. **事件**（`src-tauri/src/session.rs`）：`SessionEnded { handle, status }`，
   `Serialize + Deserialize + specta::Type`，手写 `impl tauri_specta::Event`；
   在 `bindings.rs` 的 `builder()` 上挂 `.events(collect_events![SessionEnded])`，
   并在 `.setup()` 里 `mount_events`（不 mount 会在发事件时 panic）。
2. **`Sessions::retire(handle)`**：注销注册 + `transport.shutdown()`（回收子进程）+ `registry.close`
   + `forget(leader)`。**必须幂等**（`Ok(None)` = 已经不在了）：用户点 × 与 shell 自己退出
   可能几乎同时发生。收尾失败只记录日志，**不中途终止收尾**（会话已经结束，留着登记更糟）。
3. **`open_session` 加一个收尾线程**：`forward(...)` 的句柄 `join` 之后再 `retire` + 发事件。
   **顺序有严格要求**：`forward` 结束 = 最后一批已经交给频道 —— 事件**排在那之后**，
   否则前端可能"先收到会话结束、后收到最后一段输出"。
4. **前端订阅**（`src/ipc/session.ts`）：一个全局监听 + `handle → 回调`表；
   `TerminalSession` 记录"已结束"，此后 `write` / `resize` / `close` 都是**空操作**
   （后端已经注销注册，再发命令只会收到 `NotFound`）。
   还要覆盖一个窄窗口：事件可能**早于**订阅到达（会话建立后立即结束）——
   那时把 handle 记下来，订阅时**补发**。
5. **接线**：`attach.ts` 的 handlers 加 `onEnded` → `TerminalPane` 加同名 prop →
   `App.tsx` 收到就 `closeTab(key)`（复用既有的关标签页路径，不新增第二条）。
6. **E2E**（`src-tauri/tests/tab_close.rs` 续一段）：在干净标签页里敲 `exit` →
   标签页自行关闭（0 个 = 空状态）、app 仍在；再开一个还能用（给 `exit_residue` 留个能用的终端）。
7. **文档**：`docs/scope.md` §5.6 补"反方向"与"标签页 ⇔ 会话同生命期"；
   `AGENTS.md` §3.3 事件表加一行；`AGENTS.md` §4 开头写明**当前前端只是验证壳层**
   （正式 UI 布局/视觉没有设计稿），`docs/scope.md` §1.3 展开。

## 验收命令

```bash
just ready        # 6/6（含 gen-types-check：新事件必须已生成并提交）
pnpm build        # 前端类型检查（tsc）—— 它不在 just ready 里
just test-e2e     # 自包含：启动 Vite + app → 逐个目标 → 收尾
```

`tab_close` 用例新增段落必须打印：

```
✅ 在终端里敲 exit：会话自己结束，标签页跟着关掉（app 仍在）
```

手工复核（`just dev`，不要在有后台作业的标签页里试 —— 见「未覆盖」）：

```bash
just dev
# 1) 新开一个标签页，敲 exit —— 标签页应当自己消失（回落到空状态或上一个标签页）
# 2) 同时看宿主机进程表：`sleep 600` 一类都不该留下
```

## 回滚

删掉事件与 `retire` 的接线（`open_session` 回到只 `forward`），前端去掉订阅。
**回滚之后行为退回 0305 之前**：会话自己结束时无人收尾（表里留一条已结束的会话）。

## 实施记录

| 命令 | 结果 |
|---|---|
| `just ready` | **6/6 全部通过**（含 `gen-types-check`：新事件已生成并提交） |
| `just test` | **70 tests run: 70 passed**（新增 3 条 `retire` 单测：回收干净 / 幂等 / 回收子进程失败也注销注册） |
| `just test-e2e` | 退出码 **0**，10 个用例全部通过（`tab_close` 多验了一段"敲 exit"） |
| `pnpm build`（tsc + vite build） | 退出码 0 |

真实 app 上的实测（`tab_close` 的新段落 + app 日志里那一行）：

```
在终端里敲 exit：标签页自己关掉（app 仍在）
✅ 在终端里敲 exit：会话自己结束，标签页跟着关掉（app 仍在）
[akasha_lib::session][INFO] 会话自己结束：已收掉并从登记簿摘牌 handle=8 session=8 retired.status=Some(Code(0))
```

**实现**：事件 `SessionEnded`（`src-tauri/src/session.rs`，手写 `impl tauri_specta::Event`
—— 不引 `derive` 特性，那要多一个 proc-macro 依赖）+ `Sessions::retire`（幂等、不中途终止收尾）
+ `open_session` 的收尾线程（**`join` 掉 `forward` 之后**才收尾并发事件）
+ 前端订阅（`src/ipc/session.ts`：一个全局监听 + `handle → 回调`表，含"事件早于订阅"的补发）
+ `attach.ts` / `TerminalPane` / `App.tsx` 把它接到既有的关标签页路径上。

**三个问题**（都记进了 `docs/STATUS.md`）：

1. **`tauri-specta` 的事件必须 `mount_events`**：命令靠 `invoke_handler` 就够了，
   事件漏了这一步**不在启动时报错**，而是在**发**的时候 panic（`EventRegistry not found`）。
2. **官方 `Channel` 不会通知"流结束了"**：它收到收尾帧 `{index, end:true}` 时只把回调
   注销掉（`cleanupCallback`），**不通知** `onmessage` —— 前端光看字节通道看不出会话已经结束。
   所以"会话结束"必须由**事件**通知。
3. **会话里还有别的进程握着 PTY 时，主端读不到 EOF**：这时会话**不算**结束、标签页**不该**关闭
   （真实终端同样是这个行为）。用例因此必须在**干净**的标签页里敲 `exit`。

**一个几乎被引入的 bug**：会话结束的回调**必须走 `ref`** —— `attachTerminal` 只在挂载时接一次线
（`useEffect(…, [])`），而 `App` 每次渲染都会传一个新的箭头函数（它闭包着**当时**的标签页列表）。
直接接那个函数的话，"第二个标签页里的 shell 退出"会走进一份过期闭包，`findIndex` 找不到那个 key
——表现就是**标签页无法关闭**（而且只在多标签时出现）。

**未覆盖**

- SSH / serial 的"会话自己结束"（阶段 5/8）：接的是**同一个入口**（输出流结束 = 会话结束），
  但现在没有能真正运行的载体。
- "会话结束后标签页显示成什么"（灰掉 / 保留输出 / 自动关）—— 属于正式 UI，见 `scope.md` §1.3。
- 事件与最后一批输出的**严格顺序**没有保证：事件排在 `forward` 结束之后发，但频道与事件是
  两条通道。极端情况下最后一屏可能来不及画就被关闭（敲 `exit` 本来就没什么可看）。
- 在**有后台作业**的标签页里敲 `exit` 不会关标签页（见问题 3）—— 这是刻意的，但没有用例守着
  （要造一个"握着 PTY 的后台作业"再断言"标签页还在"）。
