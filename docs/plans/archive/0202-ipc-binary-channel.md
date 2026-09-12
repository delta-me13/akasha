# Plan 0202: IPC 二进制通道（`Channel<Vec<u8>>`）

- **关联**：ROADMAP 阶段 2 ·「IPC 二进制通道」
- **前置**：plan 0201（合批器已就绪）
- **状态**：已完成（2026-09-11）
- **影响面**：`src-tauri/src/**`（command + Channel + 生成器）、`src/ipc/**`（生成层 + 接收层）、
  `.ast-grep/rules/`、`justfile`（生成与比对配方）

## 目标

把合批后的字节流经 **`tauri::ipc::Channel<Vec<u8>>`**（或 raw body）送到前端，
**不走默认 JSON IPC** —— 默认路径会把字节流序列化成数组/字符串，吞吐直接崩（`AGENTS.md` §3.2）。

同时把这条路径**变成结构性规则**，而不是靠自觉：
`no-string-pty-channel`（拦截 `Channel<String>` 承载 PTY 字节流）。

## 非目标

- **不**做渲染（plan 0203）
- **不**在手写层碰 IPC：前端只走 `src/ipc/` 生成层，禁止裸 `invoke("...")`（`AGENTS.md` 绝对禁止 #1）
- **不**在 `src-tauri` 里写业务逻辑（薄壳原则）
- **不**做渲染：本 plan 验到"字节以 raw 形态到达 JS 且分批正确"为止，
  「大输出持续渲染」由 plan 0203 的验收覆盖（本 plan 的 验收命令 #1 原本把两件事写在一起）

## 前置检查

```bash
cargo metadata --no-deps --format-version 1 >/dev/null && echo OK
ls src/ipc/ 2>/dev/null || echo "生成层尚未存在（tauri-specta 未接入，见 ROADMAP 阶段 3 附注）"
just gen-types        # 期望在当前状态下有明确行为（生成或提示待接入）
```

## 步骤

1. 定义 command：启动一个会话并让**前端把频道传进来**（tauri 的频道由前端创建，
   后端只收 id），批次来自 plan 0201 的合批器。
   ⚠️ 实际用的是 `Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`，
   **不是**计划里写的 `Channel<Vec<u8>>` —— 后者是 JSON 数组（见「实施记录」）。
2. `src-tauri/src/**` 只做编组：把 `Transport` 的批次接到 Channel 上，不解析、不解码、不缓存业务状态。
3. 前端侧：新增接收包装函数（放 `src/ipc/`），**接收即写入 xterm 缓冲**，不进 React state。
4. 落地 ast-grep 规则 `no-string-pty-channel`：
   - 先写负例（临时 `Channel<String>`）确认规则**变红**，再删负例确认**转绿**（`AGENTS.md` §6）
5. 走一遍真实路径（Victauri）：`invoke_command` → `wait_for` 到真正结束 → 断言前后端状态一致。
   **禁止用 sleep 猜测**（`AGENTS.md` §7）。

## 验收命令

```bash
# 1. 大输出走 raw 通道（真 app、真 PTY、10 MB）
just dev                                        # 另开终端常驻
cd src-tauri
VICTAURI_E2E=1 cargo test --test session_channel -- --test-threads=1
# 期望：raw 通道送达 ≥10000000 字节；批次数远小于字节数/1024；全程无错误

# 2. 生成层与 Rust 一致（已并入 just ready）
just gen-types-check      # 期望：生成物已提交，无输出

# 3. 结构护栏（含新规则）
ast-grep scan             # 期望退出码 0
just test                 # 期望全绿（含 app 侧 5 条会话单测）
```

**规则负例自检**（用 `cp` 备份还原，别用 `git checkout`，坑 #12）：

```bash
# 在 src-tauri/src/ 下临时放一个探针文件，同时写命中的与被诱饵的：
#   Channel<String> / Channel<Vec<u8>>                       ← 应当命中
#   Channel<InvokeResponseBody> / Channel<Previewer>          ← 必须**不**命中
ast-grep scan --rule .ast-grep/rules/no-string-pty-channel.yml src-tauri/src/probe_rule.rs
rm src-tauri/src/probe_rule.rs
```

