# 便携性：约束、重定向杠杆与验证方法

> 本文是**参考资料**，不是规则。规则（P1/P2/P3 三条定位性约束）在
> [`scope.md`](./scope.md) §1。
>
> 为什么单独一份：这里列举的写入路径会随着我们在各平台实测而**增长**，
> 而 `scope.md` 的能力清单不该跟着长。两者的时效性不同。
>
> **核心命题**：*"不写其他文件夹"是一个可验证的断言，不是一句设计意图。*
> 所以本文的重点不是罗列，而是最后那节**怎么证明它**。

---

## 1. 前提：webview 不是一个组件

用户明确豁免了 webview。但 **webview 连带触发六类写入，只有第 1 类真正来自它自己**。

| # | 来源 | 平台 | 默认写哪 | 重定向手段 |
|---|---|---|---|---|
| 1 | webview 自身数据目录 | Windows | `%LOCALAPPDATA%\<id>\EBWebView` | ✅ `WebviewWindowBuilder::data_directory()` / `WEBVIEW2_USER_DATA_FOLDER` |
| | | Linux | `$XDG_DATA_HOME` / `$XDG_CACHE_HOME` | ✅ `XDG_*` |
| | | macOS | 系统容器 | ⚠️ 见 §3 |
| 2 | **GTK / GLib / dconf 设置**<br>（webview 的依赖栈，不是 webview） | Linux | `~/.config/dconf`、`/run/user/<uid>/dconf` | ✅ `XDG_CONFIG_HOME` + dconf profile / `GSETTINGS_BACKEND` |
| 3 | **GPU 驱动 shader cache**<br>（我们必踩：`addon-webgl` 是 `AGENTS.md` §4.1 指定的渲染器） | Linux | `$XDG_CACHE_HOME/mesa` | ✅ 落在 `XDG_CACHE_HOME` 下，跟着 #1 一起被收走 |
| | | Windows | `%LOCALAPPDATA%\D3DSCache` 等 | ⚠️ 驱动层，需逐项确认 |
| 4 | fontconfig 缓存 | Linux | `~/.cache/fontconfig` | ✅ `XDG_CACHE_HOME` |
| 5 | **AppKit 窗口状态保存**<br>（AppKit 的行为，不是 webview） | macOS | `~/Library/Saved Application State/<bundleid>.savedState` | ⚠️ `CFFIXED_USER_HOME` 是 CFPreferences 的覆盖手段，**需实测** |
| 6 | 临时文件 | 全平台 | `%TEMP%` / `/tmp` / `$TMPDIR` | ⚠️ 已由 P3 豁免，但要确保**只有它** |

**好消息**：Linux 上 #1 / #3 / #4 全部落在 `XDG_*` 之下 —— **重定向 `XDG_*` 一个杠杆
就收走大半**，不需要逐项处理。

**坏消息**：#5 在 macOS 上**不在你说的"webview 除外"里**，而且没有可靠的关闭开关。

