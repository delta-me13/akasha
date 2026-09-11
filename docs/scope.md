# 能力范围（Scope）

> 回答**"这个产品预备有什么"** —— 能力的唯一来源。`ROADMAP.md` 从这里取阶段，
> ADR 从这里取需要定案的决策。
>
> 时效：**偶尔变，只增删能力条目**。不写进度（进度在 [`STATUS.md`](./STATUS.md)），
> 不写"为什么这样定"（那在 [`adr/`](./adr/)）。
>
> 一个能力条目要进这里，必须能回答"它的验收标准是什么"。答不出来的，
> 说明还没想清楚，先留在讨论里。

图例：**`v1`** 首版就有 · **`later`** 明确要做，但不阻塞首版（可延后） ·
**`no`** 明确不做（**写下非目标与写下目标同样重要**）

> `later` 与 `no` 的区别是实质性的，不要混：§9 的表格逐条标注是哪一种。
> 把"推迟"写成"永不做"会让后人放弃一个本来就该做的事；
> 把"永不做"写成"推迟"则会让它每年被重新提一次。

---

## 1. 产品定位

**便携优先的多协议终端 + 文件传输客户端。**

三条定位性约束（它们会否决具体技术方案，所以放在最前面）：

| # | 约束 | 否决了什么 |
|---|---|---|
| P1 | **不依赖操作系统组件**（凭据由本程序自己保管） | `keyring` 类方案（GNOME Keyring / Keychain / Credential Manager）、任何"让 OS 帮我保管密钥"的设计 |
| P2 | **便携**：数据与二进制同目录 | 把状态散落在 `$HOME` / `%APPDATA%` 的设计 |
| P3 | **Windows 不向 C 盘写文件**（temp 与 webview 运行时可写目录除外） | 默认的 WebView2 用户数据目录、默认的 appdata 路径 |

> P2/P3 有**一个物理上限**，见 §7——webview 是你不控制的系统组件。
> 这不是"实现得好不好"的问题，是必须显式处理和文档化的问题。

> **P1 禁止的是"依赖 OS 凭据库"，不是"禁止一切外部前置"。** 用户自备的外部 CLI
> （如 §6 的 `bw`）是可接受的——但它必须被**显式声明为前置依赖**并在启动时检查，
> 而不是假装不存在。这与"需要系统里有 `ssh`"是同一类安排。

---

## 2. 后端（三个）

| 后端 | v1 | 说明 |
|---|---|---|
| **local** | ✅ | 本地 PTY |
| **ssh** | ✅ | 含端口敲门、跳板；认证用密钥池 |
| **serial** | ✅ | 串口（波特率/数据位/校验/流控） |

**共同抽象**：一个**窄**的会话 trait，而不是 PTY 专属 trait。

```
Session: write(bytes) / output_stream() / resize(尽力) / shutdown() / exited()
```

- 能力差异**用 capability flag 表达**，不要用"多几个方法都得实现一遍"。
  serial 没有窗口尺寸、没有信号、没有退出码；SSH 没有本地进程语义。
- 这个抽象必须在 `crates/akasha-pty` 落成**通用**形态，否则第二个后端到来时要重构。

**平台差异（不是"编译一下就有"）**：

| 后端 | Linux | Windows | macOS | Android |
|---|---|---|---|---|
| local | ✅ | ✅ ConPTY | ✅ | ❌ **需独立实现** |
| ssh | ✅ | ✅ | ✅ | ✅ 可共享 |
| serial | ✅ | ✅ | ✅ | ❌ **需 USB Host + JNI** |

- **Android 上的 local 拿不到 `/dev/ptmx`**（SELinux 沙箱）。Termux 能用 PTY 是因为它
  自带 userland bootstrap，不是普通 app 的路径。
- Android 暂定 `later`，v1 **只保证 trait 边界能容纳它，不写任何 Android 代码**。
- 「Android 只做 ssh/sftp」是可接受的降级形态，但必须是**显式决定**，不能是意外结果。

---

## 3. 配置与凭据池