## 回滚

回退 command 与前端接收层、删除规则；无数据影响。回滚后前端会失去输出通道 —— 与开工前等价。

## 实施记录

（2026-09-11，CachyOS / rustc 1.98.1）

### 最重要的一条：计划里写的 `Channel<Vec<u8>>` 是**错的**

它看起来就是"二进制通道"，但 tauri 有一条

```rust
impl<T: Serialize> IpcResponse for T      // ← blanket impl，Vec<u8> 也命中
```

于是 `Channel<Vec<u8>>::send(bytes)` 走的是 `serde_json::to_string` ——
一个 64 KiB 的批次变成"六万多个数字的 JSON 数组"，**正是 `AGENTS.md` §3.2 说要禁止的
那条慢路**。规范与 plan 都把这种写法当成"正确做法"写了下来。

真正走 raw 的只有 `Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw(bytes)`：
小包经 eval 送成 `ArrayBuffer`，大包走 fetch 通道；JS 侧 `new Uint8Array(payload)`。

连带两个后果，都记在 `docs/STATUS.md` 的坑里：

* **`InvokeResponseBody` 没有 `specta::Type`** → 生成器写不出它的 TS 类型。
  所以命令参数声明成"频道句柄是个字符串"（`RawChannel(String)`，线上本来就是
  `__CHANNEL__:<id>`），Rust 侧用 `JavaScriptChannelId::from_str` + `channel_on` 还原成
  raw 频道；"怎么建频道"那一小段留在 `src/ipc/session.ts`（`AGENTS.md` §5 已登记这处例外）。
* **`u64` 不能直接过 IPC**：生成器拒绝它（BigInt 精度），而"危险地当 number 用"是全局开关。
  改用壳层的 `u32` 句柄 + **checked** 转换 —— 截断会把用户的按键送进另一个会话。

### 大输出实测（真 app + 真 PTY）

```
open_session → 1
write_session → true
✅ raw 通道送达 11383949 字节，分 181 批（等待耗时 1651 ms）
```

181 批 / 11.38 MB ≈ 63 KiB 每批 —— 与 plan 0201 的 64 KiB 容量触发吻合，
说明"逐块交付"没有回来（用例里也钉了断言：`batches < bytes / 1024`）。
1.65 s 的等待里绝大部分是 `yes` 在往 PTY 里写 10 MB，不是这条通道。

### 顺手修掉的两个坑（都不是本 plan 的目标，但不修走不下去）

* **仓库里出现第二个 bin 后，`tauri dev` 直接失败**：
  `cargo run could not determine which binary to run`。症状极具迷惑性 ——
  Vite 起得来、tauri 开始监听、看起来"什么都正常"，但 app 根本没启动。
  修法是 `default-run = "akasha"`。
* **只写 `path` 的依赖会被 `cargo deny` 判成 wildcard**（`wildcards = "deny"`）：
  `found 2 wildcard dependencies for crate 'akasha'`。path 依赖要同时写 `version`。

### 验收命令的实测输出

```
VICTAURI_E2E=1 cargo test --test session_channel  → 1 passed（见上面的字节数）
just gen-types-check                              → 通过（纳入 ready，6/6 全绿）
ast-grep scan                                     → 退出码 0（新规则已用正负例验证）
just test                                         → 45 tests run: 45 passed
```

⚠️ 沙箱里跑 E2E 有个**环境约束**：每次 bash 调用是独立的 bwrap（私有 PID / 临时目录），
所以 `just dev` 与 E2E 用例**必须在同一次调用里**起，否则找不到 Victauri 的发现文件。
本机（非沙箱）分开跑没有这个问题。

### 与计划的偏差

* 验收命令 #1 里的"前端应持续渲染"归 **plan 0203**（本 plan 明说不做渲染）。
  这里验到的是"raw 字节到底、分批正确、无错误"。
* 计划没写生成器接入（`just gen-types` 当时还是 TODO 桩），但它是本 plan 的前提 ——
  没有它，前端只能手写第二份签名，等于违反 §5。已在本 plan 内落地并纳入 `ready`。
