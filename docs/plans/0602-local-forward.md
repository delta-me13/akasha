# Plan 0602: 本地转发 `-L`

- **关联**：ROADMAP 阶段 6 ·「本地转发 `-L`（复用阶段 5 的 `direct-tcpip`）」
- **前置**：plan 0601（隧道实体与状态机）· plan 0505（原语）
- **状态**：进行中

## 目标

本地监听端口 → 经 SSH 连接上的 `direct-tcpip` channel → 远端目标。

这是三处复用**同一个原语**的第三处（前两处：跳板、SFTP B 档）——
本 plan **只做接线**，不重写原语：搬运交给 `copy_bidirectional`，一条流就是全部形状。

`target_host` / `target_port` **到本 plan 才第一次参与**：plan 0601 的隧道只连规则所属主机。

## 非目标

- 动态转发（plan 0603）与远程转发（plan 0604）：**只实现 `local` 一个方向**，
  另外两个方向在命令边界上明确拒绝，而不是静默按 `-L` 处理
- 重连（plan 0605）：连接中断即进入「失败」，不由本 plan 循环重试
- 关闭语义的完整形态（plan 0606）：本 plan 只做到"停止后端口释放、在途连接被回收"
- 规则池的增删改界面：规则仍由测试直接写库（同 plan 0601）

## 先定死的三件事

1. **先绑定，后连接。** 端口被占用是本类功能最常见的一类失败，而用户此时**还不该**
   回答任何凭据询问 —— 绑定先失败就先报错，握手与提问都不发生。
2. **目标地址由对端解析。** 送给 `direct_tcpip` 的是规则里的原字符串，
   不在本机解析（本机解析等于绕开跳板机）；拿不到目标地址的路由 → 明确报错。
3. **绑定失败 = 没登记成。** 与「登记了但连不上」（plan 0601 的 `Ok(TunnelAttempt)`）
   分开：端口被占用不是"这条隧道失败了"，而是"这条隧道根本没起来" ——
   池里那条规则与主机都没问题，重试也不会好，用户要动的是端口。

## 步骤（每步都能独立验证）

1. **`akasha-ssh` 新增 `relay` 模块**（纯 SSH 侧，无 Tauri）
   - `LocalListener::bind(host, port)`：绑定 + 记住实际地址；空绑定地址明确拒绝。
   - `LocalListener::serve(runtime, connection, target) -> LocalForward`：
     在**给定 runtime** 上起一条任务（库不自建 runtime，ADR-0003 D2）——
     接受循环 + 每条入站连接一条 `direct_tcpip` 通道 + `copy_bidirectional`。
   - `LocalForward::shutdown()`：发信号即返回（收尾在 runtime 上做：停止接受、
     回收在途任务、**礼貌断开**那条连接）。sender 一 drop 也走同一条路。
   - 验证：`just test`，新增 relay 用例（端口 0 由内核分配 / 端口被占用报可读错误 /
     空绑定地址被拒）。
2. **crate 层用例：接上真服务端**（`akasha-ssh/tests/local_forward.rs`）
   - 测试进程内起一台 SSH 服务端（`testing`）+ 一个回声服务端；客户端经本地端口读写回声；
     断言服务端的 `direct_tcpip` 请求与中继字节数；停止后端口释放、连接断开。
   - 验证：`just test`（`local_forward` 目标）。
3. **app 侧：隧道实体由"持有连接"改为"持有转发"**（`src-tauri/src/tunnel.rs`）
   - `Tunnel.forward: Option<LocalForward>`；probe `tunnels` 每条多一个 `bind`
     （实际监听地址 —— "有没有端口在听"是这条功能的唯一对外事实）。
   - `tunnel_open`：读规则 → 校验方向 → **绑定** → 登记（`连接中` 事件）→ 连接 →
     起转发 → `已连接`；绑定失败 → `Err(TunnelError::Bind)`，**不登记**。
   - `tunnel_retry`：先停止上一次的转发 → 重读规则（端口 / 目标可能已被改）→ 绑定 →
     `连接中` → 重连。
   - `tunnel_stop` 与 `Sessions::shutdown_all`：停止转发（停止监听 + 断开连接）。
   - 验证：`just test`（`Sessions` 既有用例不许破：`len()` 与 `registered()` 仍相等）。
