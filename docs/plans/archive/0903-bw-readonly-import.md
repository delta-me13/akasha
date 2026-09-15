# Plan 0903: 只读导入 SSH key 条目

- **关联**：ROADMAP 阶段 9 ·「只读导入 SSH key 条目（`sshKey.privateKey`）」
- **前置**：plan 0902（CLI 能用了）· plan 0905（拿得到 session key）· 阶段 4（本地密钥池）· [ADR-0002](../adr/0002-secret-storage.md) D10
- **状态**：进行中（2026-09-15）

## 目标

把 Bitwarden 的 SSH key 条目**只读导入**本地池：**不写回上游**（v1 只读，`scope.md` §7）。

`scope.md` §7 的两条要一起落地，缺一条这次导入就不完整：

- 「性质 = 独立导入池，**与本地密钥池之间没有任何同步机制**」→ 入库的是一个**快照**：
  私钥落进密钥池（受保护页那一条路不变），另外记一行**来历**（上游条目 id、`revisionDate`、
  `fingerprint`）。来历表就是 `scope.md` §7 的「导入池」，也是 plan 0904 的缓存。
- 「导入池 = 可导入、可删、可改备注」→ 删除走 `keys` 行（来历行随外键一起走）。

## 非目标

- 写回 Bitwarden（`later`，`scope.md` §10）。
- 其他条目类型（登录 / 笔记 / 卡……）—— 只做 `type = 5`。
- 私钥落盘成文件：导入后私钥**存库**（阶段 4），**不落临时文件**。
- 自动刷新与后台轮询：刷新判据与自检是 plan 0904。
- **在本地实现 vault 密码学**（`no`）：解密由 `bw` 自己做，我们只读它给的 JSON。

## 形状（落地时定下的四件事）

| 决定 | 取值 | 为什么是它 |
|---|---|---|
| 调用形态 | `bw list items --raw` | `bw get item <id>` 需要先知道 id，而唯一的枚举入口就是 `list`；`list` 没有"按类型过滤"的开关（`--help` 只有 folder / collection / organization / search / trash / archived） |
| 筛选 | `type == 5`（`CipherType.SshKey`）且 `sshKey.privateKey` 非空 | 上游 2026.2.0 的实现里 `SshKeyExport.toView` 对三个字段都是"缺一即抛"，所以空私钥那一档只能由我们**跳过并说明**，不能让整批失败 |
| 名字 | 上游条目名（`name`） | 与 `~/.ssh/config` 导入同一口径：同名默认**不动**，`overwrite` 才整行替换 |
| 来历 | `bw_items`（cipher_id / name / revision_date / fingerprint / key_id） | 快照必须能回答"这一行从哪个条目来、上游改过没有"——`keys` 是**本地**池（`scope.md` §7：与 Bitwarden 无关、永远可用），来历不塞进它 |

### ⚠️ 整个 vault 的明文会经过本进程一次

`bw list items --raw` 交出的是**解密后**的全部条目（含每一处登录口令）。上游 CLI 的接口里
没有"只列 SSH key"这条路，所以这一趟无法避免。落点与边界：

- 只保留 `type == 5` 的那几条；其余字段**不反序列化到我们的类型里**（serde 的未知字段直接丢）。
- 那段 stdout 包在 `zeroize::Zeroizing` 里，读完即擦零；**不落盘、不进日志、不进事件载荷**
  （`AGENTS.md` §3.4 / ADR-0002 D13）。
- 私钥到手立刻进受保护页（`keys::PrivateKey::new`），超长（> 16 KiB）与空白**当场拒绝**。
  照实记：serde 为被丢弃的字段临时分配的缓冲不受我们控制，这一段擦不掉。

### ⚠️ 导入的私钥怎么被用上（这一步不改 0506 的语义）

`hosts.key_id` 是主机引用钥匙的唯一方式，而 `~/.ssh/config` 导入**不导入私钥**
（plan 0506），所以导入进来的条目原本没有任何主机引用它——"导入后可用该密钥建立 SSH 连接"
这条判据会停在"库里多了一行"。桥是**逐字符相同**的一条规则，不改 0506 的任何判断：

