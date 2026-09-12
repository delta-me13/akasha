# Plan 0302: 隐藏而非销毁窗口

- **关联**：ROADMAP 阶段 3 ·「隐藏而非销毁窗口」
- **前置**：plan 0301（托盘已存在，否则"隐藏后回不来"）
- **状态**：未开始
- **影响面**：`src-tauri/src/**`（窗口事件与 app 退出拦截）、配置读取（见 plan 0303）

> ⚠️ **本步不能单独落地，要与 0303 一起**（0301 实施后新增的判断）：
>
> 1. 没有配置项之前，"点叉 = 隐藏"会让**面板里没有托盘模块的用户无法退出 app**
>    （托盘注册成功 ≠ 图标看得见，坑 #60）。0303 的 `close_behavior` 就是那条退路。
> 2. `exit_residue`（E2E）现在靠"关窗口 = 真退出"取刺激。本步之后那条刺激消失，
>    而配置项（`close_behavior = exit`）正好给出一个**可移植**的替代刺激。

## 目标

拦截窗口关闭 → `prevent_close()` + `hide()`；并在 app 层兜住"最后一个窗口关闭 = 退出"的默认行为。

**这不是实现偏好，是语义要求**：销毁窗口会连带销毁所有 `Session`，
按「连接 = Session 生命周期」（`scope.md` §2.2）就会**立刻关掉全部隧道**，直接抵消托盘的意义。

## 非目标

- **不**做"关闭即退出"的可配置开关（plan 0303）—— 本步固定为隐藏
- **不**回收任何 Session / 子进程：本步之后，**"窗口关闭时存在子进程"成了预期行为**（见验收）
- **不**做隧道（阶段 6）

## 前置检查

```bash
grep -rn 'CloseRequested\|prevent_close\|prevent_exit' src-tauri/src || echo "尚未拦截（预期）"
just dev        # 另开终端；app 需要跑着才能做下面的验证
```

## 步骤

1. `WindowEvent::CloseRequested` → `api.prevent_close()` + `window.hide()`。
2. app 层 `RunEvent::ExitRequested` → 在"默认关闭行为 = 隐藏"时 `prevent_exit()`，
   否则"关掉最后一个窗口"仍会让进程退出（Tauri 的默认行为会绕过我们的隐藏逻辑）。
3. 托盘"显示/隐藏"菜单项与 plan 0301 的菜单接线（显示 = `show()` + `set_focus()`）。
4. 把"窗口关闭不回收"这条**反向规则**写进代码注释与 `AGENTS.md` §3.3 已存在的表格对照处 ——
   它足够反直觉，一个"勤快"的清理逻辑会把用户正在用的终端和隧道全杀掉（`scope.md` §8 风险 1）。

## 验收命令

```bash
just dev
# 1) 在终端里产生可见回滚内容（例如 seq 1 200），然后关闭窗口
# 2) 进程仍在：
pgrep -af 'target/debug/akasha'      # 期望：仍有进程
# 3) 从托盘把窗口显示回来 —— 回滚缓冲必须还在（证明窗口是隐藏而非重建）
#    人工确认：能看到 seq 1 200 的输出；或用 Victauri eval_js 读取 xterm 缓冲行数
```

Victauri 侧（窗口显示回来之后，app 在跑）：
- `eval_js` 读 xterm 实例的 buffer 长度（`term.buffer.active.length`）—— 期望与隐藏前一致；
- `introspect { action: "processes" }` —— 期望 shell 子进程仍在（**这是预期行为，不是泄漏**）。

## 回滚

移除 `prevent_close` / `prevent_exit` 拦截，回到 plan 0301 的"关闭即退出"。

## 实施记录

（边做边追加：记录隐藏前后**缓冲行数与子进程清单**的实际输出。）
