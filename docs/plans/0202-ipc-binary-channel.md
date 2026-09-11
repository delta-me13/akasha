# Plan 0202: IPC 二进制通道（`Channel<Vec<u8>>`）

- **关联**：ROADMAP 阶段 2 ·「IPC 二进制通道」
- **前置**：plan 0201（合批器已就绪）
- **状态**：未开始
- **影响面**：`src-tauri/src/**`（command + Channel）、`src/ipc/**`（生成层）、`.ast-grep/rules/`

## 目标

把合批后的字节流经 **`tauri::ipc::Channel<Vec<u8>>`**（或 raw body）送到前端，
**不走默认 JSON IPC** —— 默认路径会把字节流序列化成数组/字符串，吞吐直接崩（`AGENTS.md` §3.2）。

同时把这条路径**变成结构性规则**，而不是靠自觉：
`no-string-pty-channel`（拦截 `Channel<String>` 承载 PTY 字节流）。

## 非目标

- **不**做渲染（plan 0203）
- **不**在手写层碰 IPC：前端只走 `src/ipc/` 生成层，禁止裸 `invoke("...")`（`AGENTS.md` 绝对禁止 #1）
- **不**在 `src-tauri` 里写业务逻辑（薄壳原则）

## 前置检查

```bash
cargo metadata --no-deps --format-version 1 >/dev/null && echo OK
ls src/ipc/ 2>/dev/null || echo "生成层尚未存在（tauri-specta 未接入，见 ROADMAP 阶段 3 附注）"
just gen-types        # 期望在当前状态下有明确行为（生成或提示待接入）
```

## 步骤

1. 定义 command：启动一个会话并返回 `Channel<Vec<u8>>`（或让前端把 Channel 传进来），
   批次来自 plan 0201 的合批器。
2. `src-tauri/src/**` 只做编组：把 `Transport` 的批次接到 Channel 上，不解析、不解码、不缓存业务状态。
3. 前端侧：新增接收包装函数（放 `src/ipc/`），**接收即写入 xterm 缓冲**，不进 React state。
4. 落地 ast-grep 规则 `no-string-pty-channel`：
   - 先写负例（临时 `Channel<String>`）确认规则**变红**，再删负例确认**转绿**（`AGENTS.md` §6）
5. 走一遍真实路径（Victauri）：`invoke_command` → `wait_for` 到真正结束 → 断言前后端状态一致。
   **禁止用 sleep 猜测**（`AGENTS.md` §7）。

## 验收命令

```bash
# 1. 大输出不打崩：在终端里跑一条产生 ≥10MB 输出的命令，前端应持续渲染
#    （例如 seq / yes | head -c 10000000；具体命令由实现者按 shell 环境选定并记录）
#    判据：无掉帧卡死、无内存暴涨、进程可正常结束

# 2. 生成层与 Rust 一致
just gen-types && git diff --exit-code src/ipc/   # 期望为空（生成物已提交）

# 3. 结构护栏
ast-grep scan

# 4. 真实路径（Victauri，app 需在跑）
just dev        # 另开终端
just doctor     # 期望识别到本项目
```

**规则负例自检**（用 `cp` 备份还原，别用 `git checkout`，坑 #12）：

```bash
cp src-tauri/src/<该 command 所在文件> /tmp/ch.rs.bak
# 把 Channel<Vec<u8>> 临时改成 Channel<String>
ast-grep scan        # 期望报告 no-string-pty-channel
cp /tmp/ch.rs.bak src-tauri/src/<该 command 所在文件>
```

## 回滚

回退 command 与前端接收层、删除规则；无数据影响。回滚后前端会失去输出通道 —— 与开工前等价。

## 实施记录

（边做边追加：记录大输出的**实际字节数、耗时与内存表现**。）