- `IdentityFile` 的 **basename** 与池里某把钥匙的 `name` 完全相同时，把 `key_id` 接上，
  报告里出一条 note；
- 不相同则**行为与今天一致**（`key_id` 留空 = 走 ssh-agent，逐条出既有的那条警告）。

不匹配、不做模糊比较：`IdentityFile` 里写的是路径，池里存的是名字，两者之间只有"同名"
这一条可靠的关系；按后缀 / 前缀猜会把条目连到**另一把**钥匙上。

## 步骤

1. **库格式 v3**（`src-tauri/crates/akasha-store/src/schema.rs`）：加 `bw_items` 表 +
   `migrate_step` 的 `2 => 3` 分支 + `TABLES` 加一项。列：
   `cipher_id TEXT PRIMARY KEY` · `name TEXT NOT NULL` · `revision_date TEXT NOT NULL` ·
   `fingerprint TEXT NOT NULL` · `key_id INTEGER NOT NULL UNIQUE REFERENCES keys(id) ON DELETE CASCADE`。
   `ON DELETE CASCADE`：删掉池里那行钥匙，来历行随之消失（导入池没有"孤儿"这种状态）。
2. **解析**（`src-tauri/crates/akasha-bw/src/items.rs`，纯函数）：`bw list items --raw` 的字节
   → 上游给的总条数 + 其中 `type == 5` 的条目。`fingerprint` 两版字段名都认
   （上游实现里导出模型叫 `keyFingerprint`、SDK 与官方文档叫 `fingerprint`）。
3. **取数**（`src-tauri/crates/akasha-bw/src/cli.rs`）：`Cli::items(&mut Session) -> Vec<u8>`
   （原始字节，解析在上面那个纯函数里）。同时补 `BwError::Locked`：上游原文
   `Vault is locked.` —— 它此前会落进 `CommandFailed`，而"先解锁"与"看那句原话"是两个动作。
4. **落库**（`akasha-store/src/pools/keys.rs`）：`import_snapshot(conn, items, overwrite)`，
   **一次事务**：新建或替换 `keys` 行 + 写 `bw_items` 行。同名跳过（`Reason::Exists`）。
5. **接线**（`src-tauri/src/bitwarden.rs`）：`bw_import_keys(overwrite) -> BwImportReport`。
   需要 vault 解锁（要写库）+ session（要读 vault）。两把锁**不同时持有**：先在 `bw` 那把锁里
   执行完 `list items` 并解析成我们自己的类型，放锁之后才去动库 —— 反向（持库锁起进程）会让
   一次 `bw` 调用把整个库锁住几百毫秒。
6. **界面**（`src/ipc/bitwarden.ts` + `src/bitwarden/BwPanel.tsx`）：一个"从 Bitwarden 导入
   SSH 密钥"按钮 + 一份报告（导入了几条 / 跳过了哪几条、为什么）。选择器 `data-bw-import*`。
7. **E2E**（`src-tauri/tests/bw_import.rs`）：假 `bw` 的 `list items --raw` 交出一份**真**的
   ed25519 私钥 → 导入 → 报告与池里都对得上 → 用 `~/.ssh/config` 把主机指到它 →
   `open_ssh_session` **真的连上进程内服务端**，且服务端记下的指纹与导入时存的一致。

## 验收命令

```bash
# 1) crate 层：解析（纯函数）+ 落库（真库、一次事务）+ 库格式迁移 2 → 3
cargo nextest run --package akasha-bw --package akasha-store
# 预期：全绿；含"type 不是 5 的条目一条都不进"、"私钥为空的那条被跳过而不是整批失败"、
#       "v2 库打开后变成 v3 且原有四类池一行不少"

# 2) 门禁与前端构建
just ready && pnpm build
# 预期：just ready 6/6；pnpm build 退出码 0

# 3) 端到端
just test-e2e
# 预期：exit 0，且 ── E2E: bw_import ── 打印导入报告与"已连接"，
#       服务端 observed.offered_keys 里那把钥匙的指纹 == 我们在库里存下的 fingerprint
```

