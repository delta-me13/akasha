# Bitwarden 集成（参考资料）

> 本文是 [`scope.md`](./scope.md) §7 的**展开**。
> **结论在 `scope.md`，条款原文、条目结构与推导过程在这里** —— 分开是因为两者时效不同：
> 结论偶尔变，上游条款与字段会随版本变。
>
> 之所以单独成文：这些内容一度住在 `scope.md` 里，把它撑到 600 行以上，
> 而"能力清单"不该装条款原文。规则见 `AGENTS.md` §8.1。

---

## 1. 前提：「装了 Bitwarden 桌面端就有 `bw` 吗？」

**没有。** 桌面端**不含** CLI。`bw` 是**独立下载**：
npm `@bitwarden/cli`、snap，或 GitHub Release 的二进制。

所以"用户自备前置"的实际含义是：**用户要额外去找并安装**一个约 100 MB 的 CLI。
这比"系统里已经有"重得多，也是为什么 §2 的许可证结论值得写下来。

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

**结论：专有变体既不能打包，也不能作为生产依赖** —— "不打包"是法律要求，不是取舍。

### 2.1 剩下的路

| 方案 | 评价 |
|---|---|
| **用户自备**（**当前定案**） | ✅ 我们不分发，GPL 义务不落到我们头上。代价：用户须自行安装 ~100 MB CLI，且我们**必须告知装哪个变体** |
| 打包 `bw-oss-*` | 合法，但把 GPL-3.0-**only** 义务带进我们的分发（许可证文本 + 源码获取途径），并要自己承担下载/校验/更新 |
| 运行时下载官方二进制 | ⚠️ 有先例（Raycast 扩展[这样做过](https://github.com/raycast/extensions/pull/8315)），但那个 PR 合并于 **2023-09**，**早于** 2024.6.1 的许可证分叉 —— **不能作为分叉后的先例** |

**因此"前置检查"要做的不止是"有没有 `bw`"**：还要**分辨变体**，
探测到专有变体时给出提示。别让用户在生产里用专有变体而我们一声不吭。

> **体积是次要原因**：约 100 MB 的 Node SEA 自包含二进制（好处是不需另装 Node）。
> 两个原因结论一致，但**法律那条才是决定性的**。

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
把新条目类型的支持押在它身上风险太高。

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

**先把 fingerprint 说准**：它是**公钥的 SHA-256**。
不要与 SHA-512 混 —— Bitwarden SSH agent 对 RSA 密钥一律用 sha512 **签名**
（[clients#16681](https://github.com/bitwarden/clients/issues/16681)），
那是**签名哈希**，与指纹是两件事。

---

## 5. `revisionDate` 与 `fingerprint`：两个字段、两个职责

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

- **会话**：由本程序驱动 `bw unlock`（向用户索取主密码），拿到 `BW_SESSION` 后
  **仅在内存中持有**；不落盘、不写进任何环境文件。
- **v1 没有任何 `bw create` / `bw edit` 调用**。这避开了上游写入的并发/冲突/回滚问题，
  也避开了"Bitwarden 不接受某密钥格式"的分支。
- 双向移动/复制明确推迟（`scope.md` §10）。

> **变更记录**：最初设想是"两池之间允许移动或复制（双向）"。现已缩小为
> **v1 只读导入**。理由：写回上游引入并发/冲突/格式兼容三类问题，
> 而"从 Bitwarden 导入到本地池"已覆盖主要使用场景。

---

## 7. 实现前必须实测（目前仍未验证）

需要**一个真实 vault** 才能测，本地无法推断：

- [ ] 未解锁时 `bw` 的报错形态（决定前置检查怎么写）
- [ ] `bw list items --raw` 的 JSON 形状（决定解析与字段映射）
- [ ] SSH key 条目是否**稳定可见**于列表
- [ ] **如何分辨专有变体与 OSS 变体**（许可证结论依赖这一点）

放到 `docs/plans/` 里作为一个可执行的前置检查项。