> Mesa 的 shader cache 路径来自其官方 envvar 文档：默认 `$XDG_CACHE_HOME/mesa`，
> 可用 `MESA_SHADER_CACHE_DIR` / `MESA_SHADER_CACHE_DISABLE` 覆盖
> （[Mesa envvar 补丁](https://lists.freedesktop.org/archives/mesa-dev/2017-January/141746.html)，
> 该版本用的变量名是 `MESA_GLSL_CACHE_*`，现行名已改为 `MESA_SHADER_CACHE_*`）。
> **因为它就住在 `XDG_CACHE_HOME` 下，我们不需要单独处理它。**

---

## 2. 时序：晚一步就无效，且失败是静默的

**所有环境变量必须在进程极早期设置** —— 在 GTK / AppKit / webview 初始化之前。
一旦相关库已经读过环境变量，后续修改无效，而**现象是数据悄悄写到了默认位置**，
不会报错。

这意味着重定向不能写在 `tauri::Builder` 的 `setup` 回调里（太晚），
必须在 `main()` 的最前面，或者由一个启动器脚本/包装二进制完成。

> **本项目已经实测撞过这个坑**：在受限环境里 `just dev` 需要写 `$HOME/.local/share`
> 与 `/run/user/1000/dconf`，表现为 `Failed to setup app: 只读文件系统 (os error 30)`，
> 而它发生在 `cargo build` **成功之后**——极易被误判成"编译过了但跑不起来"。
> 见 `AGENTS.md` §1 与 [`just.md`](./just.md) §6。

---

## 3. 数据目录位置的平台差异

"数据存在 bin 所在文件夹" 这句话在三个平台上含义不同：

| 平台 | "bin 所在文件夹" 实际是 | 注意 |
|---|---|---|
| Linux | 二进制所在目录 | 装到 `/usr/bin` 时不可写 → 退回 OS 目录 |
| Windows | `.exe` 所在目录 | 装到 `Program Files` 时不可写 → 退回 OS 目录 |
| macOS | **`.app` 旁边的目录** | ⚠️ **`.app` 内部不可写**：会破坏代码签名 |

**因此便携模式必须由标记触发，而非无条件**：

1. bin 同目录存在数据目录（或便携标记文件）→ 用它
2. 否则 → 退回 OS 标准数据目录
3. 便携模式下若检测到**不可写** → **启动即明确报错**

第 3 条是重点：**不要静默退回 OS 目录**。那会让用户以为数据在 U 盘上、实际写在 C 盘 ——
这是最糟的失败模式（用户把 U 盘拔了，数据没了，而他不知道）。

---

## 4. 其他系统组件依赖（与写入路径无关，但同属"不依赖系统组件"）

| 依赖 | 平台 | 可避免？ |
|---|---|---|
| WebKitGTK / GTK3 / GLib / fontconfig | Linux | ❌ 不可避免（用 Tauri 就得有）—— 属 webview 豁免范围 |
| WebView2 runtime | Windows | ❌ 同上（Evergreen 由系统提供） |
| **`libudev`**（`serialport` 的端口枚举） | Linux | ⚠️ **待确认**能否关掉。这是 webview 之外的独立系统库 |
| OpenSSL | 全平台 | ✅ **可避免，且应当避免**：SQLCipher 用 `bundled-sqlcipher-vendored-openssl`；SSH 选 `russh`（`ring`/`aws-lc`）而非 `ssh2`（libssh2 → OpenSSL） |
| `bw` CLI | 全平台 | 已定案为**刻意的例外**，见 [`scope.md`](./scope.md) §6 |
| 系统 `ssh` | 全平台 | ❓ **与纯 Rust SSH 方案冲突**，见 §5 |

> 注意 OpenSSL 那一行：它**不是不可避免，而是我们自己选出来的**。
> 选错 SSH 库 + 忘了 vendored feature，就会平白多一个跨平台链接负担。
> 这正是 `scope.md` 要求 SSH 选型走 ADR 的原因之一。

---

## 5. `ssh -G` 与"不依赖系统组件"的冲突

`ssh -G <host>` 能输出**完全解析后**的 ssh config（`Include` / `Match` / `Host *`
优先级极难自己正确重实现），是一条很有价值的技巧。

**但它要求系统里有 `ssh` 二进制** —— 与"不依赖系统组件"直接冲突。三条出路：

| 出路 | 依赖系统 `ssh` | ssh config 解析 |
|---|---|---|
| 1. 声明 OpenSSH 为前置依赖 | ❌ 需要 | 白捡且完全正确。但 SSH 是**核心**功能（不像 Bitwarden 可选），把核心建立在外部二进制上风险更大 |
| 2. 纯 Rust，且不支持导入系统 config | ✅ 不需要 | 范围最小，但用户体验明显受损 |
| 3. 纯 Rust 为默认，检测到 `ssh` 时才提供"导入" | ⚠️ 可选 | 折中；须接受两条解析路径的差异 |

**倾向 3。** 这条必须写进 SSH 的 ADR —— 它决定要不要实现一个 ssh config 解析器，
而那是**本项目最容易被低估的一块复杂度**。

---

## 6. 怎么验证（本文最重要的一节）

"不写别处"可以被证明或证伪，所以按 `AGENTS.md` §8 的原则，**不要只把它写成散文**。

| 平台 | 方法 |
|---|---|
| **Linux** | `strace -f -e trace=openat -e status=successful`，过滤 `O_WRONLY` / `O_CREAT` / `O_APPEND`；或用 `bwrap` 只读绑定 `$HOME` 后运行 |
| Windows | Process Monitor，过滤 `CreateFile` + `Write`；或把 `%USERPROFILE%` 设为只读后运行 |
| macOS | `fs_usage`；或用 `sandbox-exec` 以拒绝写规则启动 |
| **CI 近似（可自动化）** | 把 `HOME` / `XDG_*` / `TMPDIR` 指向一个临时目录，断言应用的全部写入都落在该目录 + 白名单 temp 之内 |

> **本项目已经无意中做过 Linux 那一行的实验**：沙箱拒绝工作区外写入，
> 于是暴露了 `$HOME/.local/share` 与 dconf 两条路径。那次"失败"就是一次便携性实测。

**目标形态**（**尚未实现**，待列入 `ROADMAP.md`）：一条名为 `portable-check` 的配方，
形态类似 `just syscheck` —— 至少覆盖上表"CI 近似"那一行，本机则跑更严格的版本。

**验收标准：白名单之外零写入。** 白名单目前只有 `%TEMP%` / `/tmp` / `$TMPDIR`。