三套独立的池，都存 [`SQLite`](#5-存储与加密)：

| 池 | 内容 | CRUD |
|---|---|---|
| **密钥池**（本地） | 私钥/公钥/备注/关联主机 | ✅ |
| **ssh 配置池** | host/port/user/认证方式/跳板/敲门序列 | ✅ |
| **serial 配置池** | 端口/波特率/数据位/停止位/校验/流控 | ✅ |

- 私钥**默认不落盘到临时文件**：建立连接时在内存中交给 SSH 层，不留中间文件。
- ssh 配置池的字段集**对齐 `~/.ssh/config` 的能力子集**，并保留"从系统 ssh config 导入"
  的位置（导入 ≠ 同步，导入是一次性快照）。
- ⚠️ `~/.ssh/config` 的 `Include` / `Match` / `Host *` 优先级**极难**正确重实现。
  需要解析系统配置时，优先用 `ssh -G <host>`（输出**完全解析后**的有效配置）而不是自己实现。

---

## 4. 文件传输（SFTP）

**形态**：双栏，左右各自从池里选主机（类似 Termius 的 sftp 页）。

| 方向 | v1 | 说明 |
|---|---|---|
| local ↔ host | ✅ | 双向 |
| host ↔ host | ✅ | 见下 |

### 4.1 host↔host 的拓扑（已定：优先直连，失败回退中转）

**前提事实**：SFTP **协议层面没有 server-to-server copy**，只有
`open/read/write/close/stat/rename`。任何两个 SFTP 服务器之间的传输**必须**由客户端驱动，
字节必然流经本机。这不是实现问题，是协议问题。

在这个前提下有两档可选，**两档都实现，运行时优先 B**：

| 档 | 机制 | 本机网络开销 | 前置条件 |
|---|---|---|---|
| **B（优先）** | 在到 A 的 SSH 连接上开 `direct-tcpip` channel 直通 B，SFTP 跑在隧道里 | **1×**（只需上到 A） | A 必须能连到 B |
| **A（回退）** | 内存 relay：读 A → 写 B | 2×（下 A + 上 B） | 无。**永远可用** |

- **B 并不绕开本机**：它把本机带宽砍半，并解除"本机必须能直达 B"的限制。
  真要"数据完全不经过本机"只能靠在 A 上执行 `rsync`/`scp`（需要 A↔B 互信、
  进度只能解析远端输出），**列为非目标**。
- **无论哪档都不落盘**：全程流式，不经临时文件。
- 传输引擎必须支持**并发 in-flight 请求**——多小文件的往返开销会主导耗时，
  拓扑再好也不能弥补串行请求。

---

## 5. 存储与加密

| 项 | 决定 |
|---|---|
| 引擎 | `SQLite`（`rusqlite`），**同步 API + 专用线程**，不用 `sqlx`（SQLCipher 支持弱） |
| 加密 | **SQLCipher**，feature `bundled-sqlcipher-vendored-openssl`（vendored 是必须的：否则交叉编译撞系统 OpenSSL） |
| 密钥来源 | **用户口令 → KDF → 库密钥**（因为 P1 排除了 OS keychain）。口令不落盘 |
| dump | 支持 |
| 导出 | 可选**是否加密**导出文件 |

- **`bundled-sqlcipher` 单独用不够**：必须有 `-vendored-openssl`，否则四平台构建会撞上
  系统 OpenSSL 的交叉编译问题。
- **导出是最高危的操作**，因为库里含私钥：
  - 加密导出：独立口令（不复用库口令），自包含，可直接给另一台机器用
  - 明文导出：**必须显式二次确认 + 明示内容含私钥**，不许作为默认选项
- 库文件是**唯一真相源**，不是缓存。

---

## 6. Bitwarden 集成

| 项 | 决定 |
|---|---|
| 接入方式 | **`bw` CLI**（官方）。**用户自备的系统前置，不由本程序打包** |
| 前置检查 | 探测 `bw` 是否存在；缺失时**明确报"需要安装 Bitwarden CLI"**，不静默失败 |
| 方向 | **v1 只读导入**（Bitwarden → 本池）。**不写回 Bitwarden** |
| 性质 | 独立导入池，**与本地密钥池之间没有任何同步机制** |
| 为什么不是 cache | 叫 "cache" 会诱导后人实现失效/回填/合并逻辑。它是**快照 + 显式导入** |
| 本地池 | 完整 CRUD，**与 Bitwarden 无关，永远可用** |
| 导入池 | 可导入、可删、可改备注；**不回写上游** |
| 离线 | 有加密离线缓存；缓存静态保护强度**必须与本地池同级** |
| 加密 | 由 Bitwarden 负责，本程序**只读写接口**，不实现 vault 密码学 |
| 自实现 vault 加密 | `no` —— 理由见 §9，不在此重复（一处真相） |

> **为什么 `bw` 不打包**：它是约 100 MB 的自包含二进制（Node SEA，好处是不需要
> 另装 Node）。随包分发等于让每个用户为可选功能付体积代价。改为**声明式前置依赖**
> 后，P1 与便携都不再冲突。
>
> **接受的代价**：未安装 `bw` 的机器上，Bitwarden 相关功能**整体不可用**。
> 这是显式选择，不是缺陷——因此前置检查必须给出可读的报错而不是静默禁用。
>
> 另：`bw` 自己的 appdata 可用 `BITWARDENCLI_APPDATA_DIR` 重定向进便携目录。

> **变更记录**：最初设想是"两池之间允许移动或复制（双向）"。现已缩小为
> **v1 只读导入**。理由：写回上游引入并发/冲突/密钥格式兼容三类问题，
> 而"从 Bitwarden 导入到本地池"已覆盖主要使用场景。

### 6.1 新鲜度判定：用 `revisionDate`，**不是** fingerprint

Bitwarden 的 SSH key 条目结构（官方 Rust SDK `bitwarden_exporters::SshKey`）：

```rust
pub struct SshKey {
    pub private_key: String,  // OpenSSH private key, PEM
    pub public_key: String,   // ed25519/rsa, RFC4253
    pub fingerprint: String,  // "SHA256:<base64>"  —— 注意是 SHA256
}
```

- **fingerprint 是 SHA-256**，不是 SHA-512。SHA-512 的关联来自另一个问题
  （Bitwarden SSH agent 对 RSA 密钥一律用 sha512 **签名**）——那是**签名哈希**，
  与指纹是两件事，不要混。
- **fingerprint 不能判断"是否过期"**。它只是公钥的哈希，回答"这是不是同一把钥匙"，
  不回答"这把钥匙新不新"。
- **正确做法**：缓存存 `{item_id, revision_date, fingerprint, 加密私钥}`；
  - `revision_date`（条目元数据）**变化** → 上游改过 → 刷新
  - `fingerprint` → **完整性/同一性**校验（解密出来的私钥是否仍对应同一公钥）
  两个字段职责不同，都要存。

### 6.2 会话与写入范围

- **会话**：由本程序驱动 `bw unlock`（向用户索取主密码），拿到 `BW_SESSION` 后
  **仅在内存中持有**；不落盘、不写进任何环境文件。
- **v1 没有任何 `bw create` / `bw edit` 调用**。这避开了上游写入的并发/冲突/回滚
  问题，也避开了"Bitwarden 不接受某密钥格式"的分支。
- 双向移动/复制明确推迟（见 §9）。

---

## 7. 已识别的风险与设计约束

这三条都是**在写代码前必须处理**的，不是实现细节。

### 风险 1 — 便携 vs. webview：P2/P3 有物理上限

webview 是你不控制的系统组件，它**自己要写可写目录**：

| 平台 | 组件 | 默认写哪 | 能否重定向 |
|---|---|---|---|
| Windows | WebView2 | `%LOCALAPPDATA%\<id>\EBWebView` | ✅ Tauri `WebviewWindowBuilder::data_directory()` |
| Linux | WebKitGTK | `$XDG_DATA_HOME`、dconf | ⚠️ 只能靠 `XDG_*` / dconf 环境变量，且**必须在 GTK 初始化之前**设置 |
| macOS | WKWebView | 系统容器 | ⚠️ `.app` 内**不可写**（破坏代码签名）→ 数据目录必须放在 `.app` **旁边**，不是里面 |

**结论**：P2/P3 对**应用自身的数据**成立，对 webview 运行时只能做到"重定向到我指定的目录"。
必须在启动早期（GTK/webview 初始化前）完成环境重定向，并把这四种情况写进文档。
`%TEMP%` / `/tmp` 仍会被使用——这是 P3 里已经豁免的部分。

> 本项目已经在受限环境里实测撞过这个坑（`just dev` 需要写 `$HOME/.local/share` 与
> `/run/user/1000/dconf`），见 `AGENTS.md` §1 与 `docs/just.md` §6。

### 风险 2 — 便携 vs. 数据目录可写性

"数据存在 bin 所在文件夹" 在以下场景**会失败**：macOS `.app`（只读+签名）、
Linux 装到 `/usr/bin`、Windows 装到 `Program Files`。

**方案**：便携模式**由标记触发**，而非无条件。
- bin 同目录存在数据目录（或便携标记文件）→ 用它
- 否则 → 退回 OS 标准数据目录
- 便携模式下若检测到不可写 → **启动即明确报错**，不要静默退到 OS 目录（那会让用户
  以为数据在 U 盘上，实际在 C 盘）

### 风险 3 — 便携 vs. Bitwarden 接入：**已定案**（2026-09-11）

结论：**`bw` CLI 作为用户自备的系统前置（不打包）+ v1 只读导入**，见 §6。
这样 P1 与便携不再冲突：代价从"每个用户 +100 MB"变成"未装 `bw` 的机器上功能不可用"。

下面四条路留档，避免以后重新论证一遍：

| 方案 | SSH key 条目 | 依赖 | 问题 |
|---|---|---|---|
| **`bw` CLI**（官方） | ✅ 支持 | 自包含二进制，但**约 100 MB**（Node SEA，不需另装 Node） | 无状态模型（须自己管 `BW_SESSION`）；写自己的 appdata（可由 `BITWARDENCLI_APPDATA_DIR` 重定向） |
| **`rbw`**（纯 Rust 非官方） | ❓ 未确认支持新条目类型 | **需外部 `pinentry`** | 作者声明"功能上已对我是完整的"，只修回归不添功能；SSH key 条目是较新的类型 |
| **Bitwarden 官方 Rust SDK** | — | — | 公开的 `bitwarden` crate 是 **Secrets Manager**；含 `SshKey` 的 `bitwarden-exporters`/`bitwarden-vault` 标注 **"Do not use"（internal）** —— 不是可依赖的公开面 |
| 自实现 vault 密码学 | — | — | **已列为非目标**（§6） |

**选它的理由**：它是唯一**官方维护且可靠支持 SSH key 条目**的路径。
`rbw` 更符合"纯 Rust"审美，但它非官方、依赖外部 `pinentry`，且作者已声明只修回归
不添功能——把新条目类型的支持押在它身上风险太高。

**实现前必须实测一次**（目前仍未验证）：`bw` 处理 `sshKey` 条目的具体行为 ——
未解锁时的报错形态、`bw list items --raw` 的 JSON 形状、SSH key 条目是否稳定可见。
这三件决定前置检查与导入逻辑怎么写。

---

## 8. 平台矩阵

| 平台 | v1 | 备注 |
|---|---|---|
| Linux | ✅ | 开发主机 |
| Windows | ✅ | 需验证 ConPTY 与便携 |
| macOS | ✅ | 交叉编译**不可能**，只能 macOS runner / 真机 |
| Android | `later` | 只留 trait 边界，见 §2 |

**CI 平台矩阵要在阶段 1 就建立**，不能像早期 ROADMAP 那样放到收尾——
PTY/serial/ssh 的平台差异是**主体工作量**，把它留到最后等于把最大的坑留在最晚。

**交叉编译现实**（决定工具选型）：

| 目标 | Linux 上 `cargo check --target` | Linux 上出包 | 手段 |
|---|---|---|---|
| Linux | ✅ 原生 | ✅ | — |
| Windows | ✅（`check` 不链接，只需 `rustup target add`） | ❌ | CI `windows-latest` |
| macOS | ✅ | ❌ | CI `macos-latest`（需 macOS SDK + 许可） |
| Android | ✅ | ✅ | `cargo-ndk` + NDK + `tauri android` |

> 因此 **`cargo-xwin` 不是当前需要的工具**：`cargo check --target` 已覆盖"挡 cfg 错误"
> 这个 90% 的诉求。只有当某个依赖的 C build script 在 check 阶段就失败时才引入它
> （它解决的是**链接**问题，救不了 Tauri 的 Windows 打包——WiX/NSIS/WebView2 bootstrapper
> 都要求 Windows 主机）。

---

## 9. 非目标与延后项

**逐条标注是 `no`（永不做）还是 `later`（推迟，可重开）** —— 见开头图例。

| 项 | 类别 | 理由 |
|---|---|---|
| 自实现 Bitwarden vault 密码学 | `no` | §6。错了即灾难；由 Bitwarden 负责 |
| 打包 `bw` CLI | `no` | §6。改为声明式系统前置，避免每个用户 +100 MB |
| host↔host **真不中转**（在 A 上跑 `rsync`） | `no` | §4.1。需 A↔B 互信，进度不可控。B 档（direct-tcpip）已覆盖带宽诉求 |
| 依赖 OS 凭据库 | `no` | P1 |
| 插件系统 | `no` | 每次都会被提，但会与"凭据池不漂移"的约束冲突 |
| 自带运行时（Node/Python）作为应用依赖 | `no` | P1。`bw` 不打包（同上前两条） |
| **写回 Bitwarden**（创建/修改条目） | `later` | §6.2。v1 只读导入；写回引入并发/冲突/格式兼容三类问题 |
| Android 代码 | `later` | §2。v1 只留 trait 边界 |
| 移动端（除 Android 的 ssh/sftp 外） | `later` | 未评估 |

---

## 10. 相关文档

- 下一步做什么 → [`../ROADMAP.md`](../ROADMAP.md)
- 现在到哪了 → [`STATUS.md`](./STATUS.md)
- 为什么这样定 → [`adr/`](./adr/)
- 命令怎么用 → [`just.md`](./just.md)
