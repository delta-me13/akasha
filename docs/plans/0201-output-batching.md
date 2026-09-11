# Plan 0201: `Transport` 输出合批（≥16ms 或 ≥64KiB）

- **关联**：ROADMAP 阶段 2 ·「`Transport` 输出合批」
- **前置**：plan 0105（`Transport` trait 与假实现）
- **状态**：未开始
- **影响面**：`crates/akasha-pty`（或 `crates/akasha-core`）中的合批器；`benches/`

## 目标

读循环按「**≥16ms 或 ≥64KiB**」聚合一次再交付，**禁止逐字节 / 逐行 emit**（`AGENTS.md` §3.2）。

合批器本身是**纯逻辑**（输入：字节块 + 时钟；输出：批次），因此可以脱离 PTY 与 Tauri 做确定性单测 ——
这是本 plan 的主要可测性来源。

## 非目标

- **不**接 IPC / 前端（plan 0202 才把批次送过边界）
- **不**做解码：批次仍是 `&[u8]` / `Vec<u8>`，**绝不假设 UTF-8**（字节流可被切在多字节序列中间）
- **不**引入 async runtime 决策（这条属于阶段 2 的实现细节，但不要顺手引入 `tokio`）

## 前置检查

```bash
cargo nextest run -p akasha-pty     # 期望先绿（不要在红的基线上开工）
grep -rn '16ms\|64KiB\|64 \* 1024' docs/ AGENTS.md | head
```

## 步骤

1. 定义合批器接口：`push(chunk)` / `flush()` / `poll_due(now)` —— **时钟由外部注入**，
   单测里不要用真实 `sleep`（`AGENTS.md` 禁止用固定 sleep 等待异步完成）。
2. 两条触发条件：
   - **容量**：累计 ≥ 64KiB → 立刻交付
   - **时间**：距本批首字节 ≥ 16ms → 交付
3. EOF / 错误 / `shutdown` 路径必须 `flush` 残批（**不要丢尾巴**）。
4. 参数集中定义（常量或配置项），不要散落在 read loop 里。
5. bench（criterion）：合批吞吐基线（MB/s）与每批开销，结果写进「实施记录」。
6. 单测边界：正好 64KiB；跨过 64KiB 的大块（一次交付一块，不切碎导致额外拷贝）；
   16ms 边界；EOF 时残批被交付；空输入不产生空批次。

## 验收命令

```bash
cargo nextest run -p akasha-pty batching      # 期望全绿（合批边界单测）
cargo bench -p akasha-pty                     # 期望跑出吞吐基线数字
grep -rn 'emit\|write_all' crates/akasha-pty/src | grep -v batcher   # 目视：确认没有逐字节路径
```

**判据**：合批边界有单测断言，且**有吞吐基线数字**（ROADMAP）。数字记进本文与 `docs/STATUS.md`。

## 回滚

合批器是新增模块，删除即可；若已接入调用点，回退到直通路径。

## 实施记录

（边做边追加：记录 bench 的**实际数字**与运行环境。）
