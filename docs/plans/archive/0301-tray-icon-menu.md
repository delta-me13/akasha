# Plan 0301: 托盘图标 + 菜单

- **关联**：ROADMAP 阶段 3 ·「托盘图标 + 菜单」
- **前置**：plan 0204（退出路径的回收已经可靠 —— 托盘必须建立在"退得干净"之上）
- **状态**：已完成（2026-09-12）
- **影响面**：`src-tauri/Cargo.toml`（`tray-icon` feature）、`src-tauri/src/tray.rs`（新）、
  `src-tauri/src/session.rs`（会话表变更通知）、`src-tauri/src/lib.rs`（接线）

## 目标

常驻系统托盘：图标 + 菜单（显示/隐藏窗口、**隧道列表与各自状态**、退出）。

菜单里的隧道列表是 `scope.md` §5.2 指定的**"失败可见"落点** —— 有限次重连后标记失败必须有
地方显示，否则就是最危险的失败模式：用户以为隧道还在转发，实际早已死掉。

## 非目标

- **不**做关闭行为配置（plan 0303）；**不**做单实例（plan 0304）
- **不**在本步实现隐藏语义（plan 0302）—— 本步只保证托盘存在与菜单可用。
  ⚠️ 所以"点叉后进程仍在"**不属于本步的验收**：它要求 `prevent_close`，那是 0302 的行为。
  （原判据把它写在了本步，已修正 —— 判据写在错的 plan 里，做的人只会做出一个半成品。）
- **不**做隧道（阶段 6）：本步的隧道列表先渲染"空列表"，数据结构按 `SessionId` 路由预留

## 前置检查

```bash
just syscheck                        # 期望系统库齐（托盘走 libayatana-appindicator3）
grep -n 'appindicator' .github/workflows/ci.yml    # 期望 apt 列表里已有（坑 #10）
cargo metadata --no-deps --format-version 1 >/dev/null && echo OK
```

## 步骤

1. 开启 Tauri 的托盘能力（`tray-icon` feature），用 `TrayIconBuilder` 建图标与菜单。
2. 菜单：显示/隐藏窗口、隧道列表、退出；**稳定的菜单 id**，事件按 id 分发（不按文案）。
   图标复用 `bundle.icon` 里那张 —— 构建脚本已经把它解码进二进制，不必再加图片解码依赖。
3. **会话表一变就重推菜单**（`Sessions::on_change`）：隧道列表将来靠这条管线跟着走。
4. 退出项必须走 plan 0204 的回收入口（`shutdown_all` → `app.exit`），不允许另写退出路径。
5. `capabilities/*.json` **不动**：托盘全在 Rust 侧建，前端一次都不碰（最小权限，§4.3）。
6. 建不起托盘**不挡启动**：Linux 上图标要写 `$XDG_RUNTIME_DIR/tray-icon`，只读环境里会失败。

## 验收命令

```bash
just dev
# 1) 托盘注册到宿主的 StatusNotifierWatcher
#    ⚠️ 别 grep 进程名：SNI 用的是**唯一名** `:1.<n>`，路径里才是我们的标识
gdbus call --session --dest org.kde.StatusNotifierWatcher --object-path /StatusNotifierWatcher \
  --method org.freedesktop.DBus.Properties.Get \
  org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems
# 期望：列表里出现 ':1.<n>/org/ayatana/NotificationItem/tray_icon_tray_app_akasha'

# 2) 菜单内容（对象路径 = 上面那个 + /Menu，接口 com.canonical.dbusmenu）
gdbus call --session --dest :1.<n> --object-path <那个>/Menu \
  --method com.canonical.dbusmenu.GetLayout 0 2 "@as []"
# 期望：显示/隐藏窗口 / 隧道（下挂 enabled=false 的"（暂无隧道）"）/ 退出 akasha

# 3) 点菜单项（`Event <id> clicked "<\"\">" 0`）触发显示/隐藏与退出；
#    ⚠️ **每次点击前重读布局** —— 菜单一重建 item id 就换（坑 #62）
pgrep -af 'target/debug/akasha'      # 退出之后期望：无输出
```

**机器查不了的一项**（人工确认）：托盘图标在面板里**看得见**（"注册成功"≠"面板有托盘模块"），
以及 Windows / macOS 上的实际表现。

## 回滚

关掉 `tray-icon` feature、删掉 `src-tauri/src/tray.rs` 与 `lib.rs` 的接线；
`Sessions::on_change` 没有订阅者时是空操作，可以留着。

## 实施记录（2026-09-12）

落地：`src-tauri/src/tray.rs`（图标 / 菜单 / 事件分发 / `refresh`）、`Sessions::on_change` +
`tunnels()`、`lib.rs` 的 `.setup()` 接线；四条新增单测（register/close/retire/shutdown 都发通知、
钩子必须在锁外调用、第二个订阅者被忽略、终端不被列成隧道）。

实测（Linux / niri；宿主面板提供 `org.kde.StatusNotifierWatcher`）：

| 判据 | 实测结果 |
|---|---|
| 托盘注册 | ✅ 注册表里出现 `:1.<n>/org/ayatana/NotificationItem/tray_icon_tray_app_akasha` |
| 菜单内容 | ✅ `GetLayout` → 显示/隐藏窗口 / 隧道（子项"（暂无隧道）" `enabled=false`）/ 退出 akasha |
| 会话表一变就重推菜单 | ✅ 开一个会话后 `revision 4 → 5`，item id 随之改变（这正是"重推了"的证据） |
| 菜单 → 显示/隐藏 | ✅ 点击后 `window get_state` 的 visible：true → false → true；**隐藏期间页面仍在应答** |
| 菜单 → 退出 → 零残留 | ✅ app 退出码 0；忽略 SIGHUP 的探针（pid 329）消失；日志 `sessions reclaimed reclaimed=1 trigger="tray"` |
| 退化路径 | ✅ 只读 `$XDG_RUNTIME_DIR`（沙箱）下只记一条 `tray unavailable` warn，app 照常启动 |

环境两条（都记进了 STATUS 坑）：本机**只装了 2012 年的 libappindicator 12.10.1**，
而 `tray-icon` 的加载顺序是 `libayatana-appindicator3.so.1` → `libappindicator3.so.1`
—— 验证时把 libayatana 0.6.0 解到临时目录用 `LD_LIBRARY_PATH` 跑（**不改系统**）。

遗留（**不属本步**）："点叉 = 收托盘"没做（plan 0302）。它与 0303 **耦合**：
没有配置项之前，面板里没有托盘模块的用户将**无法退出 app**；而 `exit_residue`（E2E）
现在靠"关窗口 = 真退出"取刺激，0302 之后那条刺激需要配置项才保持可移植。
