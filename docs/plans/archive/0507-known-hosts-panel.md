# Plan 0507: 主机指纹的可视与删除

- **关联**：ROADMAP 阶段 5 ·「主机指纹在前端可视与可删除」
- **前置**：plan 0503（known_hosts 校验与缓存，已完成）· ADR-0003 **D11**（已定案）
- **状态**：已完成（2026-09-27）
- **影响面**：`src-tauri/src/store/ipc/vault.rs`、`src-tauri/src/lib.rs`、
  `src/ipc/bindings.ts`（生成物）、`src/ipc/knownHosts.ts`（新）、
  `src/ssh/KnownHostsPanel.tsx`（新）、`src/App.tsx`、`src/tabs/TabStrip.tsx`、`src/App.css`、
  `src-tauri/tests/known_hosts_manage.rs`（新 E2E 目标）、`src-tauri/justfile`（`E2E_TARGETS`）、
  `docs/scope.md`、`docs/STATUS.md`、`ROADMAP.md`、`docs/plans/README.md`

## 目标

界面能**看**、能**删**我们记下来的主机密钥；**添加与修改只发生在连接过程中**。

1. **看**：一行一条，至少给出 `name` / `host` / 指纹（另给端口与算法）；
2. **删**：删掉一条 = 遗忘 —— 下次连同一台会**重新询问**；
3. **添加**：未知密钥 → 现有的询问（接受即写入 `known_hosts` 缓存）；
4. **修改**：密钥变化 → 按 D11 **拒绝并提示**（两个指纹都在）；用户在面板里删掉旧行、重连、
   确认新的 —— 触发点在连接过程，动作只有"删 + 确认"两种。

列表只覆盖**我们库里的缓存**，不读也不写用户的 `~/.ssh/known_hosts`（D11：那份文件只读）。

判据（来自 ROADMAP）：**面板里看得到、删得掉，且删除之后下一次连接重新询问**。

## 非目标

- **不做新增 / 编辑指纹的入口**：`store::pools::known_hosts::remember` 遇到同一
  `(host, port, key_type)` 上是另一把密钥时返回 `Conflict`，改密钥必须先 `forget`
  —— 界面不得绕过这一步（D11 的"既不静默接受、也不静默改写"）。
- **不改 D11 的判定顺序与"变化即拒绝"那半句**。若产品要求"连接过程中直接替换旧密钥"
  （即把变化那一问也变成可确认的询问），那要先有**新 ADR** 取代 D11 的这一条：
  `HostKeyChanged` 今天不带 `PromptId`，往返形状也要跟着改。**本 plan 不做。**
- **不动库格式**：`known_hosts` 表今天是 `id / host / port / key_type / key_blob / fingerprint`
  （`store/schema.rs`），没有 `name` 也没有时间列。`name` 由 `hosts` 池按 `(host, port)` 关联得到，
  不新增列、不迁移。
  ⚠️ `hosts.name` 唯一而 `(host, port)` **不唯一**（同一台机器可以有两行、名字不同），
  所以关联结果是一个**列表**；池里没有对应行时（例如那台被删了）界面显示"主机池里没有这一台"。
- **不做事件推送**：面板在打开与删除之后刷新，另给一个"刷新"按钮。新增事件就是新增契约，
  而这条需求不需要它。
- **不做视觉打磨**（`AGENTS.md` §4.0）：它是功能验证壳层里的一块。

## 前置检查

```bash
grep -n "pub fn known_hosts\|pub fn forget" src-tauri/src/store/pools/known_hosts.rs
grep -n "CREATE TABLE known_hosts" -A 9 src-tauri/src/store/schema.rs
grep -n "connect_connection" src-tauri/src/tunnel/ipc.rs
grep -n "KnownHostsVerifier::new" src-tauri/src/ssh/ipc.rs
```

- `store` 侧已经有 `known_hosts(conn)`（整表读出）与 `forget(conn, id)`（按 id 删）
  —— 本 plan 只加 IPC 与界面，判定与写路径一行不动；
