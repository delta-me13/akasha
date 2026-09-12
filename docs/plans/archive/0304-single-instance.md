# Plan 0304: 单实例

- **关联**：ROADMAP 阶段 3 ·「单实例」
- **前置**：plan 0302（有托盘与隐藏语义，"唤起已有窗口"才有意义）
- **状态**：未开始
- **影响面**：`src-tauri/Cargo.toml`、`src-tauri/src/**`、`capabilities/*.json`

## 目标

第二个实例**唤起已有窗口**，而不是各跑一套 —— 否则会出现两条隧道指向同一目标、
两套托盘图标、以及"退出一个还剩一个"的困惑（`scope.md` §5.5）。

## 非目标

- **不**做多窗口 / 多 profile（未评估，也不在 v1 范围）
- **不**做"同机多人共用"之类的会话共享
- **不**依赖 OS 专有单实例机制（跨平台一致性优先）

## 前置检查

```bash
grep -rn 'single_instance\|single-instance' src-tauri/Cargo.toml src-tauri/src || echo "尚未接入（预期）"
pgrep -c -f 'target/debug/akasha' || echo 0      # 开工前应为 0
```

## 步骤

1. 接入单实例能力（Tauri 的 `single-instance` 插件或等价实现），**启动即注册**，
   在窗口创建之前 —— 否则第二个实例会先闪一个窗口再退出。
2. 第二个实例的 payload 处理：显示已有窗口（`show()` + `set_focus()`），并退出自身。
3. 与 plan 0302 的隐藏语义对齐：窗口若处于隐藏状态，唤起时必须**显示出来**（不能只 focus 一个隐藏窗口）。
4. 与开发循环的关系：`just dev` 重启 app 时是"新进程 + 旧的已退出"，不要被单实例机制挡住重启；
   若出现在开发模式下互相顶掉的问题，记录并给出显式开关（例如 dev 下带独立 instance key）。

## 验收命令

```bash
cargo build --manifest-path src-tauri/Cargo.toml    # 或用 just dev 起第一个实例
# 1) 起第一个实例后，再起第二个（同一二进制）：
pgrep -c -f 'target/debug/akasha'     # 期望：1（第二个实例已退出）
# 2) 第二个实例应把已有窗口唤起并置前（人工确认：窗口出现并获得焦点）
# 3) 退出唯一实例后零残留：
pgrep -af 'target/debug/akasha'       # 期望：无输出

# 4) 确认没有把 just dev 的重启挡掉：改一个 Rust 文件，app 应照常重启
```

## 回滚

移除单实例注册，回到"可以起多个实例"的行为；无数据影响。

## 实施记录

（边做边追加：记录三种平台（至少 Linux）下的实际表现，以及 `just dev` 与单实例的交互结论。）
