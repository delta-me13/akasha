# Plan 0201: `Transport` 输出合批（≥16ms 或 ≥64KiB）

- **关联**：ROADMAP 阶段 2 ·「`Transport` 输出合批」
- **前置**：plan 0105（`Transport` trait 与假实现）
- **状态**：已完成（2026-09-11）
- **影响面**：`src-tauri/crates/akasha-pty`（新增 `src/batcher.rs`、`benches/batching.rs`）；`justfile`（新增配方）

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
cd src-tauri                                    # 仓库根没有 manifest（坑 #8）
cargo nextest run -p akasha-pty                 # 期望先绿（不要在红的基线上开工）
grep -rn '16ms\|64KiB\|64 \* 1024' ../docs/ ../AGENTS.md | head
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
cd src-tauri
cargo nextest run -p akasha-pty batcher   # 期望全绿（12 条合批边界/不变式单测）
cargo bench -p akasha-pty                 # 期望跑出吞吐基线数字（约 1 分钟）
grep -n 'emit' crates/akasha-pty/src/*.rs # 期望无输出：没有逐块交付的路径
```

（`just bench` 是上面第二条的常规入口；`just test` / `just ready` 覆盖第一条。）

**判据**：合批边界有单测断言，且**有吞吐基线数字**（ROADMAP）。数字记进本文与 `docs/STATUS.md`。

## 回滚

合批器是新增模块，删除即可；若已接入调用点，回退到直通路径。

## 实施记录

（2026-09-11，CachyOS / rustc 1.98.1 / release 模式，未插电状态未记录）

### 吞吐基线（`just bench`，criterion 0.8.2）

| 基准 | 中位数 | 换算 |
|---|---|---|
| `batching/capacity/4kib_chunks` | 74.08 µs / 4 MiB | **52.7 GiB/s** |
| `batching/overhead/one_byte_per_batch` | 923.4 ns / 批 | **14.1 ns/批** |
| `batching/end_to_end/64kib_chunks` | 810.8 µs / 8 MiB | **9.64 GiB/s** |

读法：前两条是合批器**纯逻辑**的上下界（52.7 GiB/s 的搬运上限、14 ns 的每批固定成本），
第三条是含线程与两个 channel 的**真实形状** —— 与第一条的差距就是那套管道的代价
（8 MiB 全在 L2/L3 里，所以第一条偏乐观；真实数据要从内核搬进用户态，两条都够用）。
整组跑完约 30 s（criterion 的统计开销占大头，不是被测代码慢）。

**复跑一次**（同机、同二进制、隔了几分钟）：51.1 GiB/s / 14.4 ns 每批 / 9.37 GiB/s ——
三组都在 3% 以内浮动，方向上还有正有负。这正是「基线不是门禁」的实证
（`AGENTS.md` §7）：同一份代码复跑都有这个量级，拿它当通过条件只会得到随机红。

### 手段上的一处追加：`spawn_batcher`（计划外，但必要）

计划只写了合批器本体（`push` / `poll_due` / `flush`）。落地时发现**光有它接不上东西**：

`Box<dyn Read + Send>` 是阻塞的，也没有可移植的超时接口。若读循环"读一块→喂一块"，
**时间触发永远不会单独发生** —— 它会退化成"下一块字节到来时才顺便交付"。这不是
理论问题：shell 打完提示符 `> ` 之后就没有输出了，用户正**等这个提示符**，而它会
一直躺在缓冲里直到用户按键。只按容量合批的实现在交互场景是坏的。

所以补了一个驱动 `spawn_batcher`：读线程只把块搬进有界队列（8 × 64 KiB，带背压），
合批线程用 **`recv_timeout(期限)`** 等 —— 字节先到就走容量/时间那两条，
期限先到就走时间那条。它等的是 channel 的期限，**不是 `sleep`**（`AGENTS.md` §0 禁的是后者）。

由此，边界数字全部由**注入假时钟**的纯逻辑单测钉住（精确到 1 ms / 1 字节）；
驱动那三条只断言"内容"与"最终一定会交付"（5 s 上限等 50 ms 的事），不看调度时序 ——
否则就是在用调度运气当断言。

### 验收命令的实测输出

```
cargo nextest run -p akasha-pty batcher   → 12 tests run: 12 passed, 14 skipped
cargo bench -p akasha-pty                 → 见上表；另有 `0 passed; 26 ignored`（libtest 在 bench 模式下不跑 #[test]，正常）
grep -n 'emit' crates/akasha-pty/src/*.rs → 无输出
grep -rn 'write_all'                       → 只有 pty.rs:82（**输入**路径：把用户按键写进载体，与输出合批无关）
just test                                  → 39 tests run: 39 passed（新增 12 条）
```

`cargo nextest run -p akasha-pty batching` 这条原本的过滤器**一条都匹配不到**
（模块名是 `batcher`，用例名里也没有 `batching`）—— 而 nextest 在零匹配时是
**报错**：`error: no tests to run`。过滤器是手段、会随模块改名失效，判据是"12 条边界
单测全绿"。已改成 `batcher`（`docs/STATUS.md` 坑 #27）。

### 与计划的偏差

- **`bench` 成为配方**（新增 `just bench`，配方 19 → 20）：基线要反复跑、要能对比，
  就必须有一个被 `docs-check` 登记的入口，不能只活在 plan 的代码块里。
- 顺带修了 plan 0105 以来一直照抄的 `cargo …` 写法：仓库根没有 manifest（坑 #8），
  验收命令得先 `cd src-tauri`（或走 `just` 转发）。
- `Trigger`（这一批为什么被交出来）是计划外的小追加：**只断言字节内容的话，
  "容量"与"时间"两条触发在字节上完全一样**，实现写反了测试照样绿。
