# Plan 0403: 四套池的 CRUD

- **关联**：ROADMAP 阶段 4 ·「四套池的 CRUD：密钥 / ssh 配置 / serial 配置 / 端口转发规则」
- **前置**：plan 0401（库能开）· plan 0402（密钥能派生）· plan 0406（机密的内存形态已定）
- **状态**：已完成（2026-09-12）
- **后续**：0407（解锁与锁定的生命周期：谁持有 `Connection`、口令从哪来、何时抹掉）

## 目标

四套独立池的增删改查：**密钥池**（本地）/ **ssh 配置池** / **serial 配置池** / **端口转发规则池**
（字段集见 `scope.md` §3 的表）。库是**唯一真相源**，不是缓存。

## 非目标

- UI（各池的呈现形式未定，后端先按语义建模，见 `scope.md` §1.2）
- 从系统 `~/.ssh/config` 导入（阶段 5 的 plan 0506）
- Bitwarden 导入池（阶段 9）
- **解锁生命周期**（口令从哪来、谁来持有解好的 `Connection`、锁定时抹什么）→ 0407。
  本 plan 只做"库的落点"与"库的状态"，因为它是纯函数 + 只读命令，不需要先定生命周期
- ~~配置文件的迁移~~ → **已裁定不做**：`config.json` **不并进库里**（ADR-0002 D11）

## 设计要点（落进代码注释，不只在结论里）

1. **表结构进 `create()`**（这是 plan 0402 留的唯一入口）。`open()` 除版本号外**再查一次表在不在**：
   `user_version = 1` 的含义从此是"**这四张表**"，不是"一个空库" —— 否则"能开但内容不对"
   又回来了（D7 的意图）。列的形状由 `user_version` 负责：v1 一旦发布，改列就是 v2。
2. **私钥读出走受保护页**（ADR-0002 D13）：`Secret<[u8; 16384]>`，判据按 D13 的表**重验一遍**
   （`N` 与口令不同，那一页的大小也因此不同）。**写入侧先拒绝超过一页的私钥** ——
   否则库里会留下一条以后**读不出来**的记录，而那要到连接时才发现。
   `Protected`（`memsafe` 的薄封装）由口令与私钥共用：抄第二份就等于让两份各自漂移。
3. **`PRAGMA foreign_keys = ON` 必须在每个连接上开**：sqlite 的 FK **默认不生效**，
   声明了当没声明 —— 是"静默失效"里最典型的一种。删除被引用的行 → **明确报错**（RESTRICT），
   不静默把引用清空（"规则悄悄换了所属主机"比"删不掉"难查得多）。
4. **P2 第 2 条要能被机器查**，落成两条：
   ① 表里**没有任何名字带 `path` / `dir` / `file` 的列**（"别哪天引入一个密钥文件路径字段"）；
   ② 打开的库里枚举**所有**文本值，没有一个包含我们的数据目录。
   serial 的端口名（`/dev/ttyUSB0` / `COM3`）是**设备名**，不是我们的文件位置 —— 见 `scope.md` §3。
5. **`New*` 与 `*` 分开**：未入库的行**没有 id**，这件事打成类型事实，不用 `Option<i64>`
   （`Option` 会让每个调用点自己编一个"没有 id 时怎么办"）。
6. **app 侧只接路径与状态**（`vault_status` + 一个 probe）：它让 `AGENTS.md` §7 那条
   "真实路径走通"第一次有对象，且**不需要**先定解锁生命周期。

## 步骤（每步都能独立验证）

- [x] 1. 把 0406 的受保护页提成 `protected.rs`，口令改为用它；0406 的契约测试**判据不变、必须全绿**
- [x] 2. `schema.rs`：四张表 + `create()` 建表 + `open()` 校验；加一条 schema 快照测试
- [x] 3. `unlock` 里开 `foreign_keys` + 读回断言；"被引用时删不掉"的用例
- [x] 4. 密钥池：CRUD + 私钥的受保护页读路径 + 写入侧超页拒绝 + D13 判据重验
- [x] 5. ssh / serial / 转发三池：CRUD（含方向与缺省值的约束）
- [x] 6. 库内无绝对路径的两条检查（列名 / 全库文本值）
- [x] 7. 接进 app：`config.rs` 的 `vault_path`、`vault_status` 命令 + `vault` probe、
      `just gen-types` 产物、E2E 用例 `vault_status`
- [x] 8. 文档：ADR-0002 §7.3 与 §10、`scope.md` §3、`ROADMAP.md`、`STATUS.md`

## 验收命令

