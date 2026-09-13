# Plan 0501: 写 ADR-0003（SSH 栈与资源模型）

- **关联**：ROADMAP 阶段 5 ·「ADR-0003 进入实现中」
- **前置**：无（但它是 plan 0502 起的**前置**）
- **状态**：已完成（2026-09-12）
- **影响面**：`docs/adr/0003-*.md`（新增）、`docs/adr/README.md`（状态）
- **后继**：plan 0502 – 0505、阶段 6 全部（它们从本 ADR 取实现约束）

## 目标

把 SSH 这一层的**线协议与资源模型**定案：改它要重写一整层，所以必须在写代码前定死。

- `russh` 的**版本与 API 面**（以及为什么不用 `ssh2` / 系统 `ssh`）
- `Transport` 在 SSH 上的映射：能力差异怎么表达（无本地进程语义、无退出码）
- **连接 = 拥有它的 `Session` 的生命周期**；关 Session 立刻断连，连带中止重连与传输
- **不复用连接**（一实体一连接）及其副作用：内存凭据缓存 / agent 优先 / known_hosts 缓存
- `direct-tcpip` **原语只实现一次**，供三处复用（跳板 / SFTP B 档 / 本地转发）
- 隧道实体与状态机（`连接中 / 已连接 / 重连中(n) / 失败 / 已停止`，状态变化发事件）
- 重连：**3 次 + 指数退避**（参数可配置，有默认值）

## 非目标

- **不**写实现代码（plan 0502 起）
- **不**定 SFTP 传输拓扑与断点续传（前者在 `scope.md` §4.1 已定且**可逆**，后者是 `later`）
- **不**定存储与机密（ADR-0002）
- **不**展开 `~/.ssh/config` 的完整解析（已定案为受限子集 + 显式报错）

## 前置检查

```bash
sed -n '/## 2\. /,/## 3\. /p' docs/scope.md     # 后端、SSH、端口转发的已定案条目
sed -n '/### 5.1/,/### 5.2/p' docs/scope.md     # Session 模型
sed -n '1,60p' docs/adr/README.md               # 门槛与队列
cargo search russh 2>/dev/null | head -3         # 记录调研时的版本（可联网）
```

## 步骤

1. **调研先行**：确认 `russh` 当前版本、`direct-tcpip` / `tcpip-forward` / `forwarded-tcpip`
   的实际 API 形态；把调研结论（含版本号与链接）写进 ADR 的「事实依据」一节 ——
   否则 ADR 会退化成愿望清单。
2. 逐条把 `scope.md` §2.1 / §2.2 / §5.1 的**已定案**转成决策条目（决定 / 理由 / 否决的替代路）。
3. 补上还没有值的部分：`russh` 具体版本与 feature、认证优先级（agent 还是密钥池）、
   内存凭据缓存的键与失效条件、隧道状态机的转移表、退避序列的具体取值。
4. 写明不可逆点：连接模型与 `Transport` 映射一旦被代码固化，改动会波及 IPC 类型与前端。
5. `docs/adr/README.md` 里 0003 状态改为「**实现中**」（三态见 [`../../adr/README.md`](../../adr/README.md)）——
   实现完成后再转「已定案」。

## 验收命令

```bash
ls docs/adr/0003-*.md
grep -n '状态' docs/adr/0003-*.md           # 期望：实现中（Implementing，<日期>）
grep -n 'russh' docs/adr/0003-*.md | head   # 期望：出现**带版本号**的结论，不是泛指
just docs-check                             # 期望退出码 0
just ready                                  # 期望退出码 0
```

**判据**：ADR 覆盖 `scope.md` §2.1 / §2.2 / §5.1 的每一条已定案，
且「事实依据」一节有**可核对的版本号**（不是"据我所知"）。

## 回滚

三态见 [`../../adr/README.md`](../../adr/README.md)：进入「实现中」后仍可就地修订（记修订行），
「已定案」后只能由新 ADR 取代。回滚 = 删除该文件。

## 实施记录

**调研方式**（结论见 ADR-0003 §2，每一条都带出处）：`crates.io` 的 API + 从
`static.crates.io` 下载的 `russh-0.63.3` 源码（引用的行号是解包后的路径，可逐条核对）；
本机侧读 `Cargo.lock` 与 `rustc --version`。**没有**靠"据我所知"写版本号。

**量到的关键事实**：`russh = 0.63.3`（2026-09-09，Apache-2.0，edition 2024，
`rust-version = 1.89`，本机 1.98.1）；crypto 后端 `ring` / `aws-lc-rs` 二选一必开
（`src/lib.rs:90` 的 `compile_error!`），而 `ring` 要的 `0.17.14` **正是** lock 里已有的那个
（`rustls` / `quinn-proto` / `reqwest` 带入）；上游把 `ssh-key` 钉在**预发布** `=0.7.0-rc.11`；
上游**没有**任何重连设施。API 侧逐条核对了 `channel_open_direct_tcpip`、
`Channel::into_stream()`（`impl AsyncRead + AsyncWrite`）、`tcpip_forward` +
`server_channel_open_forwarded_tcpip(…, reply)`、`impl Signer for AgentClient<R>`、
`check_server_key` 默认拒绝一切、`Config` 默认 `keepalive_interval = None`。

**被否决的备选**（连同理由留在 ADR §3–§8 里）：`aws-lc-rs`（cmake / bindgen / NASM 的交叉
编译代价）、库内自建 runtime（drop 语义与测试隔离）、为跳板单写一条连接路径（三处各写一遍）、
连接复用 / multiplexing（"随机关掉别人的隧道"）、`learn_known_hosts` 直接写用户的文件。

**验收命令的实际输出**：

```text
$ ls docs/adr/0003-*.md
docs/adr/0003-ssh-stack-and-resource-model.md
$ grep -n '状态' docs/adr/0003-*.md | head -1
3:- **状态**：**实现中**（Implementing，2026-09-12）
$ grep -c 'russh' docs/adr/0003-*.md
11            # 且出现的是 `=0.63.3` 这类**带版本号**的结论（D1）
```

**留给后续 plan 的**（ADR §12 的清单，逐条已挂到具体 plan）：`TransportError` 的背压变体与
`transport.rs:47` 注释的修改、`keepalive` 的取值 → plan 0502；隧道状态机的转移表 → 0601；
库内 known_hosts 表（要动 `user_version` 迁移）→ 0503。
