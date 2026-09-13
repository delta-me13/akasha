# Plan 0503: known_hosts 校验与缓存

- **关联**：ROADMAP 阶段 5 ·「known_hosts 校验与缓存」
- **前置**：plan 0502（连接建立路径已存在）· ADR-0003 **D11**（策略已定死）
- **状态**：已完成（2026-09-13）
- **后继**：plan 0504（把"未知 host key 问用户"这一段接到前端）

## 目标

按 **D11** 落地信任策略，并使其形状可由 plan 0504 消费：

| 情形 | 行为 |
|---|---|
| 我们的库里有这台主机的同类型密钥，且一致 | 连 |
| 库里有记录，**不匹配** | **拒绝并提示**（带记录值与实际值两个指纹）—— 不静默接受、不静默改写 |
| 库里没有 → 用户的 `~/.ssh/known_hosts` 里有且一致 | 连（**只读**，不得写入用户的文件） |
| 用户文件里有、不匹配 | 拒绝（指出在哪一行） |
| 两边都没有（**未知**） | 交给调用方提问；无人提问 / 用户否认 → 明确报错，**不得默认接受** |
| 用户确认了未知密钥 | 记入**我们的库**（`user_version` 2 的 `known_hosts` 表），下次不再询问 |

## 非目标

- **接前端**：提问是注入的（`HostKeyPrompt`），本 plan 只提供 trait 与测试桩；
  真实 app 上的界面与往返协议属于 **plan 0504**。
- 写入用户的 `~/.ssh/known_hosts`：不写入（D11 已否决 `learn_known_hosts`）。
- 哈希主机名（`|1|…`）/ `@cert-authority`：读取复用上游 `check_known_hosts_path`
  （该函数支持哈希主机名），但**我们自己的表**只按明文 `host:port` 记录。
- 指纹的 UI（列表页 / 删除一项的界面）：池层的操作在本 plan 内实现，界面归 plan 0504。

## 前置检查

```bash
cd src-tauri && cargo tree -p akasha-store | grep -c sqlcipher   # 1（库还是那个库）
cd src-tauri && cargo nextest run -p akasha-store                # 先确认基线是绿的
```

## 决策（展开时定下来的形状）

1. **库格式升到 v2 = 表与 `user_version` 一并变更**（ADR-0002 D7 说"改列就是 v2 加迁移"）。
   本仓库的**第一次迁移**，规则一次定死，后续 v3 沿用同一形状：
   - `schema.rs` 里 **v1 的 DDL 冻结为 `DDL_V1`**（它是"v1 是什么"的定义，迁移测试依赖它构造真实 v1 库）；
   - `TABLES_V1`（4 张）/ `TABLES`（5 张，当前格式）；
   - `open` 的顺序：解锁 → **按库中写入的版本**校验形状 → 逐步迁移 → 按当前版本校验形状；
   - 迁移在**一次事务**内完成（含写 `user_version`）：失败即回滚为 v1，不产生部分迁移的库；
   - `> FORMAT_VERSION` 仍然拒绝（新程序写入的库，旧程序不得猜测）；`0` 仍然拒绝。
2. **迁移在 `open` 内自动执行**（不询问用户）。理由：`vault_unlock` 是 app 唯一的开门路径，
   不自动迁移将使旧库无法打开，而拒绝是 D7 留给**降级**的处置，不适用于升级。
   **代价（如实记录）**：**迁移后旧版本程序打不开这个库**（`UnsupportedVersion { found: 2 }`），
   且 `open` 从此可能**写**文件（只读挂载上的 v1 库会报错，错误信息说明"需要升级但写不进去"）。
3. **不做自动备份**：事务回滚是失败时的保障；升级前的副本通过已有的加密导出获得（plan 0404）。
   向数据目录写入 `*.bak` 会改变 `portable.md` §3.1 的清单，该变更应单独讨论。
4. **表结构**（`known_hosts`，属缓存而非池）：主机密钥是**公开**信息，本层不含机密。

   | 列 | 说明 |
   |---|---|
   | `host` / `port` | 目标（明文，不存哈希主机名 —— 哈希形式由用户文件承担） |
   | `key_type` | `ssh-ed25519` 一类 |
   | `key_blob` | SSH 线格式的密钥本体（**判定的材料**：逐字节比较，不比指纹文本） |
   | `fingerprint` | `SHA256:…`，给人核对用 |
   | `UNIQUE (host, port, key_type)` | 同一主机同一类型只记录一个；不同类型各记一行 |

5. **三态判定 + 两个注入点**（与 0502 的 `CredentialProvider` 同一模式）：
   - `HostKeyCache`：库内读写（`recorded` / `remember`），由调用方实现（0504 接库）；
   - `HostKeyPrompt`：未知密钥时向用户提问（`confirm`），由调用方实现（0504 接前端）；
   - 两者都是**同步** trait —— 与 0502 一样，实际代价（在 async 中阻塞）由 plan 0504 一并解决。