```bash
# 池的 round-trip 与"库里没有绝对路径"。预期：akasha-store 全部通过
cd src-tauri && cargo nextest run -p akasha-store

# 可执行的 DoD。预期：6/6 绿（fmt-check / lint / test / deny-offline / gen-types-check / docs-check）
just ready

# 真实路径（AGENTS.md §7 第一条）：真 app 上 invoke vault_status，再读 probe 对账。
# 预期：vault_status.path 落在便携数据目录里；state 为 missing（没建过）/ empty（0 字节）/ present
just test-e2e

# 手查 P2 第 2 条：预期**无输出**（没有任何名字像路径的列）
grep -nE '"?(.*_)?(path|dir|file)"?\s' src-tauri/crates/akasha-store/src/schema.rs
```

## 回滚

表结构只出现在 `create()` 里，`open()` 从不改库 —— 把 `schema.rs` 与 `create()` 的那一句
去掉就回到 0402 的状态（旧 fixture 是 `target/` 下的临时产物，删掉即可）。
app 侧的改动是**新增**一个命令与依赖，回滚就是删掉它们 + 重跑 `just gen-types`。

## 实施记录（2026-09-12）

```
$ cd src-tauri && cargo nextest run -p akasha-store
Summary [1.947s] 61 tests run: 61 passed, 0 skipped          # 0403 起从 25 → 61
$ just test
Summary [2.037s] 160 tests run: 160 passed, 0 skipped        # akasha 47 / pty 37 / core 15 / store 61
$ just ready
→ fmt-check ✅ / lint ✅ / test ✅ / deny-offline ✅ / gen-types-check ✅ / docs-check ✅   6/6
$ just test-e2e
── E2E: vault_status ──
vault_status = {"path":"/home/lycurgus/akasha/src-tauri/target/debug/akasha-data/akasha.db","state":"missing"}
落点是便携目录：/home/lycurgus/akasha/src-tauri/target/debug/akasha-data/akasha.db
```
`just test-e2e` 退出码 **0**：14 个通过、`window_close` **显式跳过**（这台机器 `tray_ready=false`
→ 关窗语义降级为退出，它只验"隐藏"那条路并打印了判据）。

### 逐条判据的实测值

| 判据 | 实测 |
|---|---|
| round-trip | 四套池各一条（含私钥**逐字节**回来、三个方向的转发、每个枚举取值） |
| 库里不存绝对路径 | 列名规则 +「全库文本值」两条；含**诱饵负例**（`/home/nobody/secret` 必须被看见，`direction` 必须不被误判） |
| 建库后的库文件 | **36864 字节 = 9 页**（0402 时是 4096 一页）；SQLite **3.46** |
| 外键 | `PRAGMA foreign_keys` 读回 **1**；删还被引用的密钥/主机 → `Conflict` 且行还在 |
| 私钥那一页（D13 重验） | smaps 多一页 `---p`、**16384 字节**；`VmLck` **68 → 84 kB**；`VmFlags = mr mw me lo ac wf dd sd`；`drop` 后那一页**消失** |
| 超长/空私钥 | 构造层就拒（`SecretTooLong` / `EmptyPrivateKey`）—— 库里不可能有"读不出来"的行 |

### 与原计划的偏差（都在文档里）

1. **多了一个计划外的决定：库里不许重名**（`name UNIQUE` 四张表都有）。原因：`~/.ssh/config`
   允许重复 `Host` 块、先匹配者生效 —— 那是文本文件的历史包袱，而库里的重名是"两行看起来一样、
   行为取决于没人能看见的顺序"。同时四套池都加了 `name`（用户给的标签），`scope.md` §3 的表已同步。
2. **跳板链的成环检查**不在原计划的步骤里：自环由 `CHECK` 挡，而 `A→B→C→A` **库表达不出来**
   （要递归），所以在 `update_host` 里逐跳走一遍（`MAX_JUMP_DEPTH` 只是防死循环的兜底）。
   `insert` 不需要：新行此刻没有入边（归纳：老数据无环 + 新行无入边 = 仍无环）。
3. **`get_registry` 那条 §7 的检查当前不可满足**（实测 `[]`）：本仓库的命令都没有
   `#[inspectable]`，注册表不镜像命令集。替代证据是真路径上的 `invoke_command` 成功；
   `AGENTS.md` §7 已就此补一句说明（单独提交）。

### 没有做的事（照实记）

- **解锁生命周期**：本 plan 只接"落点与状态"，见非目标与 plan 0407。
- **列的形状校验**：`open` 只查表在不在，不逐列比 —— 在打开路径上再写一份 DDL 的镜像，
  那份镜像迟早与 DDL 不一致，而"不一致的检查"比没有检查更坏。形状由快照测试 + 版本号负责。
