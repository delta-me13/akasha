# Plan 0506: `~/.ssh/config` 受限子集导入

- **关联**：ROADMAP 阶段 5 ·「`~/.ssh/config` **受限子集**导入（`Match` / `Include` 显式报错）」
- **前置**：plan 0502（ssh 配置池的字段集先立起来）· plan 0505（**已完成** —— 导入的 `ProxyJump` 这时才真能用）
- **状态**：已完成（2026-09-13）
- **关联决策**：ADR-0003 **D14**（六条指令 + "不许静默误解析"）。本 plan 把它的**三档边界**、求值语义与 `ProxyJump` 语法写全；落地后按三态规则在 ADR 的 §14 记一行。

## 目标

导入**受限子集**：`Host` / `HostName` / `User` / `Port` / `IdentityFile` / `ProxyJump`。
**明确不支持 `Match` 与 `Include`**，遇到时**显式报错**而不是静默跳过
（`scope.md` §8 风险 4 —— "受限"绝不能变成"悄悄错"）。

两条配套要求（否则受限会退化成静默误解析）：

1. 遇不支持指令：报错或明确警告，绝不静默跳过
2. UI 上列出**支持哪些指令**，让用户知道边界在哪

导入是**一次性快照**：读一次文本、算出一批条目、落进 ssh 配置池，此后与系统 config 再无关联。

## 三档边界（D14 的展开 —— 本 plan 唯一的新决策）

| 档 | 含哪些 | 行为 |
|---|---|---|
| **导入** | 六条 | 映射进池 |
| **认识、但不导入** | OpenSSH 10.5 定义的其余关键字，按类别：保活/超时、算法与压缩、认证方式与凭据来源、known_hosts 策略、转发、日志与本地呈现、环境与本地命令、连接复用 | **逐条警告**并继续 —— 说清"这条指令**不生效**" |
| **整份报错** | ① `Match` / `Include`（条件与内容都不在我们读到的文本里，可改写**任何**条目）② 改目的地或解析路径：`ProxyCommand` / `ProxyUseFdpass` / `Canonicalize*` / `BindAddress` / `BindInterface` / `AddressFamily` / `Tunnel` ③ 改信任来源：`HostKeyAlias` / `RevokedHostKeys` ④ `IgnoreUnknown`（"放行"的语义）⑤ **表里没有的** | 一行都不写；报错**一次列全**（带行号） |

第二档为什么敢放行：它**不改"连到哪台机器"**，也**不改"我们信任哪把主机密钥"**，而"没生效"会**逐条**出现在报告里 —— 不是静默。第三档的兜底是**默认报错**：真有一条我们漏判的"会改目的地"的新关键字，它会落在⑤，方向是保守的那一边。

## 判据（语义依据，实测于系统 OpenSSH 10.5 的 `ssh -G`）

`Host` / `Match` 的意义只有一条规则：**每个参数首次取到的值生效**
（`ssh_config(5)`："the first specified value will be used"）。由此：

- 文件**开头到第一个 `Host` / `Match` 之间**的指令是**全局**的（等价于 `Host *`）；
- **先写的胜**：开头 `Host *` + `Port 2222`，后面 `Host foo` + `Port 33` → `foo` 的端口是 **2222**（实测），不是 33 —— "每块一行"那种读法会静默连错端口；
- **关键字不分大小写**（`hostname` = `HostName`），**`Host` 模式分大小写**（`Host foo` 不匹配 `FOO`，实测）；
- `Host` 一行可有多段模式，支持 `!` 取反与 `*` / `?` 通配；**通配块本身不是条目**，它的取值靠匹配并入具体条目（`Host *` 就是这么用的）。

## 非目标

- 完整解析器（`scope.md` §8 风险 4 已否决：优先级规则很绕，错了会**静默连错主机**）
- 完全放弃读取系统 config（已否决：用户已有的配置要手工重录）
- 与系统 config 的**持续同步**：导入是**一次性快照**（`scope.md` §3）
- **私钥文件不导入**（本 plan 的取舍）：`IdentityFile` 只让条目算成 `auth = publickey` + `key_id` 留空（= 走 ssh-agent），报告里列出"密钥文件没有导入"。把私钥复制进库是**机密写入**，该单独一个 plan 并按 ADR-0002 D13 重验；库也不存路径（P2）
- 写回 `~/.ssh/config`；系统级 `/etc/ssh/ssh_config`（只读用户那一个文件）

## 步骤（每步都能独立验证）

