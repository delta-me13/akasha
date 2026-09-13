# Plan 0502: `src-tauri/crates/akasha-ssh` —— 连接 + 认证

- **关联**：ROADMAP 阶段 5 ·「`src-tauri/crates/akasha-ssh`：连接 + 认证（密钥池 / agent / 内存凭据缓存）」
- **前置**：plan 0501（ADR-0003「实现中」）· 阶段 4（受保护页 `Protected` 与密钥池都在）
- **ADR**：[ADR-0003](../../adr/0003-ssh-stack-and-resource-model.md) D1–D4 / D7–D8 / D11；
  §12 挂给本 plan 的两条（`TransportError` 的背压变体、`transport.rs:47` 的注释）一并落地
- **状态**：已完成（2026-09-13）

## 目标

纯 Rust SSH（**不调系统 `ssh`**）：`akasha-ssh` 能**连上并认证**。认证来源按
**agent → 密钥池 → keyboard-interactive → password** 的顺序取用；同一台主机的第二次、第三次
连接不再询问凭据（内存缓存，不落盘）。

## 非目标

- 端口转发与 `direct-tcpip` 原语（plan 0505 / 阶段 6）
- `~/.ssh/config` 导入（plan 0506）与 known_hosts 的**持久化**（plan 0503）——
  本 plan 只留**接口**（`HostKeyVerifier`），策略与库表由那两份定
- SFTP（阶段 7）
- **把 SSH 会话接进 IPC / 前端**：本 plan **不接**；该缺口由 plan 0504 落地
- 断线重连与隧道状态机（阶段 6；本 plan 输出"握手 + 认证"的失败分类）

## 判据的范围与边界

判据原文为「同主机开三个 Session **只问一次**凭据」。本 plan 在 **crate 层**验证：**三个
`SshTransport`（= 三个 Session 各自的载体，D5 一实体一连接）连同一台主机，`CredentialProvider`
只被调用一次。**

- 测试目标是**进程内的 `russh` 服务端**（dev-dependency）：不需要 root、不需要外部 `sshd`、不写
  用户文件、三平台 CI 均可运行，且能**逐条记录服务端看到的认证方式**。
- ⚠️ 边界如实记录：该服务端由本仓库自行搭建，证明的是**客户端这条链**（顺序、缓存、失效、字节
  往返），**不是**与 OpenSSH 的互操作。互操作属真机实测，记入 `docs/STATUS.md` 的待验证；本
  plan 的用例通过不构成"能连接所有服务器"的结论。
- 接 IPC / 前端是**另一件事**：它需要一条"向后端询问凭据 → 前端作答"的往返协议与提示界面；该缺
  口在本 plan 结束时记入 `docs/STATUS.md`，其后由 **plan 0504** 落地（2026-09-13 重排：编号与执
  行顺序对齐）。

## 前置检查

```bash
cd src-tauri && cargo info russh | head -5          # 沙箱里可能被拒（问题 #11 提权）
grep -n '^name = "ring"' -A 1 Cargo.lock            # 0.17.14：选 ring 不新增 crypto 后端
```

## 步骤（每步都能独立验证）

- **S1 依赖与骨架**：`cargo new --lib src-tauri/crates/akasha-ssh`；
  `russh = { version = "=0.63.3", default-features = false, features = ["ring", "rsa"] }`
  （D1）。验收 = `cargo tree -p akasha-ssh | grep -c aws-lc` 为 **0**、`cargo check -p akasha-ssh` 绿。
- **S2 `akasha-pty` 的两条现状**（ADR §12）：加 `TransportError::Busy`（背压的唯一表达，
  `Unsupported` / `Closed` 都不适用），改 `transport.rs:47` 那条将"没有本地进程"与"没有结局"
  混成一句的注释。验收 = 两条各自的单测（新变体能被 `Display` 打出、能力位语义不变）。
- **S3 受保护页原语对外开放**：`akasha-store::protected` 提为 `pub`（`Protected` / `Exposed` /
  `PageError` + `Display`）—— 实现**只有一份**（理由见该模块文档）。
  验收 = `cargo nextest run -p akasha-store` 不退化。
- **S4 凭据缓存**（D8）：键 = `(host, port, user, 认证方式)`（私钥口令那一支另加**密钥标识**，
  因为同一台主机的两个密钥各有各的口令）；值为受保护页中的**口令**，不是私钥。三条失效：库锁定 /
  进程退出（`clear()`）、服务端拒绝（删去该条）、显式忘记（`forget()`）。**没有 TTL**。
  验收 = 单测覆盖命中 / 删一条 / 清空 + `VmLck` 随缓存建立与清空走一个来回
  （ADR-0002 D13 对每个新用途的要求）。
