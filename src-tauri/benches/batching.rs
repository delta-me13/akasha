//! 合批的**吞吐基线**（criterion）。
//!
//! 这组数字**不是门禁**（`AGENTS.md` §7）：MB/s 随机器、编译器版本、是否插电而变，
//! 拿它当通过条件只会得到一条随机红的流水线。它的用处是**改动前后能对比** ——
//! 合批正落在每个字节的必经之路上，一次"顺手多一次分配/多一次拷贝"就能让吞吐腰斩，
//! 而单测只看正确性，一点都看不出来。
//!
//! 跑法：`just bench`（或 `cargo bench -p pty`）。实测数字记在
//! `docs/plans/archive/0201-output-batching.md` 与 `docs/STATUS.md`。

use std::hint::black_box;
use std::io::{self, Read};
use std::time::{Duration, Instant};

use akasha_lib::pty::{BatchPolicy, OutputBatcher, spawn_batcher};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

/// 读线程一次搬多少 —— 与 crate 内部的常量同值，好让每条 `read` 都正好是一个满块。
const READ_CHUNK: usize = 64 * KIB;

/// 纯合批器：4 KiB 块（≈ 一次 PTY 读的常见规模）一路喂到**容量**触发。
///
/// 关心的是每字节的搬运成本：`extend_from_slice` + 每批一次 `Vec` 所有权转移。
fn capacity_path(c: &mut Criterion) {
    let chunk = vec![0x41u8; 4 * KIB];
    let total = 4 * MIB;
    let mut group = c.benchmark_group("batching/capacity");
    group.throughput(Throughput::Bytes(total as u64));
    group.bench_function("4kib_chunks", |b| {
        b.iter(|| {
            let mut batcher = OutputBatcher::new(BatchPolicy::DEFAULT);
            // **假时钟**：每块推进 1 µs，16 块才 16 µs —— 离 16 ms 的期限差三个数量级，
            // 所以这条路径上只可能发生容量触发，与真实时钟无关（不会 flaky，也不会等）。
            let mut now = Instant::now();
            let mut delivered = 0usize;
            for _ in 0..(total / chunk.len()) {
                now += Duration::from_micros(1);
                if let Some(batch) = batcher.push(black_box(&chunk), now) {
                    delivered += batch.bytes.len();
                }
            }
            if let Some(batch) = batcher.flush() {
                delivered += batch.bytes.len();
            }
            black_box(delivered)
        });
    });
    group.finish();
}

/// 每**批**的固定开销：一块一交付（`max_bytes = 1`，等价于把合批关掉）。
///
/// 这是最坏情况，而它并非虚构 —— 时间触发在真实场景里也会走到形状相同的路径
/// （稀疏输出、提示符）。按 Elements 报，读出来的就是"每批多少 ns"。
fn per_batch_overhead(c: &mut Criterion) {
    let chunk = [0x41u8; 1];
    let batches = 64 * KIB;
    let policy = BatchPolicy {
        max_bytes: 1,
        max_delay: Duration::from_secs(60),
    };
    let mut group = c.benchmark_group("batching/overhead");
    group.throughput(Throughput::Elements(batches as u64));
    group.bench_function("one_byte_per_batch", |b| {
        b.iter(|| {
            let mut batcher = OutputBatcher::new(policy);
            let now = Instant::now();
            let mut delivered = 0usize;
            for _ in 0..batches {
                if let Some(batch) = batcher.push(black_box(&chunk), now) {
                    delivered += batch.bytes.len();
                }
            }
            black_box(delivered)
        });
    });
    group.finish();
}

/// 端到端：[`spawn_batcher`] 的真实形状（读线程 → 有界队列 → 合批线程 → 批次队列）。
///
/// 与上面两条的差值就是**线程与 channel 的代价** —— 那才是接进 app 之后实际跑的东西。
/// 块取满 64 KiB，每次都是容量触发，耗时与 16 ms 的期限无关。
fn end_to_end(c: &mut Criterion) {
    let total = 8 * MIB;
    let mut group = c.benchmark_group("batching/end_to_end");
    group.throughput(Throughput::Bytes(total as u64));
    group.bench_function("64kib_chunks", |b| {
        b.iter(|| {
            let reader: Box<dyn Read + Send> = Box::new(Pattern {
                remaining: total,
                chunk: vec![0x41; READ_CHUNK],
            });
            let batches = spawn_batcher(reader, BatchPolicy::DEFAULT);
            let mut bytes = 0usize;
            let mut count = 0usize;
            for batch in batches {
                bytes += batch.bytes.len();
                count += 1;
            }
            black_box((bytes, count))
        });
    });
    group.finish();
}

/// 合成读端：连续给 `chunk` 大小的块，共 `remaining` 字节，然后 EOF。
///
/// 不落盘、不真的起进程 —— 基准要量的只有搬运，I/O 会把这个信号淹掉。
struct Pattern {
    remaining: usize,
    chunk: Vec<u8>,
}

impl Read for Pattern {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let n = buf.len().min(self.chunk.len()).min(self.remaining);
        buf[..n].copy_from_slice(&self.chunk[..n]);
        self.remaining -= n;
        Ok(n)
    }
}

criterion_group!(benches, capacity_path, per_batch_overhead, end_to_end);
criterion_main!(benches);
