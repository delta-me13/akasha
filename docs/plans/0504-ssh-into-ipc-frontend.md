# Plan 0504: SSH 接进 IPC / 前端

- **关联**：ROADMAP 阶段 5 ·「SSH 接进 IPC / 前端」
- **前置**：plan 0502（连接 + 认证在 crate 层成立）· plan 0503（host key 的信任策略已定，
  未知 host key 是"问"而不是"临时接受"）
- **状态**：未规划（骨架）
- **展开时机**：plan 0503 之后 —— 在那之前，真连接没有可信的 host key 策略，
  接了也只能自造一套临时的

## 目标

让**当前运行的这个 app** 能开一个 SSH 会话：从界面选一个主机 → 后端连上 → 终端里字节能双向流。
今天 `akasha-ssh` 已经能连能认证，但**没有任何命令或界面碰得到它**（plan 0502 的判据只到 crate 层）。

按 `src-tauri/src/session.rs` 的现状，缺的是三样：

1. **一条带目标的命令**：`open_session` 今天只会 `PtyTransport::spawn_default()`。
   `Sessions::register` 已经是 `<T: Transport + 'static>`，所以 `SshTransport` 能进同一个注册表 ——
   要加的只是"按目标选载体"这一段（ADR-0003 D3 的落点）。
2. **一条"后端问 → 前端答"的往返协议**：凭据（口令 / 私钥口令 / keyboard-interactive）与
   未知 host key，都是**后端在连接中途需要用户回答**。事件 + 命令成对，且必须有超时与取消 ——
   前端走了不能让连接线程永远卡着。
3. **一个提示界面**：验证壳层级别即可（`AGENTS.md` §4.0）—— 判据是"这条后端行为能被验证"，
   不是"好不好看"。

## 非目标

- **正式 UI**：没有设计稿（`scope.md` §1.3）。这里只做能被 E2E 驱动的提示与标签页。
- **跳板 / 转发 / SFTP**：它们是 plan 0505 与阶段 6 / 7 的事。
- **连接复用**：`scope.md` §2.2 已定案不做到同一主机的复用 —— 本 plan 只保证
  "三个会话只问一次凭据"在**真 app 上**也成立（靠 0502 的内存缓存）。
- **主机池的增删改查界面**：池的 CRUD 在阶段 4 已落地，本 plan 只消费它。

## 判据（来自 ROADMAP，展开时必须变成可粘贴的验收命令）

真 app 上开一个 SSH 会话：字节能双向流（敲命令看到输出）、凭据只问一次、关标签页零残留。

⚠️ 它需要**一台真的 SSH 服务端**：本机的 `/usr/bin/sshd`，或一个由测试进程自己起的服务端。
"怎么准备目标"是展开时第一件要定下来的事 —— 判据的可复现性全押在它上面。

## 展开时要补

- [ ] 「## 步骤」：命令与事件的形状（目标怎么给、问题怎么问、答案怎么回、超时多长）
- [ ] 「## 验收命令」：一条真路径的 E2E（Victauri `invoke_command` + `wait_for`，禁止 sleep）
      与 `introspect { action: "processes" }` 的零残留确认
- [ ] 与 plan 0505 的接口对齐：`SshTransport` 现在把 `Handle` 交给那条 task（收尾的唯一出口），
      而 D9 的原语要 `direct_tcpip(&Handle, …)` —— 谁持 `Handle`，本 plan 定形状
- [ ] **库那一侧的适配器**（plan 0503 留下的口子）：`KnownHostsVerifier` 要一个
      `'static` 的 `HostKeyCache`，而今天 `Vault { unlocked: Mutex<Option<Unlocked>> }`
      借不出这种句柄（`State<'_, Vault>` 拿不到 `Arc`）。要先把库连接做成可共享的句柄
      （`Sessions` 就是这么做的），再接 `akasha_store::known_hosts` 的两个函数。
      ⚠️ 凭证那条路的阻塞问题也一并解决：`CredentialProvider` / `HostKeyPrompt` 都是**同步** trait，
      而它们在 `russh` 的 async 回调里被调用 —— 要保证这条路上还有富余的 worker。
