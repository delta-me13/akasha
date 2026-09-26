# Agent 执行器：沙箱受限时的构建与运行出口

只在受限环境（本机 DSH 的 workspace-write 沙箱）里需要，完整权限的终端不需要它。能力表（policy）
在工作区之外，执行器内部没有任何命令表。MCP 工具列表里已有同名实现时优先用 MCP，见 §7。

## 1. 解决什么

macOS 上 DSH 给 workspace-write 会话套的 Seatbelt 配置只放行 `/dev/null` 与工作区的写，
于是两类操作必然失败，且失败信息看起来像工具或代码故障：

| 现象 | 原因 |
|---|---|
| `spawn /bin/sh 失败: failed to openpty: ... PermissionDenied` | `/dev/ptmx` 的 O_RDWR 落在 `(deny file-write*)` 上，与登录 shell 是 zsh 还是 bash 无关 |
| `failed to open ~/.cargo/registry/cache/... Operation not permitted` | `~/.cargo` 在工作区之外，cargo 每次构建都要取它的包缓存锁 |

`just dev` 与 `just test` 两样都要，所以在沙箱内既不能构建也不能运行本应用。

## 2. 信任模型

能力只来自工作区之外的 policy 文件，执行器只承认其中登记过的动作：

| 约束 | 实现 |
|---|---|
| 无任意命令 | 动作表只从 policy 读取；脚本内部没有可用命令表 |
| 无参数通道 | 请求只能携带一个动作名（`^[a-z][a-z0-9-]{0,31}$`）与一个 16 位十六进制的请求 id，无法夹带参数、路径或元字符 |
| 无 shell | 一律 `execve`，固定 `workdir` 与最小环境，argv 原样取自 policy |
| 改脚本即失效 | 脚本 sha256 必须等于 `policy.script_sha256`，否则拒绝启动 |
| policy 不能藏进可写处 | policy 位于工作区或临时目录时 `--check` 直接拒绝，这两处对沙箱可写 |
| 不借用执行器的权限写文件 | 请求目录对沙箱可写，因此一律以目录 fd + `O_NOFOLLOW` 访问：符号链接的请求目录、符号链接的文件、被换成普通文件的等待通道都不生效 |
| 缩权不受限 | 停止与回收永远是允许的动作，不需要 policy 登记 |
| 平台 | POSIX；Windows 只允许 Git Bash / MSYS2 的 POSIX Python，见 §7 |

## 3. 配置（三步）

1. 生成样板：`just runner-policy`，同时刷新下面的样板文件。
2. 复制为 `~/.akasha-agent-runner/policy.json`，按需删减动作：

   ```json
   {
     "script_sha256": "<脚本当前哈希，由第 1 步生成>",
     "workdir": "/Users/firefly/akasha",
     "idle_timeout_sec": 900,
     "max_lifetime_sec": 14400,
     "action_timeout_sec": 1800,
     "max_concurrent_actions": 4,
     "result_ttl_sec": 3600,
     "actions": {
       "check": ["/Users/firefly/.local/share/mise/shims/just", "check"],
       "test":  ["/Users/firefly/.local/share/mise/shims/just", "test"],
       "dev":   ["/Users/firefly/.local/share/mise/shims/just", "dev"]
     }
   }
   ```

   `max_concurrent_actions` 与 `result_ttl_sec` 可省略，默认 4 与 3600。
   机器生成的样板见 [`agent-runner.policy.json`](./agent-runner.policy.json)。
3. 启动一次，需要一次提权：`just runner-start`。沙箱内它会失败于 openpty，该失败就是提权的依据。

改过 `scripts/agent-runner.py` 之后必须重做第 1、2 步：脚本哈希不符时执行器拒绝启动。

## 4. 使用

### 4.1 同步（默认入口）

```sh
just runner-run check          # 提交并阻塞到结束；退出码 = 动作退出码
```

动作的输出与执行器的日志都在运行时目录的 `out.log` 里，`runner-run` 结束时会附上末尾数行。

### 4.2 异步 + 显式等待（非轮询）

```sh
just runner-submit test        # 立刻返回，打印 request=<id>
just runner-wait <id>          # 阻塞等待；已结束则立即返回，可重复等待
just runner-result <id>        # 不阻塞，看一眼（未结束退出码 4）
```

等待由 FIFO 等待通道与 `select` 完成：内核唤醒，不轮询文件系统，也不需要在客户端另写检测
脚本。等待方被中断（例如前台调用超时）不影响动作本身，结果留在请求目录里，重新 `runner-wait`
即可取回。

### 4.3 状态与回收

```sh
just runner-status             # 授权清单、执行器状态、在飞请求与最近完成
just runner-stop-dev           # 只回收 app（dev），保留执行器
just runner-stop               # 停止执行器并回收它名下的全部进程组
```

### 4.4 请求协议（MCP 形态必须满足同一语义）

客户端在 `req/<id>/` 下依次放置三样东西，顺序固定：

1. `wait`：等待通道（FIFO，客户端创建）；
2. `action`：动作名，单行；
3. `ready`：空文件。**只有它出现，请求才算完整**，执行器在此之前不接纳。

执行器收尾时先原子写 `result.json`，再往 `wait` 写一行并关闭。等待方先以 `O_NONBLOCK` 打开
`wait` 的读端，再读 `result.json`，结果先落盘与读者先就位两种次序都不会漏唤醒。

