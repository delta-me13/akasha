# Plan 0303: 关闭行为可配置（收托盘 / 直接退出）

- **关联**：ROADMAP 阶段 3 ·「关闭行为可配置」
- **前置**：plan 0302（隐藏语义已成立，本步把它变成可选项）
- **状态**：未开始
- **影响面**：配置读取层（新增最小配置载体）、`src-tauri/src/**`、`crates/akasha-core`（配置类型）

## 目标

`close_behavior = tray | exit`（默认 `tray`），**回收逻辑读配置，不得硬编码"关窗即杀"**
（`AGENTS.md` §3.3）。

配置在阶段 4 之前**没有数据库**，所以本步需要一个最小载体：一个位于应用数据目录的
JSON / TOML 文件。**它必须在启动早期可读**（窗口事件之前），且**读不到时用默认值并可诊断**。

## 非目标

- **不**做配置 UI（`scope.md` §5.2：先做后端）
- **不**建数据库（阶段 4）；**不**把配置塞进 `tauri.conf.json`（那是打包期静态配置，不是用户配置）
- **不**改变默认值：默认仍是**收托盘**

## 前置检查

```bash
grep -rn 'close_behavior\|CloseRequested' src-tauri/src | head    # 现状：硬编码隐藏
ls docs/portable.md && sed -n '1,40p' docs/portable.md            # 数据目录推导的要求（P2）
```

## 步骤

1. 在 `crates/akasha-core` 定义配置类型与默认值（**可单测，零 Tauri 依赖**）。
2. 加最小配置载体：数据目录下的一个文件；读写集中在**一个模块**，不散落。
   - 路径推导遵循可搬迁要求（`docs/portable.md` §2：相对可执行文件，不存绝对路径）
3. 窗口事件按配置分支：
   - `tray`：`prevent_close` + `hide`（plan 0302 的行为）
   - `exit`：走 plan 0204 的 `shutdown_all()` → 真正退出，**零残留**
4. 配置解析失败 / 字段缺失 / 值非法：落回默认值并记一条 `tracing` 警告（不要 panic）。
5. 记录一条迁移债：阶段 4 的存储落地后，这个文件应并入数据库（在 plan 0403 的「前置检查」里回看一眼）。

## 验收命令

```bash
# 1. 单测覆盖解析与默认值
cargo nextest run -p akasha-core config        # 期望全绿（含非法值回退）

# 2. 行为切换（人工：改配置 → 重启 → 点叉）
just dev
#    默认（或显式 tray）：点叉后进程仍在
pgrep -af 'target/debug/akasha'                # 期望：仍有进程
#    改成 exit 并重启后点叉：立刻退出且零残留
pgrep -af 'target/debug/akasha|sleep 600'      # 期望：无输出

# 3. 确认没有硬编码
grep -rn 'hide()' src-tauri/src | head         # 目视：hide 只在按配置分支的那一处
```

## 回滚

回退配置读取与分支，回到固定隐藏（plan 0302 的状态）；配置文件本身可留在数据目录（不会被读取）。

## 实施记录

（边做边追加：记录两种配置下的**实际进程检查输出**与配置文件路径。）
