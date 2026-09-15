# Bitwarden 集成（参考资料）

> 本文是 [`scope.md`](./scope.md) §7 的**展开**。
> **结论在 `scope.md` 与 [`adr/0007`](./adr/0007-bitwarden-cli-acquisition.md)，条款原文、
> 命令面与实测过程在这里** —— 分开是因为两者时效不同：结论偶尔变，上游条款、命令与
> 字段会随版本变。
>
> 之所以单独成文：这些内容一度放在 `scope.md` 里，使它增长到 600 行以上，
> 而"能力清单"不该装条款原文。规则见 `AGENTS.md` §8.1。

**本集成使用 `bw` 的主依据是官方文档**：<https://bitwarden.com/help/cli/>。
CLI 自称"self-documented"（`bw --help` 与 `bw <command> --help` 覆盖同一份内容），
因此文档与 `--help` 可以互相核对；本文记录的是**我们用到的那些**命令与实测输出。

---

## 1. 前提：CLI 从哪来

**装了 Bitwarden 桌面端不等于有 `bw`。** 桌面端**不含** CLI，`bw` 是独立下载：
npm `@bitwarden/cli`、snap、Flatpak（随桌面端一起）、或 GitHub Release 的本地可执行文件。

**本项目的口径（ADR-0007 D4）：两个轴各自可切，默认都取 `host`。**

| 轴 | `host`（默认） | `managed` |
|---|---|---|
| 二进制 | `PATH` 里的 `bw` | 运行时下载到数据目录 `bitwarden/bw-<版本>/` |
| 状态目录（`BITWARDENCLI_APPDATA_DIR`） | 不设 —— 用 CLI 自己的默认目录 | 数据目录 `bitwarden/appdata/` |

- **运行时下载只取 OSS 变体**（ADR-0007 D1）：理由见 §2。
- **`host` 轴上找不到 `bw` 时不静默切换**：界面说清"这台机器上没有 `bw`"并给出一个显式的
  下载动作（ADR-0007 D5）。
- **默认取 `host` 是用户当次指令**：CLI 的状态里有 access token，让它在系统默认位置不动，
  比随本项目的便携目录迁移的风险低。

---

## 2. ⚠️ 许可证：这是"不打包"的硬约束，不是省体积的偏好

`bw` 从 **v2024.6.1** 起分叉成两个许可证不同的变体。Bitwarden 仓库里同时存在
`LICENSE_BITWARDEN.txt` 与 `LICENSE_GPL.txt`：

| 变体 | 许可证 | 能否打包分发 | 能否用于生产 |
|---|---|---|---|
| `bw`（专有） | Bitwarden License Agreement | ❌ **2.3(i) 禁止分发** | ❌ **2.1 仅限"内部开发与内部测试、非生产环境"** |
| `bw-oss-*` | **GPL-3.0-only** | ⚠️ 可以，但会把 GPL-3.0 义务带进我们的分发 | ✅ |

**2.1 原文**：

> a limited, non-exclusive, non-transferable, royalty-free license to use the
> Commercial Modules **for the sole purposes of internal development and internal
> testing, and only in a non-production environment**.

**2.3 另禁止**：*"sell, rent, lease, **distribute**, sublicense, loan or otherwise
transfer the Commercial Modules to any third party"*，以及
*"use the Commercial Modules to create a competing product or service"*。

**官方文档对两个变体的说法**（下载章节原文）：

> For each bundle of the Password Manager CLI available on GitHub, there is an OSS
> (e.g. `bw-oss-windows-2024.12.0.zip`) and non-OSS build (e.g. `bw-windows-2024.12.0.zip`).
> The non-OSS version is the default package distributed on distribution platforms and
> includes features under a non-OSS license, such as device approval commands, that the
> OSS version lacks.

### 2.1 三条路与当前定案