## 回滚

- 摘掉 `bw_import_keys` 与面板按钮即可：库里的 `bw_items` 行只是来历，`keys` 行本身照常可用。
- 库格式**不退回去**（v3 是加法，v2 → v3 的迁移只加表；退回 v2 需要显式删表，不提供）。

## 实施记录

### 落地时改过口径的三处

1. **多了一档 `Skip::Claimed`**：`bw_items.key_id` 是 `UNIQUE`，而"池里那个名字已经归另一条
   上游条目了"是**默认行为之外**的一种情形（上游允许两条条目同名）。第一版直接顶掉它，
   结果撞上约束、整批失败 —— 而静默让后来者赢更坏（先导入的那条会从"有来历"变成"没来历"，
   用户看不到任何提示）。现在它是**跳过**，并在报告里说清下一步（改上游的条目名）。
   用例：`bw_import_pool.rs` 的 `two_items_with_the_same_name_in_one_batch_do_not_overwrite_each_other`
   与 `an_existing_name_is_left_alone_until_overwrite`。
2. **`IdentityFile` 那句警告改写**：原文是"没有 agent 就先把它加进密钥池，再改这一行"——
   接上池里同名钥匙这条规则落地之后，它在**已经接上**的情形里是错的。新措辞把两种情形都说清，
   而"究竟哪一种发生了"由导入报告里那条 `linked` 说明（`pools.rs` 的 `linked_notes`）。
3. **宿主引用钥匙那一半属于本条**：ROADMAP 的判据是"导入后可用该密钥建立 SSH 连接"，
   而在只读导入完成之前，池里的钥匙**没有任何主机能引用它**。所以 `import.rs` 加了那条
   逐字符同名的规则，`hosts_import.rs` 补了正反例（同名接上 / 不同名与 0506 行为一致）。

### 验收命令的实际输出

```console
$ just test
Summary [15.195s] 471 tests run: 471 passed, 0 skipped     # 本轮之前是 452

$ just test-e2e          # ── E2E: bw_import ──
测试服务端 127.0.0.1:33657（主机密钥 SHA256:6Q5pP6IF…）；客户端那把一次性钥匙的指纹是 SHA256:Guft+2mY0CgrRlKCCrc5C5a5gw8gVnVPXO6EHRgkwzU
没登录就导入：还没有登录这个 vault：先登录（登录过就先解锁）
面板上的导入报告：上游给了 2 条，其中 SSH 密钥 1 条； 新增 1 条、替换 0 条、跳过 0 条。akasha-e2e-bw-key · SHA256:Guft+2mY…
导入 ssh_config：{"created":[{"id":1,"name":"akasha-e2e-bw-host"}],…}
提示问答（按发生顺序）：["hostKey:SHA256:6Q5pP6IF…"]
服务端：methods=["publickey"] offered_keys=["SHA256:Guft+2mY0CgrRlKCCrc5C5a5gw8gVnVPXO6EHRgkwzU"]
test result: ok. 1 passed; 0 failed（29.88 s）
```

判据那一行是最后两行：**服务端看到的指纹**与**导入时存下来的指纹**逐字符相同 ——
"连上了"与"用的是这把钥匙"是两件事，只断前者的话，一条走 ssh-agent 的会话也能过。

### 与本节推断不一致的地方

- 假 `bw` 的 `list items` 用**引号包住的 heredoc**（`<<'JSON'`）交出来：整份 JSON 原样进 stdout，
  里面没有一处会被 shell 解释。第一版用 `printf '%s'` 内联，私钥里的 `\n` 与 `$` 都要自己处理，
  而"这份 JSON 恰好没被 shell 改过"是个不该由人来维持的前提。
- E2E 里 `tabs` 是 **2** 而不是 1：app 起来时就有一个本地终端标签页，SSH 会话是第二个
  （`is_connected` 数的是全部标签页）。这条在 `ssh_session` 一类的用例里已经有先例，
  第一版按 1 写，症状是"等到超时也说没连上"，而服务端已经收到了认证。

