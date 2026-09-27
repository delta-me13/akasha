# Plan 0307: macOS 上从 Dock 唤回窗口

- **关联**：ROADMAP 阶段 3 ·「macOS 上从 Dock 唤回窗口」
- **前置**：plan 0301 / 0302（托盘与「隐藏而非销毁」）
- **状态**：已完成（2026-09-27）
- **影响面**：`src-tauri/src/lib.rs`、`src-tauri/src/single_instance.rs`、`docs/STATUS.md`、
  `ROADMAP.md`、`docs/plans/README.md`

## 目标

macOS 上窗口被收进托盘（隐藏）之后，**点 Dock 图标能把窗口唤回来**。

改动之前：全仓只处理 `RunEvent::Exit` 与 `RunEvent::Ready`（`src-tauri/src/lib.rs`），
**没有** `RunEvent::Reopen` 的处理点；而窗口隐藏之后唯一的唤回入口是托盘子菜单的
「显示/隐藏窗口」。Dock 的等价入口因此没有行为 —— 这与 `docs/scope.md` §5「关窗不退出、
必须能再次唤出」的口径不一致。

## 非目标

- **不做视觉/版式工作**（`AGENTS.md` §4.0）：本 plan 只补一条唤回行为，不碰界面观感。
- **不做托盘模板图标**：`docs/scope.md` §377 已把 macOS/Linux 的图标尺寸与模板图标归 UI 阶段。
- **不改关闭语义**：`close_behavior` 的两个取值与它们的降级口径（托盘不可用 → 关窗即退出）不变。

## 触发条件（实测得出的答案）

| 窗口状态 | 点 Dock 图标 | tauri 转出来的读数 |
|---|---|---|
| 隐藏 | 事件到达 | `Reopen { has_visible_windows: false }` |
| 已显示（再点一次） | 事件照常到达 | `Reopen { has_visible_windows: true }` |

两条都实测过（见「实施记录」）。因此**不按 `has_visible_windows` 分支**：点 Dock 图标在 macOS 上的
既有语义就是「把窗口带到前面」，而唤回的三步（还原 / 显示 / 置前）在已经可见、已经置前时都是空操作。

## 步骤

1. `single_instance::activate` 提为 `pub(crate)`，模块文档写明它现在有**两个**调用点
   （第二个实例敲门 / Dock 唤回）—— 两者要的是同一件事，抄第二份就会分叉。
2. `lib.rs` 的 `app.run` 闭包加 macOS 分支：`Reopen { has_visible_windows, .. }` →
   记一条 `window reopen requested`（`has_visible_windows` 进字段 —— 它是「事件到没到」的判据）→
   调 `single_instance::activate(handle)`。非 macOS 上没有那个变体，参数在那里用不到。

## 验收命令

```bash
# 起一个 dev 构建（沙箱外：pty 与 ~/.cargo 都在工作区之外）
just runner-submit dev

# 本机驱动走 Victauri 的**无会话 REST 接口**（MCP bridge 看不到沙箱外的发现目录，问题 #33）:
#   B=http://127.0.0.1:7373/api/tools   H="Authorization: Bearer $(cat <temp>/victauri/<pid>/token)"

# 1) 把锚点敲进终端：屏幕上出现 AKASHA-DOCK-MARKER
#    POST $B/eval_js  {"code":"…dispatchEvent(new InputEvent('input', {data:'echo AKASHA-DOCK-MARKER\\r'…"}
# 2) 隐藏窗口：POST $B/window {"action":"manage","manage_action":"hide"} → visible=false
# 3) 点 Dock 图标（**人工**：裸二进制没有 bundle，`open -a` 与 AppleScript 都够不着它）
# 4) 机器核对
#    window get_state        → visible=true 且 focused=true
#    eval_js screenText(400) → 仍含 AKASHA-DOCK-MARKER 与隐藏之前那条提示行的时间戳
#    app_state {probe:"sessions"} → live=registered=1（没有重启、没有新实例）
#    日志                    → window reopen requested has_visible_windows=false
# 5) 收尾：just runner-stop-dev
```

## 回滚

删掉 `lib.rs` 里那段 `Reopen` 分支、把 `activate` 收回私有即可。没有库格式、前端与配置改动。

## 实施记录

**2026-09-27（macOS 26.6.2 / arm64；dev 构建由 `just runner-submit dev` 在沙箱外运行，
经 Victauri 的无会话 REST 接口驱动）**

- 前置：`lifecycle` probe 报 `{"close_action":"hide","close_behavior":"tray","tray_ready":true}`
  —— 这台机器上托盘真的建成了，「隐藏窗口」这一档因此存在（不是靠猜）。
- 改动：`single_instance::activate` 提为 `pub(crate)`；`lib.rs` 的 run 闭包加
  `#[cfg(target_os = "macos")]` 的 `Reopen` 分支。首次编译报 `E0638`（那个变体标了
  `#[non_exhaustive]`，模式必须带 `..`）—— 已修。
- **实测读数**（同一进程，discovery 目录 `pid=25886` 全程未变）：
  * 敲入锚点之后屏幕上有 `AKASHA-DOCK-MARKER`；
  * `window manage hide` → `visible=false`、`focused=false`；
  * 人工点 Dock 图标之后：`visible=true`、`focused=true`；
  * `eval_js screenText(400)` 里**隐藏之前那条提示行的时间戳 `(23:06:44)` 与锚点都还在** ——
    同一个终端缓冲，没有被重建；
  * `app_state {probe:"sessions"}` → `live=1 registered=1`（没有重启、没有新实例）；
  * 日志两行：`window reopen requested has_visible_windows=false`（隐藏时那一次）与
    `window reopen requested has_visible_windows=true`（窗口已显示之后再点一次）。
- `just ready` 6/6。
- ⚠️ **一次人工动作不可省**：点 Dock 图标没有可编程的等价物。Linux / Windows 上无变化
  （那两处根本没有 `Reopen` 这个变体，编译期就不参与）。