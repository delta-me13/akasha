# Plan 0302: 隐藏而非销毁窗口

- **关联**：ROADMAP 阶段 3 ·「隐藏而非销毁窗口」
- **前置**：plan 0301（托盘已存在，否则"隐藏后回不来"）
- **状态**：已完成（**与 0303 同一批落地**，理由见下）
- **影响面**：`src-tauri/src/**`（窗口事件）、`src-tauri/crates/akasha-core`（判据）

> ⚠️ **本步不能单独落地，要与 0303 一起**（0301 实施后新增的判断）：
>
> 1. 没有配置项之前，"点叉 = 隐藏"会让**面板里没有托盘模块的用户无法退出 app**
>    （托盘注册成功 ≠ 图标看得见，问题 #60）。0303 的 `close_behavior` 就是那条退路。
> 2. `exit_residue`（E2E）现在靠"关窗口 = 真退出"取刺激。本步之后那条刺激消失，
>    而配置项（`close_behavior = exit`）正好给出一个**可移植**的替代刺激。

## 目标

拦截窗口关闭 → `prevent_close()` + `hide()`；并在 app 层兜住"最后一个窗口关闭 = 退出"的默认行为。

**这不是实现偏好，是语义要求**：销毁窗口会连带销毁所有 `Session`，
按「连接 = Session 生命周期」（`scope.md` §2.2）就会**立刻关闭全部隧道**，直接抵消托盘的意义。

## 非目标

- **不**做"关闭即退出"的可配置开关（plan 0303）—— 本步固定为隐藏
- **不**回收任何 Session / 子进程：本步之后，**"窗口关闭时存在子进程"成了预期行为**（见验收）
- **不**做隧道（阶段 6）

## 前置检查

```bash
grep -rn 'CloseRequested\|prevent_close\|prevent_exit' src-tauri/src || echo "尚未拦截（预期）"
just dev        # 另开终端；app 需要运行着才能做下面的验证
```

## 步骤

1. `WindowEvent::CloseRequested` → `api.prevent_close()` + `window.hide()`。
2. app 层 `RunEvent::ExitRequested` → 在"默认关闭行为 = 隐藏"时 `prevent_exit()`，
   否则"关闭最后一个窗口"仍会让进程退出（Tauri 的默认行为会绕过我们的隐藏逻辑）。
3. 托盘"显示/隐藏"菜单项与 plan 0301 的菜单接线（显示 = `show()` + `set_focus()`）。
4. 把"窗口关闭不回收"这条**反向规则**写进代码注释与 `AGENTS.md` §3.3 已存在的表格对照处 ——
   它足够反直觉，一个"勤快"的清理逻辑会把用户正在使用的终端与隧道一并终止（`scope.md` §8 风险 1）。

## 验收命令

```bash
just dev
# 1) 在终端里产生可见回滚内容（例如 seq 1 200），然后关闭窗口
# 2) 进程仍在：
pgrep -af 'target/debug/akasha'      # 期望：仍有进程
# 3) 从托盘把窗口显示回来 —— 回滚缓冲必须还在（证明窗口是隐藏而非重建）
#    人工确认：能看到 seq 1 200 的输出；或用 Victauri eval_js 读取 xterm 缓冲行数
```

Victauri 侧（窗口显示回来之后，app 在运行）：
- `eval_js` 读 xterm 实例的 buffer 长度（`term.buffer.active.length`）—— 期望与隐藏前一致；
- `introspect { action: "processes" }` —— 期望 shell 子进程仍在（**这是预期行为，不是泄漏**）。

## 回滚

移除 `prevent_close` / `prevent_exit` 拦截，回到 plan 0301 的"关闭即退出"。

## 实施记录

**做完的样子**：`CloseRequested` 上判一次 —— 配置要收托盘 **且托盘真的建成了** →
`hide()` 成功之后 `prevent_close()`；否则放行（关窗 = 退出，走 0204 的收尾路径）。

**与计划不同的两处**（都是核对 tauri 源码 + 实测之后改的）：

1. **不挂 `RunEvent::ExitRequested`，也不写 `prevent_exit`**（原步骤 2 取消）。
   理由：`AppHandle::exit()` **同样会触发 `ExitRequested`**（2.11 的 `exit()` 文档与
   `tauri-runtime-wry` 的 `Message::RequestExit` 都是这么写的），而托盘菜单的"退出"正是走它
   —— 在那里 `prevent_exit` 等于把唯一的退出入口一起拦掉。关窗这条路已经在
   `CloseRequested` 里拦下（窗口**不会被销毁**），不需要它兜底。剩下唯一会触发
   `ExitRequested` 的窗口路径是"窗口被 `destroy()` 强制销毁"，而那时窗口已经被销毁、
   从托盘也无法恢复 —— 放行（退出 + 回收）才是对的（与问题 #60 同一条道理）。
   这条判断连"为什么"一起写进了 `lib.rs` 的 `app.run` 注释，免得下次有人再补一遍。
2. 判据没留在 app 侧：`配置 × 托盘可用性 → 动作` 是个纯函数，落在
   `akasha-core::CloseAction::decide`（三条分支各一条单测）；`src/lifecycle.rs` 只做三件事
   —— 记录启动期事实、接窗口事件、给 Victauri probe 一份快照。

**实测**（Linux/CachyOS + niri + waybar；沙箱里 `$XDG_RUNTIME_DIR` 只读，用
`XDG_RUNTIME_DIR=/tmp/akasha-run` + `wayland-1` 软链绕过；app 与测试在**同一次 bash 调用**里，问题 #33）：

| 判据 | 实测 |
|---|---|
| 关窗前 | `app_state {probe:"lifecycle"}` → `{"close_action":"hide","close_behavior":"tray","tray_ready":true}` |
| 关窗（`window manage close` → `Window::close()` → `CloseRequested`） | 进程**仍在**；`window get_state` → `visible: false` |
| 隐藏期间 | 屏幕内容照旧读得到（`screenText` 仍含关窗前的标记）；忽略 SIGHUP 的探针**仍在运行**（**预期行为，不是泄漏**） |
| 显示回来 | `visible: true`；再敲命令有输出 → 还是原来那个会话 |
| 自动化 | `just test-e2e` **第一段**新增目标 `window_close`（真关窗 + 真按键，判据 4 层） |

**没做**：`window_close` 在 CI（xvfb：没有会话总线、没有可写 runtime dir）上**必然跳过** ——
托盘宿主是它的前提，与托盘本身没有自动化门禁是同一件事（见 `STATUS.md` 的「待验证」）。
跳过的判据来自 app 自己上报的 probe，不是"猜它可能不满足"。
