# Plan 0110: Windows 上受保护页的锁定额度

- **关联**：ROADMAP 阶段 1 ·「Windows 上受保护页与 SQLCipher 不再争抢同一份额度」
- **前置**：ADR-0009（实现中）；问题 #167
- **状态**：已完成

## 目标 / 非目标

**目标**：在 Windows 上，“库解锁着 + 读一把私钥”（以及任何同时用到 SQLCipher 与 `memsafe`
受保护页的路径）不再因为 `ERROR_WORKING_SET_QUOTA` 失败。

**非目标**：

- 不改数据文件格式、KDF、口令路径、导出容器（ADR-0002 一字不改）。
- 不改 macOS / Linux 的任何行为。
- 不给“锁页失败”加降级路径 —— ADR-0002 D13 已经定死：失败意味着“不能进行”。

## 前置检查

- 现象可复现：`cargo test --test export_contract an_encrypted_export_restores_into_another_directory`
  在本机 Windows 上失败于 `MemoryProtection(MemoryError(Os { code: 1453 }))`。
- 额度可量：一个只做 `VirtualLock` 的探针进程能锁住的字节数与每次锁的粒度无关（都停在 176 KiB）。

## 步骤（每步都能独立验证）

1. **先落一条会红的用例**：`src-tauri/src/store/protected.rs` 的
   `windows_holds_many_protected_pages_at_once`（连续建 32 个 16 KiB 受保护页）。
   验证：加实现之前它失败在 `Os { code: 1453 }`。
2. **实现抬额度**：同一个模块里的 `ensure_working_set()` —— Windows 上调
   `SetProcessWorkingSetSize` 把最小值 / 最大值抬到 16 MiB / 256 MiB，`Once` 保护，
   失败只记一条 `warn`，由 `Protected::new` 调用。验证：用例转绿；`just clippy` 无警告。
3. **全量确认**：`cargo nextest run --workspace --no-fail-fast`。
   验证：`export_contract` 与 `passphrase_contract` 两个目标全绿。

## 验收命令

```bash
# 1. 那颗会红的用例（加实现之前红、之后绿）
cd src-tauri && cargo test --lib -p akasha windows_holds_many_protected_pages_at_once

# 2. 曾经红的两条目标
cd src-tauri && cargo nextest run --test export_contract --test passphrase_contract
```

预期：第 1 条 `1 passed`；第 2 条 `18 tests run: 18 passed`。

## 回滚

删掉 `src-tauri/src/store/protected.rs` 里的两条常量、那个 `extern` 块与 `ensure_working_set()`，
并去掉 `Protected::new` 里那一行调用。回滚之后 `windows_holds_many_protected_pages_at_once`
会重新变红 —— 那正是它存在的意义。

## 实施记录

- 2026-09-20：探针实测额度 = 进程最小工作集。4 KiB 页 44 次、16 KiB 页 11 次、32 KiB 页 5 次
  （都是 176 KiB）；`SetProcessWorkingSetSize(…, 16 MiB, 4 GiB)` 之后同一份代码能锁 1022 个
  16 KiB 页；`(0, 0)` 返回 `ERROR_INVALID_PARAMETER`。
- 2026-09-20：新用例先红（`Os { code: 1453 }`），加实现之后转绿。
- 2026-09-20：`export_contract` 与 `passphrase_contract` = 18/18；全量 `--no-fail-fast`
  从「423 条里 4 红」变成「423 条全过」（另两条见 plan 0111）。
- 2026-09-20：修好之后在同一处量到的需求侧 —— `open(restored)` 之后还剩 1006 个 16 KiB 页
  可锁，SQLCipher 与我们的页合起来约占 288 KiB。
