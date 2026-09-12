# Plan 0400: 写 ADR-0002（机密存储与可搬迁）

- **关联**：ROADMAP 阶段 4 ·「ADR-0002 定案」
- **前置**：无（但它是 plan 0401 / 0402 的**前置**）
- **状态**：进行中（ADR 草案已写，**待 cyrene 裁定**；接受动作见「实施记录」）
- **影响面**：`docs/adr/0002-*.md`（新增）、`docs/adr/README.md`（状态）
- **后继**：plan 0401 – 0405（它们从本 ADR 取实现约束）

## 目标

把**数据文件格式级**的决定一次性定案并接受，让阶段 4 的实现不需要边写边改口径：

- SQLCipher 与 feature（`bundled-sqlcipher-vendored-openssl` —— vendored 是必须的）
- **口令 → KDF → 库密钥**的参数（算法、盐、迭代/成本参数的**取值与理由**）
- 口令**不落盘**、不依赖任何 OS keychain（P1）
- 导出格式（加密导出用独立口令；明文导出必须二次确认）
- 可搬迁：数据目录推导、**库里不存绝对路径**（P2）
- Bitwarden 引入的机密来源与缓存强度（与本地池同级）

## 非目标

- **不**写实现代码（那是 plan 0401 起）
- **不**在本 ADR 里记录可逆的 UI 选择（例如导出按钮放哪）—— 可逆项进 `scope.md`
- **不**提前替阶段 5 的 SSH 决策（那是 ADR-0003）

## 前置检查

```bash
sed -n '/## 6\. /,/## 7\. /p' docs/scope.md     # 存储与加密的已定案条目
sed -n '1,60p' docs/portable.md                 # 可搬迁的三条要求
sed -n '1,60p' docs/adr/README.md               # 写 ADR 的门槛与队列
```

## 步骤

1. 逐条把 `scope.md` §6 / §7 与 `portable.md` §2 的**已定案**内容转成 ADR 的决策条目：
   每条写「决定 / 理由 / 否决的替代路」。
2. 补齐 `scope.md` **还没定值**的部分，这是本 ADR 的主要产出：
   KDF 算法与参数、盐与 nonce 的存放位置、导出文件的格式与版本字段、
   缓存强度"同级"的具体含义。
3. 写明**不可逆点**：哪些决定一旦有用户数据就改不动（数据文件格式），以及迁移会付出什么代价。
4. 校验：ADR 中每条决定都能在 `scope.md` 找到出处或明确标注为"本 ADR 新增"。
5. 把 `docs/adr/README.md` 里 0002 的状态改为「已接受」，并记日期。

## 验收命令

```bash
ls docs/adr/0002-*.md
grep -n '状态' docs/adr/0002-*.md           # 期望：已接受（Accepted，<日期>）
grep -c '决定\|否决' docs/adr/0002-*.md    # 目视：每条决定都带理由与否决项
just docs-check                             # 期望退出码 0
just ready                                  # 期望退出码 0（ADR 自身不引入代码改动）
```

**判据**：ADR 覆盖 `scope.md` §6 / §7 与 `portable.md` §2 的**每一条**已定案，
且把"还没有值"的那些补上了值 —— 后者才是它存在的理由。

## 回滚

ADR 未接受前可随意改写；接受后只能由新 ADR 取代（`docs/adr/README.md` 的约定）。
本 plan 不产生用户数据，回滚 = 删除该文件。

## 实施记录

**2026-09-12：草案已写**（[`docs/adr/0002-secret-storage.md`](../adr/0002-secret-storage.md)）。
12 条决定全部带「决定 / 理由 / 否决的替代路」，总览表逐条标了出处（`scope.md` / `portable.md` / 本 ADR 新增）。

定值时真正做的选择（其余是抄 `scope.md` §6 的已定案）：

| 问题 | 取值 | 关键理由 |
|---|---|---|
| 库文件个数 | 一个（四套池 + BW 缓存） | 一次解锁、一次导出、跨池事务 |
| KDF | SQLCipher 原生 PBKDF2-HMAC-SHA512 / 256,000 | 外置盐要么旁挂文件、要么明文头部；且**可原地升级**（ADR §5.2），所以现在不必为它付永久成本 |
| 口令送入方式 | `sqlite3_key()` C API | 不进 SQL 文本 → 不进错误消息与日志；无转义陷阱 |
| 导出容器 | 同参数的另一个 SQLCipher 库；明文导出 = `KEY ''` 分支 | 恢复路径复用日常代码，不自造容器 |
| 版本字段 | `PRAGMA user_version` 单一权威 | `sqlcipher_export` 不传递它 → 导出后必须显式写 |
| WAL | 不用 | 单进程无可换的并发；而"复制单个 `.db` 即完整"正是 P2 的判据 |

**裁定后要做的收尾**（现在是「提议中」，所以一律未做 —— 未接受的 ADR 不是定案）：

1. ADR 头部状态改「已接受（Accepted，<日期>）」；
2. `docs/adr/README.md` 队列里的 0002 状态同步；
3. `scope.md` §6 补 ADR 指针（值已在 ADR，§6 只留结论），§7 的"同级"换成具体含义；
4. plan 0403 前置检查里"把 `config.json` 并入本库"一条据 D11 改掉；
5. 本文件状态改「已完成」并 `git mv` 进 `docs/plans/archive/`，
   `docs/plans/README.md` 的 0400 行同步，`ROADMAP.md` 阶段 4 第一条勾选。

**未决的实测项**（写在 ADR §7，由 plan 0401 在写第一行存储代码之前跑）：
`PRAGMA cipher_settings` 的实际输出、错误口令的报错形态、空口令的后果、`rekey` 后盐是否变化。
