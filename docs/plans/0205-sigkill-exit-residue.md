# Plan 0205: 被 SIGKILL 的退出路径也零残留

- **关联**：ROADMAP 阶段 2 ·「被 SIGKILL 的退出路径也零残留」
- **前置**：plan 0204（能跑代码的两条路径已经零残留；本 plan 只处理"没人能跑代码"的那条）
- **状态**：未开始
- **影响面**：`src-tauri/crates/akasha-pty`（spawn 路径）、`src-tauri/Cargo.toml`（可能新增 bin / 依赖）

## 目标

`tauri dev` 重编译重启（以及 `kill -9`）之后，**上一轮会话里的进程一个都不剩** ——
包括**忽略 SIGHUP** 的那些。

## 为什么 0204 修不了它（实测，不是猜测）

`tauri dev` 的重启是 `child.kill()` → `shared_child::SharedChild::kill()` → **SIGKILL**。
SIGKILL 不可捕获，进程中**没有任何代码会被执行** —— 所以"退出前显式回收"这条路
在这条路径上从原理上就不通。0204 的实测输出：新一代 app 起来了，上一轮的探针仍活着。

## 非目标

- **不**做托盘（阶段 3）；**不**改 0204 已经验证过的两条路径
- **不**用"扫描孤儿进程"当方案：见下面被否掉的候选 3

## 候选方案（开工前先定一个，并写一条 ADR）

1. **Linux `PR_SET_PDEATHSIG`**：子进程在 exec 前把"父死则我死"设成 SIGKILL，内核兜底，
   SIGKILL 也带得走。**卡点**：`portable-pty 0.9` 没暴露 `pre_exec`（`unix.rs` 里的 pre_exec
   是它自己 internal 用的），所以要么给上游提 PR，要么自带一个极小的 `akasha-spawn` 辅助程序
   （先 `prctl` + `setsid`，再 `exec` 成用户 shell）。
   ⚠️ `PDEATHSIG` 的"父"是**创建它的那个线程**，不是进程 —— 必须有**长驻的 spawn 线程**
   （或主线程）来发起 spawn，否则一个线程池 worker 退出就会误杀会话。
2. **会话监管进程（supervisor）**：PTY 子进程挂在监管进程下，app 死了监管进程察觉
   （管道 EOF）后收掉全部子进程。代价最大，但它同时也是 SSH/隧道回收的最终形态
   （阶段 5/6 会需要），值得单独立项。
3. ~~扫孤儿进程~~（**否掉**）：重启后的新实例去 /proc 里找"父是 1、带着我们特征"的进程再杀。
   判据不可靠（可能杀到别人的进程），而且只能救"有人重新启动"的场景。

平台差异照旧：Windows 的等价物是 Job Object（`kill on job close`），macOS 没有内核级等价物，
只能靠 supervisor。

## 前置检查

```bash
# 现在到底是不是 SIGKILL（上游实现）：期望 SharedChild::kill
grep -n 'dev_child.kill()' <tauri-cli 源码>/src/interface/rust.rs
# 上游有没有 pre_exec：期望 0（没有可用的钩子）
grep -rn 'pre_exec' ~/.cargo/registry/src/*/portable-pty-0.9.0/src/cmdbuilder.rs
```

## 步骤

1. 先定方案（ADR）：1 还是 2；把"父是线程不是进程"这条坑写进决定里。
2. 落地方案；**不新造退出路径** —— 0204 的 `Sessions::shutdown_all()` 仍是唯一回收入口。
3. 平台差异显式化：Linux 做全套；Windows/macOS 至少把"做不到"写进文档与 STATUS。

## 验收命令

```bash
# 探针：忽略 SIGHUP 的进程（只有点名 SIGKILL 收得走 —— 见 plan 0204）
just dev                                     # 常驻
# 在终端里跑：sh -c 'trap "" HUP; echo AKPROBE=$$; exec sleep 600' &  记下 pid
touch src-tauri/src/bindings.rs              # 触发重编译重启
# 等新一代 app 起来之后：
pgrep -af 'sleep 600'                        # 期望：无输出（本轮修好的样子）
```

## 回滚

回退 spawn 路径的改动；回滚后回到 plan 0204 的记录状态（两条路径干净、重载路径有残留）。

## 实施记录

（边做边追加。）