1. [ ] `akasha-store/src/sshconfig.rs`：**纯函数** `parse(&str)` —— 不碰库、不碰盘。词法：注释（引用外的 `#` 截断）、双引号、`key=value` 与空白两种分隔、**关键字小写化**、空行；`Host` 多段模式与 `!` / `*` / `?`；`Match` / `Include` 记成待报的问题
2. [ ] **求值**：对每个**字面**模式（无通配、非取反）扫全部块，按"首次取值"算出六条 —— 这是"不自己发明优先级"的落点。通配块与全局段只参与匹配，另记一份"未作为条目导入的块"
3. [ ] 三档分类表 + 问题类型（**行号 + 关键字 + 说法**）：报错路径**一次列全**（不是遇到第一条就停，用户一趟改完）；警告按类别给一条说法
4. [ ] `ProxyJump`：`[user@]host[:port]`、**逗号链**（`a,b` = 经 b 再经 a）、`none` = 关掉。每个跳板名走**同一个求值器**求一次值 → 命中导入的字面条目就用它，否则用池里同名的行，再否则**补建一条**（报告里列明"为跳板补建"）
5. [ ] `akasha-store/src/pools/import.rs`：**一个事务**落库 —— 先全部按 `jump_id = NULL` 插入，再用 `update_host` 逐条挂链，于是**成环检查只有一份实现**（`insert_host` 刻意不做的那份，而导入正是第一条"新行带着入边进来"的路径）。同名（`name UNIQUE`）默认**跳过并列出**，`overwrite` 时改走 `update_host`
6. [ ] app：`import_ssh_config(path, overwrite) -> ImportReport`（缺省 `~/.ssh/config`，与 0503 的 `~/.ssh/known_hosts` 同一条家目录推导；**文件不存在是明确错误**，不是"导入了 0 台"）；库锁着 → `Locked`；`just gen-types` 产物
7. [ ] 前端（验证壳层）：主机选择器上加"从 `~/.ssh/config` 导入"入口 + **支持/不支持指令清单**（风险 4 第 2 条）+ 报告（新增 / 跳过 / 未生效逐条）
8. [ ] E2E `ssh_config_import`（复用 0505 的假服务端脚手架）+ 文档：ADR-0003 D14 修订与 §14、`scope.md` §3 / §8 各补一句、`ROADMAP.md` 勾选、`plans/README.md`、`STATUS.md`（覆盖写）、归档

## 验收命令

```bash
# 1. 解析器（纯函数，不碰库也不碰盘）：预期全绿 —— 正常条目 / Host * 默认值 /
#    重复 Host / 通配 + 取反 / Match / Include / 未知关键字 / 已知但不导入 / ProxyJump 三种写法
cd src-tauri && cargo nextest run -p akasha-store sshconfig

# 2. 落库 + 既有池的回归：预期全绿（含"导入进来的行也不含绝对路径"这条 P2 检查）
cd src-tauri && cargo nextest run -p akasha-store

# 3. 可执行的 DoD。预期：6/6 绿（fmt-check / lint / test / deny-offline / gen-types-check / docs-check）
just ready

# 4. 真实路径（AGENTS.md §7 第一条）：真 app 上导入 fixture，再用**导入进来的那些行**开一个经跳板的会话。
#    预期 ssh_config_import 这一节打印：created = 2（名字来自 Host 模式）·
#    跳板服务端收到 1 条 direct-tcpip → akasha-e2e-inner.invalid:22 · 终端回声一致；
#    另外：含 Match 的那份 fixture 报错带行号，且 `vault_hosts` 的行数与导入前**一样**（一行都没写）
just test-e2e

# 5. 手查（**开发期证据，不进任何门禁** —— 系统 ssh 不是本产品的依赖，scope.md §2.1）：
#    同一份 fixture 喂给系统 ssh，比 hostname / user / port 三列。上面「判据」里的三句话就是这么做出来的
cd src-tauri && printf 'Host *\n Port 2222\nHost foo\n HostName foo.example\n' > target/probe.cfg && \
  ssh -F target/probe.cfg -G foo | grep -E '^(hostname|user|port) '
#    预期：port 2222（**不是** 22）、hostname foo.example —— 即"首次取值胜出"
```

## 回滚

解析器与落库都是**新增文件**；app 侧新增一个命令 + 一个前端入口，回滚 = 删掉它们并重跑
`just gen-types`。已经导入进池的行**没有界面能删**（阶段 4 的 CRUD 界面仍未规划），要清掉得
直接改库 —— 这是它继承的既有边界，不是本 plan 引入的。