| 方案 | 评价 |
|---|---|
| **运行时下载 `bw-oss-*`**（**当前定案**，ADR-0007 D1） | ✅ 本项目**不分发**二进制（下载发生在用户机器上、来源是上游自己的 release），因此 GPL 义务不落到本项目；GPL-3.0-only 对使用者没有用途限制。代价：缺 device approval 一类企业管理命令（本项目范围不需要） |
| 打包 `bw-oss-*` | 合法，但把 GPL-3.0-only 义务带进我们的分发（许可证文本 + 源码获取途径），并要自己承担下载 / 校验 / 更新 |
| 打包或下载**专有**变体 | ❌ 2.3(i) 禁分发；2.1 禁生产使用。**"不打包"是法律要求，不是取舍** |

### 2.2 变体的**可执行**判据（实测）

上游不随二进制附带许可证文本（两份 `cli-v2026.8.0` 产物里都搜不到 `LICENSE_*` 字样），
版本号也分辨不出（两版都输出 `2026.8.0`）。可用的判据是**命令表**：

```
# 非 OSS（bw-linux-2026.8.0.zip）：
  device-approval    Manage device approval requests sent to organizations that use SSO with trusted devices.

# OSS（bw-oss-linux-2026.8.0.zip）：命令表里没有这一行
```

因此判定 = **读 `bw --help` 的命令表里有没有 `device-approval`**。
⚠️ 它依赖上游保留这个命令名；读不出来时**报"判不出变体"**，不默认成 OSS
（ADR-0007 §5）。

### 2.3 因此 `host` 轴上仍要做前置检查

探测到**专有**变体时必须给出提示 —— 这是 `scope.md` §7 "不得让用户在生产环境中使用
专有变体而无任何提示"的落点。`managed` 那一轴只下 OSS，不涉及这条。

---

## 3. 为什么不用 `rbw` 或官方 Rust SDK

四条路都核实过，留档避免以后重新论证：

| 方案 | SSH key 条目 | 依赖 | 问题 |
|---|---|---|---|
| **`bw` CLI**（官方） | ✅ 支持 | 自包含二进制，约 100 MB | 无状态模型（须自己管 `BW_SESSION`）；写自己的 appdata |
| **`rbw`**（纯 Rust 非官方） | ❓ 未确认支持新条目类型 | **需外部 `pinentry`** | 作者声明"功能上已对我是完整的"，**只修回归不添功能**；SSH key 是较新的条目类型 |
| **Bitwarden 官方 Rust SDK** | — | — | 公开的 `bitwarden` crate 是 **Secrets Manager**；真正含 `SshKey` 的 `bitwarden-exporters` / `bitwarden-vault` 在 crates.io 上标注 **"Do not use"（internal）** —— 不是可依赖的公开面 |
| 自实现 vault 密码学 | — | — | **已列为非目标**（`scope.md` §10） |

**选 `bw` 的理由**：它是唯一**官方维护且可靠支持 SSH key 条目**的路径。
`rbw` 更符合"纯 Rust"审美，但非官方、依赖外部 `pinentry`，且作者已声明不添功能 ——
依赖它来支持新条目类型的风险过高。

---

## 4. 条目结构与字段

来自官方 Rust SDK（`bitwarden_exporters::SshKey`）：

```rust
pub struct SshKey {
    pub private_key: String,  // OpenSSH private key, PEM
    pub public_key: String,   // ed25519/rsa, RFC4253
    pub fingerprint: String,  // "SHA256:<base64>"
}
```