4. **前端：两档新失败文案**（功能验证壳层）
   - `TunnelFailed` 增加 `bind`（端口被占用一类）与 `unsupported`（方向不是 `-L`）。
   - 验证：`pnpm build`（`TunnelError` 的穷尽 `switch` 少一档即编译不过）。
5. **E2E `tunnel_local_forward`**（真实 app + 测试进程内的服务端）
   - 见「验收命令」。新增 `tests/*.rs` 必须登记进 `src-tauri/justfile` 的 `E2E_TARGETS`。
6. **文档同步**：本 plan 置「已完成」并 `git mv` 进 `archive/`（索引与 ROADMAP 指针同步）；
   ADR-0003 D9 标注第三个消费者已落地 + §14 记一行；`docs/STATUS.md` 覆盖写。

## 验收命令（可直接粘贴执行，并写出预期输出）

```bash
# 1. crate 层：relay 自身 + 接上真服务端的一条
just test          # 预期：退出码 0；新增 akasha-ssh 的 relay 用例与 local_forward 目标全绿

# 2. 门禁（格式 / lint / 全量测试 / 依赖 / 生成物已提交 / 文档）
just ready         # 预期：6/6 全部通过，退出码 0（TunnelError 变了，生成物必须已提交）

# 3. 真实 app 上的验收（自包含：没有 app 就自己起一套）
just test-e2e      # 预期：退出码 0；清单含新增的 tunnel_local_forward

# 4. 前端类型（不在 `just ready` 内）
pnpm build         # 预期：退出码 0
```

真实路径的对照（Victauri，E2E 用例逐条做同一件事）：

| 断言 | 手段 | 预期 |
|---|---|---|
| **判据：转发端口可访问远端服务** | 从测试进程连本地监听端口，写一行再读回 | 读到的就是写进去的那一行（远端回声服务答的） |
| 目标**只可能**经隧道到达 | 目标名 `akasha-e2e-forward.invalid`（RFC 2606 保留域） | 用例自行解析一次并断言**失败** |
| 走的是 `direct-tcpip`（不是别的路） | 服务端的 `direct_tcpip` 请求表 | 恰好 1 条，`host` / `port` 与规则一致 |
| 每条入站连接各开一条通道 | 同一端口再连一次 | 请求数变成 2 |
| 字节真的过了 SSH 通道 | 服务端的 `relayed_bytes` | `> 0` |
| 监听地址可见 | `app_state { probe: "tunnels" }` | 该条带 `bind = "127.0.0.1:<绑定端口>"` |
| **端口被占用的报错可读** | 界面点"打开"那条端口被占的规则 | `.tunnel-failure` 含该地址与非空原因；probe 里**没有**它（没登记成） |
| 停止后端口释放 | 点"停止" → 再连该端口 | 连接被拒（监听已撤） |
| 连接真的断了 | 服务端的 `connections_closed` | `≥ 1` |

## 回滚

- 代码：新增集中在 `akasha-ssh/src/relay.rs` 与 `tests/local_forward.rs`；
  对既有路径的改动是三处 —— `Tunnel` 的字段（`connection` → `forward`）、
  `tunnel_open` 的顺序（绑定提前）、前端两档文案。回退即恢复这三处。
- 数据：**不涉及格式变更**（`forwards` 表的列早已存在，没有新表、没有迁移）。
- 命名：`TunnelError` 新增两个变体进入 `src/ipc/bindings.ts`，与状态名同一条纪律
  （ADR-0003 §10 第 4 条）—— 改名要连同前端与生成物一起改。

## 实施记录（边做边追加）
