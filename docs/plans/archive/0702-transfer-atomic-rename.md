# Plan 0702: local ↔ host 传输（临时名 + 原子重命名）

- **关联**：ROADMAP 阶段 7 ·「local ↔ host 双向；**临时名 + 原子重命名**落盘」
- **前置**：plan 0701（双栏与两侧连接）；ADR-0006 D4（传输引擎只认两个端点）
- **状态**：已完成（2026-09-15）

## 目标

本机与远端**双向**搬文件，落盘一律**先写临时名，完成后原子重命名**（`scope.md` §4.2）：

| 时机 | 行为 |
|---|---|
| 传输中 | 写到目标目录下的临时名（`.name.part`） |
| 成功 | 原子 `rename` 成最终名 |
| 失败 / 取消 / **关闭 Session** | **删除临时文件** |

关键不变量：**用户看到最终名就等于成功** —— 未完成的文件不得以最终名存在。

## 非目标

- 断点续传（`later`）：需要记录偏移量与远端校验，不是顺带可实现的功能
- host ↔ host（plan 0703）：本条命令**明确拒绝**两栏都是主机，避免把它当成"已经做了"
- 并发 in-flight（plan 0704）：本阶段一次一个文件，引擎的接口先留出不冲突的形状

## 步骤

1. **引擎与接口**（`akasha-ssh/src/transfer.rs`）：`Endpoint`（`list` / `open_read` / `begin_write`）、
   `PendingWrite`（`write_chunk` / `commit` / `abort`）、`Cancel`、`Progress`，以及把源端点的
   一个文件搬进目标端点的 `transfer()`。引擎不认识 SSH、SFTP 与本机路径（ADR-0006 D4）。
   - 临时名与原子重命名归**目标端点**；引擎的任何退出路径都要么 `commit`、要么 `abort`
2. **本机端点**（`akasha-ssh/src/local.rs`）：`std::fs` + `tokio::fs`；
   临时名 `.name.part`（已被占用时退到 `.name.N.part`，见 D6 的唯一性要求）
3. **远端端点**（`akasha-ssh/src/sftp.rs`）：`SftpClient` 补 `open_read` / `begin_write`
   （`open_with_flags` + `rename` + `remove_file`），并实现 `Endpoint`
4. **证据链**：测试服务端的 SFTP 根改成**真实目录**（`testing.rs`），否则"文件真的落到对端盘上"
   只能靠客户端自己说。已实现的动作扩到 `open` / `read` / `write` / `rename` / `remove` / `stat`
5. **库内用例**（`akasha-ssh/tests/transfer_atomic.rs`）：确定性中断（一个"读到第一块就阻塞
   在等待上"的假端点 + 取消）、读失败后的清理、成功路径的字节与目录状态
6. **IPC 与实体**（`src-tauri/src/sftp.rs`）：`SftpOrigin`（本机 / 池里的主机）、一次传输的
   `Tracked` 状态、三条命令 `sftp_transfer` / `sftp_transfer_cancel` / `sftp_transfers`，
   探针 `sftp` 带上传输列表；`sftp_close` 变成 async —— **先取消并等清理落地，再断连接**
7. **前端壳层**（`src/sftp/SftpPanel.tsx`）：栏里多一个"本机"选项、路径输入、每个文件一条
   「传到对侧」、传输列表（进度 + 取消）
8. **E2E**（`src-tauri/tests/sftp_transfer_atomic.rs`）：见下

## 验收命令

```bash
# 1. 库那一层：确定性中断（不靠 sleep —— 假端点读到第一块就等测试放行）
just test transfer_atomic
# 预期：transfer_* 全绿；其中 cancel 那条断言目标目录**只有**源文件没有的东西

# 2. 端到端：真 app + 进程内 SFTP 服务端（根目录是真实临时目录）
just test-e2e
# 预期（sftp_transfer_atomic 那一段）：退出码 0，且打印
#   取消之前：搬了 N / 4194304 字节，目标目录 [".big.bin.part"]
#   取消之后：目标目录里 0 个条目（既没有 big.bin，也没有临时名）
#   上传之后：对端盘上 small.bin 的字节数与源一致

# 3. 门禁
just ready
# 预期：fmt-check / lint / test / deny-offline / gen-types-check / docs-check 全绿
```

判据的读数口（E2E 与手查都用）：

```bash
# app 在跑时（Victauri MCP）：传输的状态与字节数
#   app_state { probe: "sftp" }  →  [{ handle, sides:[…], transfers:[{ id, state, done, total }] }]
```

## 回滚

新增文件（`transfer.rs` / `local.rs` / 两条测试）整份删除即可；改动面是 `sftp.rs` 的两个方法、
`testing.rs` 的 `SftpRoot`、`src-tauri/src/sftp.rs` 与面板。IPC 多出三条命令与一个 `origin`
参数 —— 生成物 `src/ipc/bindings.ts` 必须一起回滚（`gen-types-check` 会拦住不一致）。

## 实施记录（2026-09-15）

### 判据实测

库内（`just test transfer_atomic`，6 条，连续执行 8 次全绿）：成功那条用两个**可控的假端点**把
"传到哪一步"变成可以等待的事件 —— 假目标端点在建好临时文件之后、以及在**端点收下第一块
之后**各报一次信号，假源端点在交出第一块之后停住。实测：那一刻目标目录里**只有**
`.payload.bin.part`，它的内容是**源文件的一段前缀**；放行之后最终名的字节与源逐字节相同、
临时名消失。取消那条把源永久停住，取消之后目标目录**一个条目都没有**；
写失败那条在第 2 次写入报错，同样一个条目都没有。

