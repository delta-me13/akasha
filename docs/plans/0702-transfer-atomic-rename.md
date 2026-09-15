# Plan 0702: local ↔ host 传输（临时名 + 原子重命名）

- **关联**：ROADMAP 阶段 7 ·「local ↔ host 双向；**临时名 + 原子重命名**落盘」
- **前置**：plan 0701（双栏与两侧连接）；ADR-0006 D4（传输引擎只认两个端点）
- **状态**：进行中

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
#   ── 取消之后：目标目录里 {…} 个条目（既没有 big.bin，也没有临时名）
#   ── 上传之后：对端盘上 big.bin 的字节数与源一致

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

## 实施记录

（边做边追加；每条记**实际输出**，不记预期）