`status` 取值：`done` / `timeout` / `rejected` / `busy` / `error` / `orphaned` / `aborted`。
只有 `done` 与 `timeout` 的 `rc` 是动作自己的退出码，被信号结束时记为负数。
请求目录默认保留 `result_ttl_sec`，之后由执行器清理；执行器每次启动也会清理一次。

## 5. 生命周期

执行器不 `setsid`、不 double-fork，随启动它的后台作业存活；下列任一条件命中即退出，
退出路径统一回收它名下的进程组，并给每个等待方写下 `aborted` 结局：

- 父进程消失（含 DSH 被 `kill -9`）；
- 空闲超过 `idle_timeout_sec`（dev 或动作运行期间不计入空闲）；
- 存活超过 `max_lifetime_sec`；
- 收到 `stop` 控制字，或 SIGTERM / SIGINT / SIGHUP。

动作不阻塞主循环：每个动作以 `setpgid` 自成进程组，主循环以 `WNOHANG` 轮询它的结局。
`stop` 在任何时刻都有效，包括动作已经无响应的时候（此时点名 SIGKILL 它的进程组）。
单动作另有墙钟上限 `action_timeout_sec`；同时在飞的动作数上限是 `max_concurrent_actions`，
超出即回 `busy`，不排队；排队会让等待方拿到别人的结局。

dev 另有一层监护进程：执行器被 `kill -9` 时，监护进程杀掉 dev 进程组并把退出码留给下一次
启动；下一次启动还会回收上一次留下的孤儿进程组，并把对应请求标记为 `orphaned`，
使等待方不必等到自己的超时。

## 6. 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `AKASHA_AGENT_RUNNER_POLICY` | `~/.akasha-agent-runner/policy.json` | 能力表位置，**必须在工作区与临时目录之外** |
| `AKASHA_AGENT_RUNNER_HOME` | `/tmp/akasha-agent-runner` | 运行时目录，存请求、状态、日志与 pid，权限 0700 |

## 7. MCP 形态与能力表

**规则：同名 MCP 优先。** MCP 工具列表里出现 `akasha-agent-runner`，或出现提供下表能力的执行器
工具时，一律先用 MCP；`scripts/agent-runner.py` 是它的降级路径。理由：MCP 形态不需要一次提权
去启动常驻进程，也没有文件通道；它的能力表由 MCP 配置持有，而脚本形态的能力表是工作区之外的
policy 文件。

下表是本机制的能力契约。两种形态的实现可以不同：脚本用 FIFO + `select` 唤醒，MCP 直接用
工具调用返回；但每一行的语义必须一致：

| 能力 | 语义契约 | 脚本形态 | MCP 形态 |
|---|---|---|---|
| run | 提交并阻塞到结束，退出码 = 动作退出码，附输出末尾 | `runner-run` | 同一语义 |
| submit | 只提交，返回请求 id | `runner-submit` | 同一语义 |
| wait | 阻塞等待指定请求结束；已结束立即返回；可重复等待 | `runner-wait` | 同一语义 |
| result | 非阻塞读结果（status 与 rc） | `runner-result` | 同一语义 |
| status | 能力清单、执行器状态、在飞请求 | `runner-status` | 同一语义 |
| stop_dev | 只回收 app 的进程组 | `runner-stop-dev` | 同一语义 |
| stop | 停止执行器并回收它名下的全部进程组 | `runner-stop` | 同一语义 |
| policy | 能力表在工作区之外，脚本哈希受 pin | `runner-policy` | 由 MCP 配置提供 |
| 生命周期 | 属主死亡 / 空闲上限 / 最长存活 / 单动作上限 / 并发上限 | §5 | 同一组语义 |
| 状态取值 | 见 §4.4 | §4.4 | 同一组取值 |
| 平台 | POSIX；Windows 仅 Git Bash / MSYS2 的 POSIX Python | 本文件 | 同 |

Windows：本机制依赖 FIFO、进程组、`select` 与 `dir_fd`，因此只支持 Git Bash / MSYS2 提供的
POSIX Python；PowerShell 与 cmd 不受支持，原生 Python 会在启动前被拒绝并说明原因。请在 Git Bash
中执行 `just runner-start` 等 runner 配方；根 `justfile` 的 `set shell` 就是 bash。

## 8. 已知边界

- 动作表里的 `just test` 与 `just dev` 会编译并执行工作区内的代码，因此"能改仓库"等于
  "能让这些配方执行任意代码"。这是"自动化构建与测试"的固有性质，不由执行器引入：
  `justfile` 与 policy 属于同级可信输入。只保留 `dev` 可把暴露面缩到"启动应用"。
- 执行器以当前用户身份运行，其边界是"拒绝任意命令注入"，不构成对恶意项目的隔离。
- 孤儿回收按「登记过的进程组 + 属主已死」判定：进程号被复用的极端情况下会错误回收无关进程组。
  执行器每次启动先清理过期请求，把这一窗口限制在最近一段时间内。
- 运行时目录对沙箱可写，被约束方因此可以拒绝服务，例如删掉等待通道、伪造请求、填满并发上限，
  但拿不到任何新的执行能力：动作名必须已登记，且所有写入都不跟随符号链接。
- 等待方有自己的时间预算，即 DSH 前台调用上限 600s。长动作应走 `runner-submit` +
  `runner-wait`，不应把同步等待拉到超过该上限。
