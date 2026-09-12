# Plan 0301: 托盘图标 + 菜单

- **关联**：ROADMAP 阶段 3 ·「托盘图标 + 菜单」
- **前置**：plan 0204（退出路径的回收已经可靠 —— 托盘必须建立在"退得干净"之上）
- **状态**：未开始
- **影响面**：`src-tauri/Cargo.toml`（`tray-icon` feature）、`src-tauri/src/**`、`capabilities/*.json`、CI apt 依赖

## 目标

常驻系统托盘：图标 + 菜单（显示/隐藏窗口、**隧道列表与各自状态**、退出）。

菜单里的隧道列表是 `scope.md` §5.2 指定的**"失败可见"落点** —— 有限次重连后标记失败必须有地方显示，
否则就是最危险的失败模式：用户以为隧道还在转发，实际早已死掉。

## 非目标

- **不**做关闭行为配置（plan 0303）；**不**做单实例（plan 0304）
- **不**在本步实现隐藏语义（plan 0302）—— 本步只保证托盘存在与菜单可用
- **不**做隧道（阶段 6）：本步的隧道列表先渲染"空列表"，数据结构按 `SessionId` 路由预留

## 前置检查

```bash
just syscheck                        # 期望系统库齐（托盘用的是 libayatana-appindicator3）
grep -n 'appindicator' .github/workflows/ci.yml    # 期望 apt 列表里已有（坑 #10）
cargo metadata --no-deps --format-version 1 >/dev/null && echo OK
```

## 步骤

1. 开启 Tauri 的托盘能力（`tray-icon` feature），用 `TrayIconBuilder` 建图标与菜单。
2. 菜单项：显示/隐藏窗口、隧道列表（含各自状态）、退出。菜单项用**稳定的 id**，事件按 id 分发。
3. 图标资源：Linux / macOS 对尺寸与模板图标有要求（macOS 用 template image）；
   先存一份可用的最小资源，尺寸/模板的打磨留给 UI 阶段。
4. 退出菜单项必须走 plan 0204 的回收入口（`shutdown_all` → 退出），不允许另写一条退出路径。
5. `capabilities/*.json` 按最小权限加托盘相关 permission，**每条都要写理由**（`AGENTS.md` §4.3）。
6. 隧道列表的数据来源是 `SessionRegistry` 的隧道类 Session；此阶段它为空，但**事件管线要通**。

## 验收命令

```bash
just dev
# 1) 托盘可见（人工确认：系统托盘区出现图标，右键/左键能展开菜单）
#    可用 just doctor 确认 app 在跑；托盘是原生窗口，webview 工具看不到它

# 2) 点窗口关闭按钮后进程仍在（注意：本步之后才由 plan 0302 接管隐藏语义）
pgrep -af 'target/debug/akasha'      # 期望：仍有进程

# 3) 从托盘菜单退出 → 零残留
pgrep -af 'target/debug/akasha'      # 期望：无输出
```

**机器查不了的一项**（写成勾选项，人工确认）：托盘图标在 Linux / Windows / macOS 上可见且菜单可点。

## 回滚

关闭托盘 feature、移除菜单构建代码与资源；回滚后回到"关闭窗口即退出"（plan 0204 的行为）。

## 实施记录

（边做边追加：记录托盘在各平台的实际表现；Linux 上若 `libayatana-appindicator3` 缺失，
记下报错形态与处理 —— 这正是要实测的一项。）