6. **错误分三支**（阶段 6 的重连判据要用）：`HostKeyChanged`（**永不重试**）、
   `HostKeyUnknown`（无人可问）、`HostKeyRejected`（被拒 / 用户否认）。
   `Handler` 中"只记指纹"的一处改为记录**整个错误**，三态原样向外传递。

## 步骤（每步都能独立验证）

1. **S1 v2 的表与版本**：`schema.rs` 拆 `DDL_V1` / `TABLES_V1` / `TABLES` / `DDL_V2`，
   `check(conn, version)` 按版本查表；`FORMAT_VERSION = 2`。→ 建库后 `known_hosts` 存在，形状快照通过。
2. **S2 迁移**：`schema::migrate_step(conn, from) -> Result<i64>` 与 `open` 内的逐步循环；
   迁移失败（不可写 / 无法迁移的版本）返回可读错误。→ 用 `DDL_V1` 构造真实 v1 库，写入一条 host 后打开。
3. **S3 `pools/known_hosts.rs`**：`list` / `lookup` / `remember` / `forget_host` / `clear`。
   `remember` 遇到**同键不同值**返回 `Conflict`（写路径上不得静默改写）。
4. **S4 `akasha-ssh` 的判定**：`HostKey` 带上 `key_blob`；新增 `known_hosts.rs`：
   `HostKeyCache` / `HostKeyPrompt` / `KnownHostsVerifier`；`SshError` 加两个变体。
5. **S5 判据测试**（进程内服务端 + 测试桩注入两个口）：见下。
6. **S6 门禁与文档同步**：`just ready` 6/6、plan 归档、ROADMAP 勾选、STATUS 覆盖写。

## 验收命令（可直接粘贴执行，并写出预期输出）

```bash
cd src-tauri

# ① 池层：表能读写，且**不许**静默改写密钥
cargo nextest run -p akasha-store --test known_hosts_roundtrip
# 预期：6 passed（回环+删除 / 同键幂等 / 同键不同值 → Conflict / 类型不同不是同一条 /
#         clear 只清缓存 / 库自己拦下端口 0 与重复键）

# ② 格式迁移：一个真 v1 库（`DDL_V1` 造出来的）开完之后是 v2，且**数据还在**
cargo nextest run -p akasha-store --test format_migration
# 预期：6 passed（升级+数据保留 / 再开一次不写文件 / 版本 0 拒绝 / 缺表的 v1 不迁移 /
#         加密导出件与明文导出件两条还原路：**升的是副本，来源一个字节不变**）

# ③ 形状快照：v1 与 v2 两组列都钉住
cargo nextest run -p akasha-store --test schema_contract
# 预期：11 passed（`the_shape_of_v1_is_pinned` 与 `the_shape_of_v2_is_pinned` 都在）

# ④ 判据本身：三态 + "确认过的不再问" + 用户文件只读
cargo nextest run -p akasha-ssh --test known_hosts
# 预期：7 passed
#   unknown_host_key_is_not_silently_accepted       —— 未知且没人可问 → HostKeyUnknown，认证一步没开始
#   a_confirmed_host_key_is_remembered_and_never_asked_again —— 确认 → 进缓存；第二个连接 **0 次提问**
#   a_declined_host_key_is_refused_and_not_recorded —— 否认 → 拒绝，缓存仍然空
#   a_changed_host_key_is_refused_without_asking    —— 记录对不上 → HostKeyChanged（两个指纹都在），且**不问**
#   a_matching_user_file_entry_is_accepted_and_the_file_is_never_written —— 文件里认 → 连上，文件**逐字节没变**
#   a_changed_user_file_entry_is_refused_and_names_the_line —— 文件里对不上 → 拒绝，带**真实**行号
#   the_vault_record_wins_over_the_user_file        —— 带一条对照：同一份文件在空缓存下会判成"变了"

# ⑤ 全量 + 门禁
cargo nextest run --workspace     # 预期：228 passed
cd .. && just ready               # 预期：6/6
```

> ⚠️ 按测试名过滤与用 `--test <目标名>` 的差别：`cargo nextest run -p … known_hosts`
> 会**一个都不匹配**（过滤器比的是测试函数名，而不是文件名），其表现为 `error: no tests to run`，
> 与"没有写测试"的现象相同。该现象曾在此处出现，已记入 `STATUS.md` 的已知问题。

## 回滚

- 代码：`git revert` 即可 —— 但**已经迁移过的库不会自动变回 v1**。
- 数据：迁移是单向的（旧版本程序会以 `UnsupportedVersion { found: 2 }` 拒绝 v2 库）。
  退回旧版本时，先用 0404 的导出留一份，或从备份 / 另一份 v1 库恢复；这条**如实记入 `STATUS.md`**。

## 实施记录

**已完成的工作**（每一步均可独立验证，验收命令见上）：