- **S5 握手 + 主机密钥**：以我们自己的 `TcpStream` 作为 `client::connect_stream` 的输入
  （便于设置超时）；`Handler::check_server_key` **强制覆写**（上游默认拒绝一切，D11）；
  `HostKeyVerifier` 是策略接口，本 plan 提供"钉住一个密钥"，known_hosts 由 plan 0503 落地。
  验收 = 钉对 → 连上；钉错 → 连不上且错误里带**指纹**。
- **S6 认证顺序**（D7）：agent（`SSH_AUTH_SOCK`，unix）→ 密钥池 → keyboard-interactive →
  password；`AuthResult::Failure { partial_success: true }` 时按 `remaining_methods`
  **续接而不从头来**。验收 = 服务端记录的认证方式序列逐条对上，且"agent 不可用"时**静默落到下一档**。
- **S7 同步 `Transport` 门面**（D3）：写 = `try_send` 入队（满 → `TransportError::Busy`，
  **不用 `blocking_send`**）；读 = 反方向队列 + `blocking_recv` 实现 `std::io::Read`，
  交给现有的 `akasha_pty::spawn_batcher`；`shutdown()` 显式收尾且幂等。
  capability = `resize + exit_status`，`session_leader() = None`（D4，看门狗不介入 SSH 这条路径）。
  验收 = 字节往返、`window_change` 到达服务端、结局（`ExitStatus`）取自 `ChannelMsg`。

## 验收命令（可直接粘贴执行，并写出预期输出）

```bash
just test                                  # 全 workspace；akasha-ssh 的用例在里面
cd src-tauri && cargo nextest run -p akasha-ssh   # 只跑本 crate（判据 4 条 + 顺序 3 条）
```

- `three_sessions_ask_for_one_credential` —— **本 plan 的判据**：三个连接、同一个 provider →
  `provider.calls() == 1`，且三个连接都能执行一条命令并取得输出。
- `a_rejected_password_is_forgotten_and_asked_again` —— 服务端第二次改为拒绝：缓存中的该条**被删除**，
  第三次连接**重新询问**（而不是以错误口令反复重试）。
- `forgetting_and_clearing_force_a_new_question` —— `forget()` / `clear()` 之后必然重新询问。
- `the_wire_shows_publickey_before_password` / `the_wire_shows_keyboard_interactive_before_password` /
  `an_unavailable_agent_falls_through` —— 服务端记录的认证序列；`SSH_AUTH_SOCK` 指向不存在的路径时，
  序列中不含 `publickey`（agent 一档静默跳过）。
- `a_wrong_host_key_is_rejected_with_its_fingerprint` —— 指纹不匹配时连接失败，错误中**带指纹**。
- `the_shell_round_trips_bytes_resize_and_exit_status` —— 字节往返、`window_change` 到达对端、
  `shutdown()` 取得 `ExitStatus` 且幂等、`session_leader()` 返回 `None`。
- `connecting_from_inside_a_runtime_is_an_error_not_a_panic` —— 在 tokio 上下文中调用同步门面
  得到 `BlockingInsideRuntime` 错误而非 panic。

> 上列用例名以实现记录中的名称为准：计划阶段曾使用另一组名称
> （`a_rejected_credential_is_forgotten`、`forgetting_is_explicit_and_complete`、
> `authentication_order_is_agent_keyboard_password`、`a_pinned_host_key_is_required`），
> 实现后按实际函数名更新。

```bash
just lint && just docs-check               # ast-grep / clippy / 文档纪律
just ready                                 # 6/6（含上面全部）
```

## 回滚

本 plan **只新增** crate 与依赖，不改既有行为（除 S2 的两处注释 / 变体）。
回滚 = 删 `src-tauri/crates/akasha-ssh/`、从 `Cargo.toml` 撤掉 `russh` / `tokio`、
还原 `TransportError` 与那条注释。**没有数据迁移**，`Cargo.lock` 的差异随之消失。

## 实施记录

**判据（crate 层）实测**（`cargo nextest run -p akasha-ssh`，2026-09-13）：