端到端（`just test-e2e`，退出码 0，22 个目标 / 29 个用例）：

```
服务端 127.0.0.1:43153 · 根目录 /tmp/akasha-sftp-3949-0
本机目标目录 /tmp/akasha-e2e-transfer-3949-0
取消之前：搬了 2883441 / 4194304 字节，目标目录 [".big.bin.part"]
取消之后：目标目录里 0 个条目（既没有 big.bin，也没有临时名）
上传之后：对端盘上 small.bin 的字节数与源一致
```

`just ready` **6/6 全绿**；`just test` **349 passed**（`akasha` 78 + `akasha-core` 30 +
`akasha-pty` 39 + `akasha-ssh` 75 + `akasha-store` 127）；
`pnpm build` 退出码 0（868.97 kB / gzip 239.10 kB）。

### 引擎的最终形状（0703 / 0704 从这里接）

`akasha-ssh::transfer`：`Endpoint`（`list` / `open_read` / `begin_write`）、
`PendingWrite`（`write_chunk` / `commit` / `abort`）、`Cancel`、`Progress`、
`transfer(source, target, request, progress, cancel)`。三条与计划一致的关键约定：

1. 端点的动作用 `BoxFuture`（trait 里的 `async fn` 不是 dyn 兼容的，而两端在运行期才决定
   是本机还是某台主机）—— 一个类型别名，不新增依赖；
2. **临时名由引擎这一层定规则**（`.name.part`，被占用时退到 `.name.N.part`），
   **由端点挑第一个不存在的**：D6 的唯一性要求因此落在接口上而没有落进某一种端点里；
3. `SftpClient` 与 `LocalEndpoint` 都实现同一个 `Endpoint`，于是 `src-tauri` 那一侧
   "一侧一个端点"只有一处代码（`Sftp::endpoint`）。

### 与 ADR-0006 的偏差（已记进那份 ADR 的修订记录）

| 偏差 | 为什么 |
|---|---|
| 新增 `SshError::File { path, reason }` 一档 | D4 说引擎不重写端点的失败，于是"哪一个文件"必须由端点自己带上；`Sftp` 那一档说的是会话，用它报"重命名失败"会把人引到对端的 `sshd_config` 上 |
| `sftp_connect` 的 `hostId` 变成 `origin`（本机 / 主机） | 一栏现在可以对着**本机**，而"池里的第 N 行"表达不了它 |
| 多出 `sftp_transfer` / `sftp_transfer_cancel` / `sftp_transfers` | D7 那张表是 plan 0701 写的，当时还没有传输 |

### 落地时红出来的四处（都不是笔误）

0. **"写成功"不等于"已落盘"，而第一版判据把它当成了同一件事**：`tokio::fs::File` 的
   `poll_write` 在**派发**阻塞写之后就返回 `Ready`，真正的 `write(2)` 要等下一次 poll ——
   于是"端点收下第一块之后，临时文件里正好是一块"这句话是错的（实测见过 0 字节，
   而 `just ready` 第二次执行时红的就是它）。判据改成"临时文件里只可能是源文件的**一段前缀**"，
   并把这条语义写进 `PendingWrite::write_chunk` 的文档：**全部落地由 `commit` 保证**。

1. **`Cancel` 推信号会静默丢**：`watch::Sender::send` 在**一个订阅者都没有**时直接返回
   `Err` 并且不写入那个值 —— "先推信号、后拿等待端"这条路上的取消于是丢失，表现是
   `sftp_close` 之后传输永远不停。改成 `send_replace`。它由那条"先推后等"的单元用例红出来。
2. **测试服务端把普通文件的类型位抹掉**：`FileAttributes::set_dir(false)` 做的是
   `permissions &= !DIR`，而 `REG` / `LNK` / `DIR` 的位互相重叠 —— 把三个都设一遍会
   得到 `Other`。只设命中的那一个。
3. **`opendir` / `open` 的响应 id 写死 0**：上游服务端分发是 `Ok(packet) => packet.into()`，
   请求号**取自 handler 的返回值**。表现是"服务端答了、客户端等满期限"。

### 遗留

- **host ↔ host 明确拒绝**（`SftpError::Unsupported`）：那是 plan 0703，两栏都是主机时
  本阶段不做 —— 让一条更慢的路径悄悄顶上会让人以为 0703 已经完成。
- **一次一个文件**：D6 的"同一批里两个同名文件不撞在同一个临时名上"由候选名的占用探测
  满足了，但并发 in-flight 本身是 plan 0704。占用探测与创建之间不是原子的（没有用
  `CREATE|EXCLUDE`）—— 0704 要认真做并发时得重新回答这一点。
- **不做 `fsync`**：判据是"用户看到最终名就等于成功"，断电后的落盘持久性不在其中
  （远端那一侧的 `fsync@openssh.com` 同样没做，见 ADR-0006 §4）。
- **传输结束后目标那一栏不自动刷新**：面板要用户再点一次「前往」才看得到新文件。
  它是壳层的取舍（`AGENTS.md` §4.0），不是后端的行为。