**先明确 fingerprint 的定义**：它是**公钥的 SHA-256**。
不得与 SHA-512 混淆 —— Bitwarden SSH agent 对 RSA 密钥一律用 sha512 **签名**
（[clients#16681](https://github.com/bitwarden/clients/issues/16681)），
那是**签名哈希**，与指纹是两个不同的概念。

---

## 5. `revisionDate` 与 `fingerprint`：两个字段、两个职责

（上游字段名为 `revisionDate`；库内列名与 §5.3 缓存元组写作 `revision_date`，两者指同一件事。）

**它们覆盖不同的失效模式，都不可省。**

| 字段 | 回答的问题 | 需要联网 | 覆盖范围 |
|---|---|---|---|
| `revision_date` | **上游变过吗？** | 是 | **所有**字段的改动（名称 / 备注 / 公钥 / **私钥**） |
| `fingerprint` | **本地这份缓存还好吗？** | **否** | 私钥是否仍与公钥配对；缓存是否被损坏 |

### 5.1 `fingerprint` 不可替代的能力：离线自检

用缓存里的私钥推导出公钥 → 算 SHA-256 → 与存的指纹比对。
**完全不需要网络** —— 这正是"Bitwarden 离线也要能用"这条需求最需要的保障。

### 5.2 但它不能替代 `revision_date`

它有明确的**盲区**：**私钥变了而公钥没变**时指纹不变（例如重新导入一对不匹配的
密钥），而 `revision_date` 会变。元数据改动（备注、关联主机）同理。

### 5.3 所以缓存必须同时存，两个时机各查一次

```
{item_id, revision_date, fingerprint, 加密私钥}
```

- **离线时**：用 `fingerprint` 校验缓存完整性（此时唯一可用的手段）
- **联网时**：比对 `revision_date` 决定是否刷新，并用上游 `fingerprint` 复核同一性

---

## 6. 会话与写入范围

- **会话**：由本程序驱动 `bw unlock` / `bw login`（向用户索取主密码），拿到 session key 后
  **仅在内存中持有**，且住在受保护页里（ADR-0007 D7，`akasha_store::protected::Protected`）；
  **不落盘、不写进任何环境文件、不进日志与事件载荷**。
- **主密码**经 `--passwordenv` 交给子进程（ADR-0007 D8）—— 不用位置参数（argv 对同机进程
  可见），不用 `--passwordfile`（会把主密码落盘）。
- **`bw` 自己的状态**（access token、`data.json`）由 CLI 管理，落点取决于状态目录那一轴：
  `host` = CLI 默认目录，`managed` = 数据目录下的 `bitwarden/appdata/`。
  ⚠️ 它是**上游的状态**，不是本项目的机密：状态目录取 `host` 时它不随便携目录迁移。
- **v1 没有任何 `bw create` / `bw edit` 调用**。这避开了上游写入的并发/冲突/回滚问题，
  也避开了"Bitwarden 不接受某密钥格式"的分支。
- 双向移动/复制明确推迟（`scope.md` §10）。

> **变更记录**：最初设想是"两池之间允许移动或复制（双向）"。现已缩小为
> **v1 只读导入**。理由：写回上游引入并发/冲突/格式兼容三类问题，
> 而"从 Bitwarden 导入到本地池"已覆盖主要使用场景。

---

## 7. 本集成用到的命令面（含实测输出）

全局选项（`bw --help`）：`--raw`（只输出裸值）、`--nointeraction`（禁止交互提问）、
`--session <key>`、`--pretty`、`--quiet`。

| 用途 | 命令 | 实测要点 |
|---|---|---|
| 版本 | `bw --version` | 输出 `2026.8.0`（**两版变体同值**，分辨变体要用 §2.2） |
| 变体判定 | `bw --help` | 命令表里 `device-approval` 一行是否存在 |
| 状态 | `bw status --raw` | 恒定退出码 0；形状见 §7.1 |
| 服务器 | `bw config server` / `bw config server <url>` | 无参数 = 读回当前服务器，输出**不带换行**；带值 = 写入。官方文档注明后续任何一次 `config` 调用会覆盖此前全部取值 |
| 登录 | `bw login <email> --passwordenv <VAR> --raw --nointeraction` | 成功时 stdout 就是 session key 本身 |
| 登录（2FA） | `bw login <email> --method <m> --code <c> ...` | `method`：`0` 认证器 / `1` 邮件 / `3` YubiKey；FIDO2 与 Duo **CLI 不支持** |
| 解锁 | `bw unlock --passwordenv <VAR> --raw --nointeraction` | 同上；`--check` 只查锁定状态 |
| 锁定 | `bw lock` | 使 session key 失效 |
| 登出 | `bw logout` | 同上，并清掉登录态 |
| 同步 | `bw sync` | 只做 pull；`--last` 只回上次同步的时间戳（ISO 8601） |
| 列条目 | `bw list items --raw --session <key>` | 只读导入的入口（plan 0903） |

### 7.1 `bw status --raw` 的形状

未登录（**实测**）：

```json
{"serverUrl":null,"lastSync":null,"status":"unauthenticated"}
```

官方文档给出的完整形状（登录之后，含两个额外键）：

```json
{
  "serverUrl": "https://bitwarden.example.com",
  "lastSync": "2020-06-16T06:33:51.419Z",
  "userEmail": "user@example.com",
  "userId": "00000000-0000-0000-0000-000000000000",
  "status": "unlocked"
}
```

`status` 三个取值：`unlocked`（已登录且解锁）/ `locked`（已登录未解锁）/
`unauthenticated`（未登录，此时 `userEmail` 与 `userId` 不存在）。

### 7.2 失败长什么样（实测）

| 场景 | 输出 | 退出码 |
|---|---|---|
| 未登录就查数据（`bw list items` / `bw unlock`） | `You are not logged in.` | **1** |
| `bw status` | 正常 JSON（未登录也是一种状态） | **0** |
| 服务器用明文 HTTP | `InsecureUrlNotAllowedError: Insecure URL not allowed. All URLs must use HTTPS.` | 1 |
| 服务器连不上 | `Unable to fetch ServerConfig from <url>/api FetchError: ... errno: 'ETIMEDOUT'` | 1 |
| 自签证书未被信任 | `... reason: self-signed certificate` | 1 |

**因此"命令成功"不能只看退出码为 0 这一件事**：`bw status` 在未登录时也返回 0，
而查询类命令在未登录时返回 1 并把那句话写在 **stdout**（不是 stderr）。

### 7.3 自签证书（自托管常见）

官方文档的做法：设 `NODE_EXTRA_CA_CERTS` 指向证书 PEM。
**实测有效**：指向自签证书之后，`bw login` 真的向本地桩发出了
`GET /api/config` 与 `POST /identity/accounts/prelogin/password`。

---

## 8. 实现前必须实测的四项：结论

| # | 问题 | 结论 | 状态 |
|---|---|---|---|
| 1 | 未解锁 / 未登录时的报错形态 | 未登录 = `You are not logged in.` + **退出码 1**；`bw status` 恒 0；**未解锁**（已登录但无 session key）下 `bw list items` 的原文**仍未实测** —— 需要一个真实 vault | ⚠️ 部分 |
| 2 | `bw list items --raw` 的 JSON 形状（`sshKey` 的嵌套） | **未实测**（需要一个真实 vault） | ❌ |
| 3 | 条目是否**稳定可见**（离线 / 未同步时） | **未实测** | ❌ |
| 4 | 如何分辨专有变体与 OSS 变体 | **已定判据**：读 `bw --help` 的命令表里有没有 `device-approval`（§2.2）。⚠️ 它是启发式（上游改命令表即失效），且读不出来时要报"判不出" | ✅ |

> #1 / #2 / #3 的共同门槛是**一个真实 vault**：登录之后 `bw status` 的形状、错误措辞与
> `sshKey` 条目的嵌套都只能在那里看到。本机无法用桩服务器造出来 ——
> `bw login` 的 session key 来自对上游返回的**加密用户密钥**解密，桩服务器要造出这个
> 就得自己实现 Bitwarden 的密钥派生与加密，而那正是 `scope.md` §10 的非目标。
> 这三项因此排在 plan 0903（只读导入）的展开时机上，见 [`STATUS.md`](./STATUS.md)。