1. **库格式升到 v2**（`FORMAT_VERSION = 2`）：`schema.rs` 把 v1 的 DDL 冻结为 `DDL_V1`
   （公开 —— 迁移测试依赖它构造真实 v1 库），加 `TABLES_V1` / `TABLES`（5 张）与 `DDL_V2`；
   `check(conn, version)` 按**库中写入的版本**查表。
2. **迁移**（`lib.rs`）：`open` 的顺序 = 解锁 → 按库中写入的版本确认形状 → 逐步迁移（**一次事务**，
   `user_version` 与表一起提交）→ 按当前版本再确认形状。`> 2` 与 `0` 仍拒绝；
   缺表的 v1 **不迁移**（该库属于损坏库，不是待迁移库）。迁移失败返回 `UpgradeFailed`（含"文件不可写"那一支）。
3. **导出与还原**：`copy_tables` 复制的是**来源的**版本号（不是写死当前版本），
   `restore` 用 `open_unmigrated` 打开来源 —— 于是"还原一个 v1 导出件"会得到一个 v2 的库，
   而**用户的导出件一个字节未变**（对照：`open` 为迁移会写入该文件）。
4. **`known_hosts` 表与池操作**（`pools/known_hosts.rs`）：`UNIQUE (host, port, key_type)`；
   `remember` 遇到同键不同值返回 `Conflict`（**写路径上不得静默改写**，D11）；
   `lookup` / `list` / `forget` / `forget_host` / `clear`。
5. **三态判定**（`akasha-ssh::known_hosts`）：`KnownHostsVerifier` 按 **库 → 用户文件（只读）→ 提问**
   的顺序执行；两个注入点 `HostKeyCache` / `HostKeyPrompt`（与 0502 的 `CredentialProvider` 同一模式）。
   `HostKey` 多带一份**密钥本体**（判定材料），`Handler` 的拒绝记录由"指纹"改为**整个错误**，
   于是"未见过"与**已变化**不会被合并为同一结果。

**判据实测**（命令见上，全部按预期）：

| 检查 | 结果 |
|---|---|
| 池层 6 条 | ✅ 6 passed |
| 迁移 6 条 | ✅ 6 passed（含"再开一次不写文件"——`open` 只在需要升级时写） |
| 形状快照 11 条 | ✅ 11 passed（v1 与 v2 两组） |
| 三态 7 条 | ✅ 7 passed |
| `cargo nextest run --workspace` | ✅ **228 passed**（本轮 +20：store +13、ssh +7） |
| `just ready` | ✅ **6/6**（fmt / clippy / test / deny-offline / gen-types / docs-check） |

**与计划的偏差（如实记录）**：

- **行号由本实现自行计算**：上游 `Error::KeyChanged` 返回的 `line` 在跳过注释行时**不递增**
  （`russh-0.63.3/src/keys/known_hosts.rs` 的 `continue` 在 `line += 1` 之前，实测：
  注释行 + 记录行得到的是 `1` 而不是 `2`）。直接沿用会把用户指向错误的行，因此
  `true_line_of` 自行计数；`RecordedIn::UserFile` 的 `line` 因此是 `Option<usize>`
  （无法确定时只给出文件名，不推测数字）。**该上游行为已记入 `STATUS.md` 的已知问题。**
- **用户文件中无法解析的行**：`check_known_hosts_path` 遇到一行无效内容会对整份文件报错。
  本实现的处置是**按"未知"继续**（`warn` 后继续读取）—— 未知的后果是向用户提问，不是静默接受；
  若在此处拒绝，用户文件中任意一行异常记录都会使该主机不可连接。
- **未完成**（如实记录）：① 库那一侧的 `HostKeyCache` 适配器（`akasha-store::known_hosts` ↔
  verifier）要等 **plan 0504** —— 它需要 app 能把库连接做成 `'static` 的共享句柄，
  而当前 `Vault` 的 `Mutex<Option<Unlocked>>` 无法借出该句柄。该条已列入 0504 的清单。
  ② 只读介质上的 v1 库（迁移写不进去）只有错误分支，**没有实测**（需要构造一个真实只读文件系统）。
  ③ 与真实 OpenSSH 的 `known_hosts` 互操作（哈希主机名 / `@cert-authority`）没有实测 ——
  仅验证了"注释行不影响判定"这一条。
- **`restore` 不再调用 `open`**：ADR-0002 §6 在讲"还原时用**来源口令**打开导出件"那句里
  同时写了"走 `open`"。那条**决定**（用来源口令、拒绝同口令、替换而非合并）没有变，
  但字面上的那个函数名现在不成立了 —— 它改走 `open_unmigrated`（同一条解锁路径，
  去掉"自动迁移"那一步），否则还原一个 v1 导出件会**改写用户的导出件**。
  ADR-0002 已定案、**不可改**，所以这条偏差记在这里与 `docs/STATUS.md`。
- **ADR 无需改动**：策略由 ADR-0003 **D11** 预先定死；而"v2 加迁移"是 ADR-0002 **D7**
  预留的（"v1 一旦发布，改列就是 v2 加迁移"）。ADR-0002 已定案不可改，本次也没有需要它改的地方。
