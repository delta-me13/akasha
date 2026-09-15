# Plan 0904: 离线缓存（fingerprint 自检 / revisionDate 刷新）

- **关联**：ROADMAP 阶段 9 ·「离线缓存：**私钥离线自检用 `fingerprint`**，联网刷新用 `revisionDate`」
- **前置**：plan 0903（导入路径与来历表已存在）· [ADR-0002](../adr/0002-secret-storage.md) D10
- **状态**：已完成（2026-09-15）

## 目标

让"从 Bitwarden 导入进来的那一份"在**离线时**能被验证完好，在**联网时**能知道上游改过没有
（`scope.md` §7：缓存放同一个库、静态保护强度与本地池同级）。

两件事各有各的字段（推导见 [`../bitwarden.md`](../bitwarden.md) §5），**不许混用**：

| 字段 | 回答 | 要联网 | 落在哪 |
|---|---|---|---|
| `fingerprint` | 库里这份私钥还好吗？ | **否** | `bw_items.fingerprint`（导入时记下的上游值） |
| `revision_date` | 上游那一条变过吗？ | 是 | `bw_items.revision_date` |

## 非目标

- 自动刷新与后台轮询：这一版只做**用户触发的检查**，刷新是用户再点一次导入（`overwrite`）。
- 在本地实现 vault 密码学（`no`）。
- 二级加密：缓存就在那个 SQLCipher 库里（ADR-0002 D10），不另立一把口令。

## 形状（落地时定下的三件事）

1. **不需要新的库格式迁移**：plan 0903 的 `bw_items` 已经把三样都存下了
   （`cipher_id` / `revision_date` / `fingerprint`）、并用 `key_id` 指到池里那一行。
   本条只加**读**它们的两条命令。
2. **算指纹的地方在 `akasha-ssh`**：解析私钥要 `ssh_key`，而那个 crate 只在那里
   （`russh::keys`）。新增 `fingerprint_of_private_key(&[u8]) -> Result<String, SshError>` ——
   输入是**从受保护页借出来的那一段**，函数内部不留副本。
3. **失败逐条报，不整批失败**：一行校验不过（私钥读不出来 / 解析不了）不影响其余行 ——
   这正是"自检"该有的颗粒度（用户要知道**哪一把**坏了）。

## 步骤

1. **crate**（`akasha-ssh`）：`keys::fingerprint_of_private_key`。用 `decode_secret_key`
   解私钥 → `public_key().fingerprint(HashAlg::Sha256)`。解不开（带口令的私钥也算）报
   `SshError::PrivateKeyUnusable { reason }`，**不静默给一个空串**。
2. **命令一**（`bw_cache_verify`，**不联网、不起 `bw`**）：读 `bw_items` → 逐行从密钥池取私钥
   （受保护页那一条路）→ 算指纹 → 与记下的比。报告：`match` / `mismatch` / `unreadable`。
   需要库解锁（要读私钥），**不需要** session —— 这就是"断网也能用"的形态。
3. **命令二**（`bw_cache_check`）：`bw list items --raw`（要 session）→ 按 `cipher_id` 比
   `revisionDate`。报告逐条：`upToDate` / `changed` / `gone`，另外列出**上游有而缓存里没有**的
   条目名（"该导入一次"的提示）。**只报不改**：刷新是用户再点一次导入。
4. **界面**：导入那一块加两个按钮（"校验缓存" / "检查上游有没有变"）与一份报告，
   选择器 `[data-bw-cache-*]`。
5. **E2E**（`tests/bw_import.rs` 里那一串的续接）：导入之后自检 = 一致 → **把库里那把私钥
   换成另一把**（等价于"缓存被换过 / 坏了"）→ 自检必须报不一致（这条是诱饵：
   少了它，"自检"与"永远说好"分不开）→ 改假 `bw` 的 `revisionDate` → 检查上游报"变过"，
   而在改之前报"一致"。

## 验收命令

```bash
# 1) crate 层：指纹算法（正例 + 一把对不上的）
cargo nextest run --package akasha-ssh --package akasha-store
# 预期：全绿

# 2) 门禁与前端
just ready && pnpm build
# 预期：just ready 6/6

# 3) 端到端（真 app + 假 bw）
just test-e2e
# 预期：exit 0，且 ── E2E: bw_import ── 里：
#   自检 = 1 条一致；换掉私钥之后 = 1 条不一致；改 revisionDate 之后 = "上游变过"
```

## 回滚

- 摘掉两条命令与界面上那两个按钮即可：`bw_items` 那几行照旧只是来历，导入本身不受影响。

## 实施记录

### 落地时的实际输出

```console
$ just test-e2e          # ── E2E: bw_import ── 的第 8 / 9 段
面板上的缓存自检：缓存自检：1 条akasha-e2e-bw-key：完好（指纹与导入时一致） · SHA256:ybq+57zH…
换掉私钥之后的自检：…akasha-e2e-bw-key：对不上：库里这把私钥不是当初导入的那一把 · SHA256:tuEWnt2D…
面板上的上游比对：与上游比对：上游给了 2 条akasha-e2e-bw-key：没变
上游改过之后的比对：…akasha-e2e-bw-key：上游变过：重新导入一次就刷新了
test result: ok
```

四行读数就是两条判据的两半：**自检从"完好"翻成"对不上"**（换掉库里那把私钥之后算出来的
指纹确实变了 —— `SHA256:ybq+…` → `SHA256:tuEW…`），**比对从"没变"翻成"上游变过"**
（只改假 `bw` 的 `revisionDate`）。两处都留了诱饵：只断"报完好"或只断"报没变"的话，
一个永远返回固定答案的实现也能过。

### 与本节推断一致的两处

- **没有新的库格式迁移**：plan 0903 的 `bw_items` 已经把三样都存下了，本条只加了两条读命令
  （库格式仍是 v3）。
- **算指纹落在 `akasha-ssh`**：`fingerprint_of_private_key` 用 `decode_secret_key` 解析私钥、
  取 `public_key().fingerprint(HashAlg::Sha256)`；正例是"与 `testing::key_pair()` 报的那一串
  逐字符相同"，负例是"解不开时报 `PrivateKeyUnusable`，不给一个空指纹"。

### 留着的

- `bw_cache_check` **只报不改**：刷新仍是用户再点一次导入。自动刷新会把一次网络往返变成一次
  对库的写入，而那条路径上没有人看着。
- 上游那一条被删掉时只报 `gone`，**不自动清理**库里的那一行：删用户库里的钥匙是用户动作。