- `known_hosts` 表没有 `name` 列（`store/schema.rs` 的 DDL）；
- **隧道的连接与终端会话走同一个校验器**：`tunnel/ipc.rs` 的 `connect_once` 调
  `crate::ssh::connect_connection`，那里构造 `KnownHostsVerifier` 并接上提问
  —— 所以"添加与修改在隧道连接过程中按需触发"这条**天然成立**，不需要新造一条路。

## 步骤（每步都能独立验证）

1. **IPC 两条命令**（`src-tauri/src/store/ipc/vault.rs`，与 `vault_hosts` 同一条借库路径）：
   - `known_hosts_list() -> Vec<KnownHostEntry>`；
   - `known_hosts_forget(id: i64) -> ()`。

   两者都经 `Vault::with_conn`：库锁着时给 `VaultError::Locked`（界面据此说"先解锁"）。
   `KnownHostEntry` 的字段：`id` / `names: Vec<String>` / `host` / `port` / `key_type` / `fingerprint`。
   ⚠️ **`key_blob` 不出 IPC** —— 它是判定材料，给人核对的是指纹。
   名字用一条 `LEFT JOIN`（`hosts.host = known_hosts.host AND hosts.port = known_hosts.port`）
   取回，在 Rust 侧把同一 `id` 的多行折成一个列表并按名字排序。
   命令名不叫 `vault_*`：按 D11 它是**缓存**，不是第 5 套池（与 `import_ssh_config` 同类，
   用动作命名）。
   验证：`cargo nextest run --lib` 与下面的 E2E 目标都能编译并执行通过。
2. **注册命令**并执行 `just gen-types`，把 `src/ipc/bindings.ts` 的差异一起提交（`AGENTS.md` §5）。
   验证：`just gen-types-check` 无差异。
3. **前端包装** `src/ipc/knownHosts.ts`：`listKnownHosts()` / `forgetKnownHost(id)`，
   错误翻译沿用 `src/ipc/hosts.ts` 的形状（锁着 → `isLocked` → 界面说"先解锁"）。
   前端不出现裸命令名（§0 禁止 #1）。
4. **面板** `src/ssh/KnownHostsPanel.tsx`（与 `TunnelPanel` / `BwPanel` 同类：不是标签页，
   关闭它不改变任何后端状态）：
   - 打开时读一次；每行显示名字列表（空则"主机池里没有这一台"）、`host:port`、算法、指纹；
   - 每行一个"删除"，删完就地提示"已删除：下次连接会重新询问"并刷新；
   - 顶部一句说明：这里只有我们记下来的指纹，用户的 `~/.ssh/known_hosts` 只读、不在列表里；
   - **没有任何新增 / 编辑控件**（非目标那两条的界面表现）。
5. **入口**：`src/tabs/TabStrip.tsx` 加一个 `onNewKnownHosts` 按钮（与 `onNewTunnel` /
   `onNewSftp` / `onNewBitwarden` 那一排同一处），`App.tsx` 加对应的状态位并挂载面板，
   `App.css` 复用现有面板样式。
6. **E2E 目标** `src-tauri/tests/known_hosts_manage.rs`，并登记进 `src-tauri/justfile` 的
   `E2E_TARGETS`（漏登记会被开头的守卫判红，问题 #170）：
   - 空缓存的库上开一条连接 → 在界面上回答 hostKey 询问"接受" → 面板里出现那一行
     （名字 = 池里那台的名字，指纹 = 服务端那把，逐字相等）；
   - 点"删除" → 该行从界面与 `store::known_hosts` 里都消失；
   - **同一条连接再来一次 → 询问再次出现**（这就是"删除 = 遗忘"的判据，正反例成对）；
   - 负例：面板里**不存在**"信任新密钥"这类入口（`eval_js` 断言没有对应选择器）；
     与它对应的行为是"变化即拒绝"，由 crate 层已有用例守着（两个指纹都在、不发起询问）。
7. **文档**：`docs/scope.md` §5.6 的视图清单加一行（指纹视图 = 仅渲染 + 显式删除动作，
   关闭它不改后端状态）；`docs/STATUS.md` 更新读数，并把「主机池的增删改查仍无界面」
   那条的边界写清：**指纹这一块的看与删有了**，主机池的增删改仍然没有。

