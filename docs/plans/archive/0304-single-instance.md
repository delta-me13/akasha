# Plan 0304: 单实例

- **关联**：ROADMAP 阶段 3 ·「单实例」
- **前置**：plan 0302（有托盘与隐藏语义，"唤起已有窗口"才有意义）
- **状态**：已完成（2026-09-12）
- **影响面**：`src-tauri/Cargo.toml`、`src-tauri/src/**`、`src-tauri/tests/single_instance.rs`

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

### 怎么接的

- `tauri-plugin-single-instance = "2.4.4"`。机制按平台分：Linux = D-Bus 会话总线上的一个
  名字（`<identifier>.SingleInstance`）、Windows = 命名 mutex、macOS = `/tmp/<identifier>_si.sock`。
- 注册在**第一个插件位**（上游 README 也这么要求：插件的 setup 在 `build()` 里按注册顺序
  跑，排在前面 = 第二个实例在别的插件的 setup 之前就退掉）。
  ⚠️ **不是**为了"避免闪一个窗口"：全部插件的 setup 都跑在窗口创建（`RunEvent::Ready`）
  之前，与顺序无关 —— 步骤 1 说的"窗口创建之前"本来就由 tauri 的时序保证。
- 第二个实例"退出自身"（步骤 2 的后半）由插件做：名字被占 → 发一次 D-Bus 调用 →
  `cleanup_before_exit()` → `std::process::exit(0)`。

### 唤起：步骤 2 + 3 都落在 `activate()`

`unminimize()` → `show()` → `set_focus()`，**三步无条件都做**。两条理由都成立：
最小化的窗口 `is_visible()` 仍为真，按可见性分支就会漏掉"还原"；而已经可见、已经置前的
窗口上这三个调用本身就是空操作。于是"隐藏的窗口必须被显示出来"（步骤 3）是**结构上**
成立的，不依赖一次可能读错的状态判断。每一步失败只记一条 `warn`。

第二个实例的 payload（argv + cwd）在 v1 **不消费**：本 app 没有"用命令行打开一个文件 /
一条连接"的入口（`scope.md` 的能力清单里没有）。

### 判据与降级

- Linux 上机制依赖**外部服务**（会话总线），容器 / CI 的 xvfb 里没有。注册之前先问一次
  （`zbus::blocking::Connection::session()` —— 与插件随后要建立的是**同一条**连接）；
  连不上就不注册，`warn` + probe `registered=false`，**app 照常启动**（§3.3 的降级口径）。
- ⚠️ 那条 `warn` 留到 `.setup()` 里打：日志插件是 builder 的一环，在它注册之前 `tracing`
  没有 `log` 出口，那时打出去的记录会**静默消失**（坑 #47）。
- Windows / macOS 的机制不依赖外部服务，`available()` 直接为真。
- 加 `zbus` 只为了这一条判据（`target.'cfg(target_os = "linux")'`）。**不是新增 crate**：
  同一个版本早已在树里（`tauri-plugin-opener` → `zbus 5.19`）。
- `capabilities/*.json` **不动** —— 插件没有前端命令（最小权限，§4.3）。

### 步骤 4 的结论：**没有出现互相顶掉，因此不加 dev 专属 instance key**

实测：起实例 A → `kill -9`（`tauri dev` 重载是同一回事）→ 立刻起实例 B —— B 照常成为主实例
并打出 `single instance registered`。原因很具体：D-Bus 的名字挂在**连接**上，进程一死总线
立刻释放，没有"陈旧的锁"要清。

不加 key 的理由：插件的 `dbus_id` 只影响 Linux，加了会变成"只有 Linux 上能多开"，
与跨平台一致性相反；而 macOS 的 socket 与 Windows 的 mutex 本来就随进程消失。

### 实测（Linux / CachyOS + niri；`XDG_RUNTIME_DIR=/tmp/akasha-run`）

| 动作 | 结果 |
|---|---|
| 起第一个实例 | probe `{"activations":0,"registered":true}`；日志 `single instance registered` |
| `window manage hide` | `visible=false` |
| 再起同一个二进制 | **154–205 ms** 后 `exit=0`；probe `activations=1`；`visible=true` |
| 进程表 | 只有 app 与它的看门狗（`--akasha-session-watchdog`，plan 0205 的同一个可执行文件） |
| `kill -9` 后立刻重启 | 新实例照常注册（开发循环不被挡） |
| `unset DBUS_SESSION_BUS_ADDRESS` + runtime dir 里没有 `bus` | `single instance unavailable`、probe `{"registered":false}`、窗口照常起来 |

### E2E

新增 `tests/single_instance.rs`，五层判据：第二个实例自己以 0 退出 / app 上报的 `activations`
加一 / **藏起来的**窗口重新可见 / 藏之前屏幕上的内容仍在（同窗口同会话）/ 只有一个 app 进程。
已接进 `E2E_TARGETS`（排在 `window_close` 之后，两者都不关 app）。

- 前提不满足（`registered != true`）时**显式跳过**并打印 probe —— 与 `window_close` 靠
  `lifecycle` 决定跳过是同一个做法：判据来自 app 自己的状态，不是"试着起一个看看"。
- "只有一个 app 进程"那层靠 `/proc` 数：非 Linux 显式跳过；看门狗按 **argv 整参数**排除（坑 #49）。
- ⚠️ 本轮踩了一个新的（STATUS 坑 #68）：cargo 重建时会拿**新的 hardlink** 换掉
  `target/debug/akasha`，正在跑的那个进程的 `/proc/<pid>/exe` 于是带 ` (deleted)` 后缀、
  `canonicalize` 直接 NotFound —— 拿它做相等比较的结果是"一个实例都找不到"，用例红得毫无线索。

### 未覆盖

- Windows / macOS 上的实际表现未验（本机只有 Linux；CI 的类型检查挡不住运行期差异）。
- CI 的 Linux E2E 上这条用例会**跳过**（xvfb 没有会话总线）—— 与托盘、`window_close` 同一个缺口。