## 实施记录（2026-09-13）

```
$ cd src-tauri && cargo nextest run -p akasha-store
Summary [17.1s] 127 tests run: 127 passed, 0 skipped     # plan 0403 起 92 → 127：+29 解析 / +5 落库 / +1 P2
$ just test
Summary [15.0s] 279 tests run: 279 passed, 0 skipped     # plan 0505 时 243（+36）
$ just ready
→ fmt-check ✅ / lint ✅ / test ✅ / deny-offline ✅ / gen-types-check ✅ / docs-check ✅   6/6
$ just test-e2e
→ 退出码 0：21 个用例通过（新增 `ssh_config_import`）+ `portable` 3
```

### E2E（`ssh_config_import`）打印的原文

```
界面：支持集 = Host,HostName,User,Port,IdentityFile,ProxyJump
导入报告：…/target/e2e-config/good.conf：新增 2 · 更新 0 · 跳过 0
未生效：第 3 行 serveraliveinterval：保活与超时只影响这条连接自己的寿命，不影响连到哪台机器
        第 4 行 identityfile：这个密钥文件没有导入：…
主机池：[{…"name":"e2e-config-jump","host":"127.0.0.1","port":34485,"jumpId":null,…},
         {…"name":"e2e-config-target","host":"akasha-e2e-import.invalid","port":22,"jumpId":1,…}]
整份被拒：第 4 行 match：条件块无法求值，而它可以给**任何**条目加设定…
读不到：读不到 …/does-not-exist.conf：没有那个文件或目录 (os error 2)
提示问答（按发生顺序）：["hostKey:BnkC…", "credential:e2e@127.0.0.1:34485",
                        "hostKey:jTGf…", "credential:e2e@akasha-e2e-import.invalid:22"]
跳板服务端：收到 1 条 direct-tcpip → akasha-e2e-import.invalid:22
终端回声：via-imported-config（目标收到了同一串）
关标签页：sessions probe = {"live":1,"registered":1}
```

### 手查：与系统 ssh 的差分对照（**开发期证据，不进任何门禁**）

同一份 fixture（就是 E2E 用的那份）喂给系统 `ssh -G`：

```
$ ssh -F src-tauri/target/e2e-config/good.conf -G e2e-config-target \
    | grep -E '^(hostname|user|port|proxyjump) '
hostname akasha-e2e-import.invalid
user e2e
port 22
proxyjump e2e-config-jump
```

四列与导入进池的那一行**逐字一致**（`host` / `user` / `port` / `jump_id` 指向的那一行）。
「判据」那三句（首次取值胜出、全局段、关键字不分大小写而模式分、没写 `HostName` 时小写化）
也都是这么实测出来的。系统 ssh 只在这个位置出现 —— 它不是本产品的依赖（`scope.md` §2.1）。

### 与原计划的偏差

1. **`ProxyJump` 只收裸名字**：原计划写的是 `[user@]host[:port]`。落地时确认 `user@` / `:port`
   是对**某一跳**的局部改写，而池里一跳就是**一行** —— 没有对应的表示。于是它变成一条报错
   （并给出"写一条 `Host` 块"的改法）；逗号链照做（`a,b` = 靠目标最近的是 b、b 的跳板是 a）。
2. **补建的跳板条目（`provisional`）**：`ProxyJump` 点到配置里没有 `Host` 块的名字时，按**同一个
   求值器**补建一条（报告里列明）。它是"够用就好"的兜底，所以**永不覆盖**池里的同名行 ——
   即使开了 `overwrite`。
3. **导入是第一条"一次插入多行、而这些行互相引用"的路径**：`insert_host` 那条成环论证
   （"新行没有入边"）不再成立，所以落库分两步（先全按 `jump_id = NULL` 插入，再用
   `update_host` 逐条挂链）—— 成环检查因此仍然只有一份实现。这一条原先没预见。

### 没有做的事（照实记）

- **多跳链没有端到端跑过**：crate 用例覆盖了 `a,b` 的链序与补建，E2E 仍只有一跳（与 0505 同一个边界）。
- **不读 `/etc/ssh/ssh_config`**：只读用户那一个文件。
- **私钥不导入**：`IdentityFile` 只兑现一半（见非目标），连接时走 ssh-agent。
- **导入进来的行在界面上改不了**（阶段 4 的池 CRUD 界面仍未规划），只能靠再导一次 + `overwrite`。