## 验收命令

```bash
# 1) 真实 app 上的端到端（沙箱外：pty 设备与 ~/.cargo 都在工作区之外）
just runner-run test-e2e
#    期望：退出码 0；新目标 known_hosts_manage 全过（询问 → 面板出现 → 删除 → 询问再现）

# 2) 生成物与代码都在门禁内
just gen-types            # 期望：bindings.ts 出现 known_hosts_list / known_hosts_forget
just runner-run ready     # 期望：六步全绿（fmt / lint / test / deny / gen-types / docs）
```

- 手测（可选，只在真实 app 上）：`just dev` → 打开面板 → 连一台已知主机并接受询问 →
  面板里多一行 → 删除 → 再连一次，询问再现。

## 回滚

两个命令、一个面板、一个 E2E 目标可以整条 `git revert`；库格式没有改动，因此没有迁移要回滚。
生成物 `src/ipc/bindings.ts` 的差异随同一次 revert 消失。

## 实施记录

**2026-09-27（macOS 26.6.2 / arm64；`just test-e2e` 经 `just runner-run` 在沙箱外执行）**

1. **两条命令**落在 `src-tauri/src/store/ipc/pools.rs`（与 `vault_hosts` 同一条借库路径）。
   ⚠️ **与本文「影响面」的一处偏离**：那里写的是 `store/ipc/vault.rs`，而池的读取都在
   `pools.rs` —— `known_hosts_list` 要与 `hosts::hosts` 并读（名字是 `LEFT JOIN` 的等价物），
   放进 `pools.rs` 才不必把主机池的读取从一个不相干的模块里再引一次。
   `KnownHostEntry`（`id` / `names` / `host` / `port` / `keyType` / `fingerprint`）**不带 `key_blob`**；
   `names` 是列表（`hosts.name` 唯一而 `(host, port)` 不唯一），池里没有对应行时为空。
2. `bindings.rs` 登记两条命令 + `just gen-types`：生成物新增 `knownHostsList` / `knownHostsForget`
   与 `KnownHostEntry`（41 行）。`just gen-types-check` 通过。
3. 前端：`src/ipc/knownHosts.ts`（`listKnownHosts` / `forgetKnownHost`，错误翻译照 `hosts.ts` 的形状）、
   `src/ssh/KnownHostsPanel.tsx`（打开时读一次 + 删除后刷新 + 一个刷新按钮；**没有**新增 / 编辑控件）、
   `TabStrip` 的 `.tab-new-known-hosts` 入口、`App.tsx` 的挂载点、`App.css` 的面板样式。
   前端 `pnpm build`（tsc + vite）通过：**894.71 kB / gzip 246.23 kB**（+8.27 kB）。
4. **读数**（`just test-e2e`，新目标 `known_hosts_manage`）：`1 passed; 0 failed`（2.37 s）。
   日志里的四行观测：
   * `提示: 第一次连接=["hostKey:SHA256:…", "credential:e2e@127.0.0.1:57633"]`
   * `面板: 行数=1 名字=e2e-known-hosts-target 指纹=SHA256:…`（与 `server.fingerprint` 逐字相等）
   * `删除: 面板行数=0 库侧=无那条记录`
   * `提示: 第二次连接=["hostKey:SHA256:…"]` —— **重新询问**成立；口令那一步走的是内存缓存
     （问题 #124），没有第二次询问凭据。
   负例在同一目标里：面板中除 `known-hosts-forget` / `known-hosts-refresh` / `known-hosts-close`
   之外没有任何按钮 / 输入控件（`eval_js` 的 `every(...)` 断言）。
   ⚠️ 该次运行的整体退出码仍是 1：`ssh_session` / `ssh_jump` 的回声判据超时（问题 #183，
   原始工作区同样复现），与本 plan 无关。
5. 文档：`docs/scope.md` §5.6 的视图表加一行（指纹视图 = 仅渲染 + 显式删除动作），并补一条
   "添加与修改只发生在连接过程中"；`docs/STATUS.md` 更新读数与「主机池的增删改查仍无界面」那条的
   边界（**指纹的看与删有了**，主机池的增删改仍然没有）。
