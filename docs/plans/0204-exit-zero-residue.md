# Plan 0204: 真正退出零残留

- **关联**：ROADMAP 阶段 2 ·「真正退出零残留（窗口关闭退出 / app 重载 / panic 三条路径）」
- **前置**：plan 0203（已能开 shell 并看到输出）
- **状态**：未开始
- **影响面**：`src-tauri/src/**`（退出钩子、registry 的 shutdown-all）、`src-tauri/crates/akasha-pty`

## 目标

**真正退出**时每个 PTY 子进程都被**显式 kill + wait 收尸**，零残留。
覆盖三条路径：关闭窗口（当前 = 退出）、`tauri dev` 的 Rust 改动重载、panic。

> ⚠️ **"窗口关闭"在本 plan 里就是退出** —— 收托盘是阶段 3（plan 0301 / 0302）才存在的语义。
> 那时"关窗口不回收"会与本 plan 的方向**相反**，那条反向规则那时才落地。
> 本 plan 只保证：**真的退出时，一个子进程都不剩。**

## 非目标

- **不**做托盘（阶段 3）；**不**让窗口隐藏（plan 0302）
- **不**做 SSH / 隧道回收（阶段 5/6；届时 registry 的 shutdown-all 要覆盖它们）
- **不**用 drop 代替回收：`drop` 不是收尸路径（`AGENTS.md` §3.3）

## 前置检查

```bash
pgrep -af 'target/debug/akasha' || echo "当前无 app 在跑（预期）"
grep -rn 'kill\|wait' src-tauri/crates/akasha-pty/src | head    # 看 shutdown 路径现状
```

## 步骤

1. `SessionRegistry` 加**显式** `shutdown_all()`：逐个 `Transport::shutdown()` → `wait()` 收尸。
   - 不得依赖 `Drop`；不得只发信号不等回收（那会留下僵尸）
2. 挂三条退出路径：
   - **窗口关闭 / 退出事件**：先 `shutdown_all()` 再放行退出
   - **app 重载**（`tauri dev` 重编译重启）：同一入口必须被调用，不能在重载路径上漏掉
   - **panic**：设置 panic hook，尽力回收后再 abort（panic 路径允许"尽力"，但必须在日志里留下证据）
3. 回收顺序要确定：先停读循环（否则会继续从上锁的 PTY 读），再 kill，最后 wait。
4. 回归：反复起停 5 次，确认没有残留累积。

## 验收命令

```bash
just dev
# 1) 开一个 shell（例如跑 `sleep 600`），记下它的 PID（在终端里 `echo $$` 或从 ps 找）
# 2) 关闭窗口（当前 = 退出），然后在 shell 里确认：
pgrep -af 'sleep 600'     # 期望：无输出（零残留）
pgrep -af 'target/debug/akasha'   # 期望：无输出

# 3) app 重载路径：在 just dev 常驻的前提下改一个 Rust 文件触发重启，
#    重启完成后确认上一轮的子进程同样不残留：
pgrep -af 'sleep 600'     # 期望：无输出

# 4) 反复 5 次起停后仍无累积：
for i in 1 2 3 4 5; do echo "--- round $i"; pgrep -af 'target/debug/akasha|sleep 600' || echo clean; done
```

Victauri 侧（app 运行中）：`introspect { action: "processes" }` 记录子进程 PID ——
退出后拿这份清单在 shell 里逐个确认。**"收托盘时存在子进程"是阶段 3 的预期行为，不是本 plan 的失败。**

单测侧（可选但推荐）：`SessionRegistry::shutdown_all()` 对假 `Transport` 的调用序列
（shutdown → wait，且每个 session 恰好一次）。

## 回滚

回退退出钩子与 `shutdown_all`；回滚后回到"退出可能残留子进程"的旧状态（这是本来就该修的问题）。

## 实施记录

（边做边追加：记录三条路径各自的**实际残留检查输出**。）