| 用例 | 结果 |
|---|---|
| `three_sessions_ask_for_one_credential` | ✅ **判据**：三个连接 → `provider.calls() == 1`、`cache.len() == 1`、服务端三次均收到同一口令，且每个连接都成功执行了一条命令（仅统计调用次数无法覆盖"缓存的凭据未被实际用于认证"的情形） |
| `a_rejected_password_is_forgotten_and_asked_again` | ✅ 服务端拒绝 → 缓存被清除 → 下一次**重新询问**（D8 失效条件 ②） |
| `forgetting_and_clearing_force_a_new_question` | ✅ 显式忘记 / 清空（③ 与 ①） |
| `the_wire_shows_publickey_before_password` | ✅ 服务端记录的序列 = `publickey, password`（D7 的顺序是**协议交互**层面的事实，不是内部实现顺序） |
| `the_wire_shows_keyboard_interactive_before_password` | ✅ `publickey, keyboard-interactive`，且 `passwords` 为空 |
| `an_unavailable_agent_falls_through` | ✅ agent 指向不存在的 socket → 协议交互中**没有** `publickey` 痕迹，直接落到口令 |
| `a_wrong_host_key_is_rejected_with_its_fingerprint` | ✅ 拒绝且错误中携带服务端真实指纹；**认证尚未开始** |
| `the_shell_round_trips_bytes_resize_and_exit_status` | ✅ 字节往返、`window:120x40` 回显（`window_change` 确实到达对端）、`shutdown()` 取得 `ExitStatus::Code(7)` 且**幂等**、`session_leader() == None` |
| `connecting_from_inside_a_runtime_is_an_error_not_a_panic` | ✅ 报 `BlockingInsideRuntime` |
| `credential_protection`（D13 第四次重验） | ✅ `VmLck` 0 → **32 kB**、`---p` 页数 **+8**；清空并释放后**两者都回到起点** |

`just ready` **6/6**（fmt-check / lint / test **208 passed** / deny-offline / gen-types-check / docs-check）。
逐 crate 计数：`akasha` 58 + `akasha-core` 15 + `akasha-pty` **39** + `akasha-store` 76 + **`akasha-ssh` 20**。

**依赖从"读源码核对"变为编译期事实**（ADR-0003 §2 的结论经实际编译复核）：
`cargo tree -p akasha-ssh | grep -c aws-lc` = **0**；`ring v0.17.14`（**唯一**版本，lock 里原有的那份）；
features 只有 `ring` / `rsa`。MSRV 无问题（上游要 1.89，本机 1.98.1）。

**偏离计划的地方（逐条给理由）**：

1. **`SshAuth` 多了 `agent_socket`**：Rust 2024 里 `std::env::set_var` 是 `unsafe`，而本仓库
   只允许 `akasha-store` 出现 `unsafe`（§3.4 + ast-grep 规则）—— 于是"agent 在哪"必须是
   **输入的一部分**。该形态另解决一个实际问题：多用户机器 / 容器里 `SSH_AUTH_SOCK` 未必是
   想要的那个。
2. **私钥口令的缓存键多了"哪个密钥"**：D8 写的是 `(host, port, user, 认证方式)`；实现时确认
   同一台主机的两个密钥各有各的口令，只按主机缓存会以错误口令解密另一个。
3. **"是否需要口令"按格式事实判断**，不靠错误类型穷举：PKCS#8 有专用头
   （`BEGIN ENCRYPTED PRIVATE KEY`），OpenSSH 新格式才看上游的 `KeyIsEncrypted`。
   PKCS#8 那条在"没给口令"时返回的只是一个解不开的 DER 错误，从类型上分不出来。
4. **keyboard-interactive 只在"一次只问一句"时用缓存**（多 prompt 是"用户名 + 口令"这类组合，
   用一句回答填所有格子必然出错），并给往返次数设上限 **8**（服务端可以一直发 `InfoRequest`，
   而这条路是长驻任务）。
5. **关通道的顺序 = EOF → 有期限读干 → CLOSE**：直接 `close()` 会把对端最后的 `exit-status`
   一并丢弃（实测：`shutdown()` 第一次取得 `None`）。该顺序属实现阶段发现的约束。
6. **受保护页原语公开**（`akasha-store::protected`）：SSH 的凭据使用该原语，不另写第二份
   `memsafe` 封装。`Passphrase` 的 `expose` 仍是 `pub(crate)` —— 公开的是原语，不是口令的通道。

**未完成的事项（交 `docs/STATUS.md`）**：与真 OpenSSH 的互操作、**真实 agent** 那条路
（只覆盖了"agent 不可用"）、"半开连接"的保活实测、并发未命中时的提问去重（single-flight）。
