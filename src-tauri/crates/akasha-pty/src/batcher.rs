//! 输出合批：把零碎字节块攒成「批次」再交付。
//!
//! 为什么必须合批（`AGENTS.md` §3.2）：PTY 的读端返回的是**内核缓冲里碰巧有多少**，
//! 一次 `read` 可能是 1 个字节（用户敲了一个键），也可能是 64 KiB（`yes` 在刷屏）。
//! 逐块往上传，等于让下游按"内核怎么切"来决定 IPC 消息数量 —— 大输出时消息数爆炸，
//! 小输出时延迟白担。
//!
//! 两条交付条件，**任一条先到就先交**：
//!
//! | 条件 | 值 | 它守的是什么 |
//! |---|---|---|
//! | 容量 | ≥ [`BatchPolicy::max_bytes`]（默认 64 KiB） | 大输出时的消息数量与内存占用 |
//! | 时间 | 距**本批首字节** ≥ [`BatchPolicy::max_delay`]（默认 16 ms） | 交互延迟 |
//!
//! **时间那条不是可选优化。** 只按容量合批的实现在"输出停下等人"时是坏的：
//! shell 打出提示符 `> ` 之后不再有输出，那 2 个字节会一直躺在缓冲里**直到用户按键**——
//! 而用户正在等这个提示符。所以按时间的交付必须由**独立于读的等待**驱动，
//! [`spawn_batcher`] 就是干这个的（它等的是 `recv_timeout`，不是 `sleep`）。
//!
//! 三层职责，别混：
//!
//! * [`OutputBatcher`] —— **纯逻辑**：字节进、批次出，时钟由调用方注入 → 可用假时钟确定性单测；
//! * [`spawn_batcher`] —— **驱动**：读线程 + 合批线程，处理"阻塞的读"与"到点的交付"这对矛盾；
//! * 下游（IPC / 渲染）—— 在 plan 0202 之后，本 crate 不管。
//!
//! ⚠️ 批次里的字节**永远不是字符串**：合批只搬运 `Vec<u8>`，不解码、不假设 UTF-8 ——
//! 一个多字节字符完全可能被切成两批（`AGENTS.md` §3.2）。切分点由**解析层**容忍。

use std::io::{ErrorKind, Read};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

/// 读线程一次 `read` 最多要多少字节。
///
/// 取 64 KiB = [`BatchPolicy::max_bytes`]：读缓冲比批次上限大，只是白占内存；
/// 比它小，则**大输出会被拆成多次拷贝**（每次 `read` 都要先抄进读缓冲）。
/// 注意它**不是**批次大小的开关 —— 批次大小由 [`BatchPolicy`] 决定，这里只是每次搬多少。
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// 读线程最多领先合批线程多少块（背压）。
///
/// 有界是刻意的：无界队列会把"下游卡住"变成"内存涨上去"，而终端输出是最容易
/// 被 `yes` 一类程序灌爆的地方。8 × 64 KiB = 512 KiB 在途，够吸掉调度抖动。
const READ_BACKLOG: usize = 8;

/// 合批参数。**只有这一处**，不散落在读循环里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchPolicy {
    /// 攒够多少字节立刻交付。
    pub max_bytes: usize,
    /// 从本批**首字节**起算，最多等多久。
    pub max_delay: Duration,
}

impl BatchPolicy {
    /// 规范值：**≥16 ms 或 ≥64 KiB**（`AGENTS.md` §3.2）。
    ///
    /// 为什么是这两个数：
    ///
    /// * **16 ms** ≈ 60 Hz 的一帧。比它小，同一帧内的输出会被拆成多次交付（下游做无用功）；
    ///   比它大，人就看得见"输出一顿一顿地往外蹦"。
    /// * **64 KiB** 是一次交付的上限规模。它同时约束三件事：单批的内存、
    ///   跨 IPC 的消息大小、以及"下游被一大坨字节堵住"的时间。
    pub const DEFAULT: Self = Self {
        max_bytes: 64 * 1024,
        max_delay: Duration::from_millis(16),
    };
}

impl Default for BatchPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 这一批**为什么**被交出来。
///
/// 不是给下游分派用的（三种情形下游一视同仁，都是"往 xterm 写字节"），
/// 而是让"合批在按预期工作"这件事**可断言、可观测**：只测字节内容的话，
/// "容量触发"和"时间触发"在字节上完全一样，实现写反了测试照样绿。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// 攒够 [`BatchPolicy::max_bytes`] —— 大输出走这条。
    Capacity,
    /// 距本批首字节已满 [`BatchPolicy::max_delay`] —— 交互场景（提示符）走这条。
    Delay,
    /// 流结束（EOF / 读错误 / 收尾）时的**残批**。
    Flush,
}

/// 一个可交付的批次。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// 字节。**未解码**，也绝不保证切在字符边界上。
    pub bytes: Vec<u8>,
    /// 交付原因（见 [`Trigger`]）。
    pub trigger: Trigger,
}

/// 合批器：纯逻辑，**时钟由外部注入**。
///
/// 注入时钟（而不是内部调 `Instant::now()`）是这份实现唯一的可测性来源：
/// "16 ms 到了没有"这条边界，用真实时钟只能靠 `sleep` 去赌，那既慢又 flaky；
/// 注入之后它是一次函数调用，断言是精确的。
#[derive(Debug)]
pub struct OutputBatcher {
    policy: BatchPolicy,
    pending: Vec<u8>,
    /// 本批首字节到达的时刻。**`None` ⇔ `pending` 为空**（不变式，由 [`Self::take`] 维持）。
    first_at: Option<Instant>,
}

impl OutputBatcher {
    /// 用给定参数造一个合批器（`const`：可以在静态上下文里定义）。
    pub const fn new(policy: BatchPolicy) -> Self {
        Self {
            policy,
            pending: Vec::new(),
            first_at: None,
        }
    }

    /// 当前参数。
    pub const fn policy(&self) -> BatchPolicy {
        self.policy
    }

    /// 喂一块字节，并告知"现在几点"。
    ///
    /// 返回 `Some` = 这一块让批次满足了交付条件（容量或时间），**现在就该交出去**。
    ///
    /// 它同时检查时间条件，是为了防一类具体的错：驱动方忘了在没新字节时调 [`Self::poll_due`]，
    /// 于是"到点了但没新字节"永远等不到交付。喂字节这条路把两种触发都走通。
    ///
    /// 空块**不产生空批次**，也不推进时钟：批次的首字节时刻永远是**真有字节**的那一刻。
    pub fn push(&mut self, chunk: &[u8], now: Instant) -> Option<Batch> {
        if chunk.is_empty() {
            return None;
        }
        if self.first_at.is_none() {
            self.first_at = Some(now);
        }
        self.pending.extend_from_slice(chunk);
        self.take_due(now)
    }

    /// 没有新字节时也要调它：时间触发只有走到这里才会交付。
    pub fn poll_due(&mut self, now: Instant) -> Option<Batch> {
        self.take_due(now)
    }

    /// 收尾（EOF / 读错误 / 收尾路径）：把残批交出来 —— **不要丢尾巴**。
    ///
    /// 没有残批时返回 `None`（不是"空批次"）：空批次会让下游白白写一次零字节。
    pub fn flush(&mut self) -> Option<Batch> {
        self.take(Trigger::Flush)
    }

    /// 本批最迟该在何时交付；`None` = 当前没有待交付的字节（驱动方可以无限等）。
    ///
    /// 驱动方靠它把"阻塞等更多字节"与"到点交付"合成一个等待 —— 见 [`spawn_batcher`]。
    pub fn deadline(&self) -> Option<Instant> {
        // `checked_add` 失败只可能是 `max_delay` 大到溢出（现实中不会发生）。
        // 退回首字节时刻 = "已经到点了"，宁可就地交付，也不要一个永不触发的期限。
        self.first_at
            .map(|first| first.checked_add(self.policy.max_delay).unwrap_or(first))
    }

    /// 两条触发条件的**唯一**判定处 —— 交付语义只有这一份，不复制到 `push` / `poll_due` 里。
    fn take_due(&mut self, now: Instant) -> Option<Batch> {
        if self.pending.len() >= self.policy.max_bytes {
            return self.take(Trigger::Capacity);
        }
        if let Some(deadline) = self.deadline()
            && now >= deadline
        {
            return self.take(Trigger::Delay);
        }
        None
    }

    /// 取走整批并清空状态。**不切碎**：一次交出的就是攒下的全部字节 ——
    /// 按 `max_bytes` 去切只会多一次拷贝，而下游（xterm 缓冲）本来就要整批吃下去。
    fn take(&mut self, trigger: Trigger) -> Option<Batch> {
        if self.pending.is_empty() {
            return None;
        }
        self.first_at = None;
        Some(Batch {
            bytes: std::mem::take(&mut self.pending),
            trigger,
        })
    }
}

impl Default for OutputBatcher {
    fn default() -> Self {
        Self::new(BatchPolicy::DEFAULT)
    }
}

/// 把 `Read` 变成**批次流**：读线程负责阻塞地读，合批线程负责按时交付。
///
/// 返回值是一个普通的 `Receiver<Batch>`：迭代到 `Err`（发送端已关）就是流结束
/// （EOF 或读错误）。**结束前最后一批一定交出来** —— 提示符、报错行、"退出"消息
/// 通常正好落在最后一批里。
///
/// # 为什么是两个线程
///
/// 一个 `Box<dyn Read + Send>` 是**阻塞**的：没有"等字节或等超时"这种选择，
/// 也没有可移植的超时接口（`set_read_timeout` 是 `TcpStream` 的，PTY fd 没有）。
/// 想同时做到"字节一到就走"和"到 16 ms 没人来也得走"，就必须让**等待可超时**：
/// 读线程只管把字节丢进有界队列，合批线程用 `recv_timeout` 等它 —— 超时就是
/// "该交付时间触发了"，不是 `sleep` 那种盲等（`AGENTS.md` §0 第 5 条禁的是后者）。
///
/// # 读错误
///
/// 读错误与 EOF 一样**结束流**，不单独上报：PTY 上的读错误绝大多数就是
/// "子进程没了"（EIO），而结局该由 [`crate::Transport::exited`] /
/// [`crate::Transport::shutdown`] 给出 —— 把故障塞进字节队列会让下游分不清
/// "这坨字节"和"读取失败"。
///
/// # 两个线程什么时候收工
///
/// 源 EOF / 读错误、或下游 drop 之后**再来一个字节**（那一瞬间 `send` 失败即知）。
/// 有一种情形它们会停在等待上：**下游 drop 了而源完全安静** —— 此时没人给它们
/// 发信号。它们不占 CPU（阻塞在 channel 上），但会一直持有读端；现实中这条流
/// 的收尾由 [`crate::Transport::shutdown`] 完成（子进程一死，读端立刻返回并带出后续）。
/// 因此**别把 `Receiver` 的 drop 当作"结束这条流"的手段**，收尾要走 `shutdown`。
pub fn spawn_batcher(reader: Box<dyn Read + Send>, policy: BatchPolicy) -> Receiver<Batch> {
    let (chunk_tx, chunk_rx) = mpsc::sync_channel::<Vec<u8>>(READ_BACKLOG);
    let (batch_tx, batch_rx) = mpsc::channel::<Batch>();

    // 读线程：**不解析、不合批**，只把内核给的块搬到队列上。
    thread::spawn(move || read_loop(reader, &chunk_tx));
    // 合批线程：等字节或等期限，两者谁先到就交批。
    thread::spawn(move || batch_loop(chunk_rx, policy, &batch_tx));

    batch_rx
}

/// 读循环。返回即关掉队列 → 合批线程收到"发端已关" → 交付残批后结束。
fn read_loop(mut reader: Box<dyn Read + Send>, chunks: &SyncSender<Vec<u8>>) {
    let mut buf = vec![0u8; READ_CHUNK_BYTES];
    loop {
        match reader.read(&mut buf) {
            // EOF：子进程收工了。
            Ok(0) => return,
            Ok(n) => {
                // `send` 会阻塞 —— 这正是背压；接收方没了就收工。
                if chunks.send(buf[..n].to_vec()).is_err() {
                    return;
                }
            }
            // 被信号打断**不是**失败，重试（否则一个 EINTR 就能吃掉整条输出流）。
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

/// 合批循环：**先交到期的，再等下一块**，等的时候带上期限。
fn batch_loop(chunks: Receiver<Vec<u8>>, policy: BatchPolicy, batches: &Sender<Batch>) {
    let mut batcher = OutputBatcher::new(policy);
    loop {
        // 1. 到期就先交（也覆盖"上一次 push 攒下的容量触发"）。
        if let Some(batch) = batcher.poll_due(Instant::now()) {
            if batches.send(batch).is_err() {
                return; // 下游走了，读线程会因队列关闭自行收工
            }
            continue;
        }

        // 2. 等下一块。有期限就等到期限，没有（批次为空）就无限等 ——
        //    这里不会空转：上一行的 `poll_due` 没交付，说明 `deadline` 一定在未来。
        let chunk = match batcher.deadline() {
            Some(deadline) => {
                let wait = deadline.saturating_duration_since(Instant::now());
                match chunks.recv_timeout(wait) {
                    Ok(chunk) => chunk,
                    Err(RecvTimeoutError::Timeout) => continue, // 到点 → 回到 1 交付
                    Err(RecvTimeoutError::Disconnected) => break, // EOF / 读错误
                }
            }
            None => match chunks.recv() {
                Ok(chunk) => chunk,
                Err(_) => break,
            },
        };

        // 3. 喂进去；攒够了会在这一步立刻交出来（不等期限）。
        if let Some(batch) = batcher.push(&chunk, Instant::now())
            && batches.send(batch).is_err()
        {
            return;
        }
    }

    // 流结束：**残批不能丢**（`AGENTS.md` §3.2 的"不要丢尾巴"）。
    if let Some(batch) = batcher.flush() {
        let _ = batches.send(batch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeTransport;
    use crate::transport::{Capabilities, TerminalSize, Transport};

    /// 测试用的固定时钟原点。合批器只比较时刻，不关心它从哪来。
    fn t0() -> Instant {
        Instant::now()
    }

    fn policy() -> BatchPolicy {
        BatchPolicy::DEFAULT
    }

    // ── 纯逻辑：注入时钟，边界精确可断言 ────────────────────────────────────

    #[test]
    fn empty_input_produces_no_batch() {
        let mut batcher = OutputBatcher::new(policy());

        assert_eq!(batcher.push(b"", t0()), None, "空块不得产生空批次");
        assert_eq!(batcher.poll_due(t0()), None);
        assert_eq!(batcher.flush(), None, "没有残批时 flush 也不该造一个出来");
        assert_eq!(batcher.deadline(), None, "没有字节就没有期限");
    }

    #[test]
    fn capacity_boundary_is_exactly_max_bytes() {
        let mut batcher = OutputBatcher::new(policy());
        let now = t0();

        // 差一个字节：不交（否则等于把 64 KiB 的上限理解成了 64 KiB - 1）。
        assert_eq!(batcher.push(&vec![0x41; policy().max_bytes - 1], now), None);

        // 补上那一个字节：立刻交，且**整批**交（不按 max_bytes 切碎）。
        let batch = batcher
            .push(b"Z", now)
            .expect("补齐到 max_bytes 必须立刻交付");
        assert_eq!(batch.trigger, Trigger::Capacity);
        assert_eq!(batch.bytes.len(), policy().max_bytes);
        assert_eq!(batch.bytes[policy().max_bytes - 1], b'Z', "顺序必须保持");
    }

    #[test]
    fn oversized_chunk_is_delivered_whole() {
        let mut batcher = OutputBatcher::new(policy());
        let big = vec![0x5au8; policy().max_bytes * 3];

        let batch = batcher.push(&big, t0()).expect("超过上限的块应当立刻交付");

        assert_eq!(batch.trigger, Trigger::Capacity);
        assert_eq!(batch.bytes.len(), big.len(), "大块一次交完，不切碎");
        assert_eq!(batch.bytes, big);
    }

    #[test]
    fn time_boundary_is_exactly_max_delay() {
        let mut batcher = OutputBatcher::new(policy());
        let now = t0();
        let delay = policy().max_delay;

        assert_eq!(batcher.push(b"> ", now), None, "刚到的字节不该立刻交付");

        assert_eq!(
            batcher.poll_due(now + delay - Duration::from_millis(1)),
            None,
            "差 1 ms 不算到点"
        );

        let batch = batcher.poll_due(now + delay).expect("正好到点应当交付");
        assert_eq!(batch.trigger, Trigger::Delay);
        assert_eq!(batch.bytes, b"> ");
    }

    #[test]
    fn push_also_honours_the_time_trigger() {
        let mut batcher = OutputBatcher::new(policy());
        let now = t0();
        assert_eq!(batcher.push(b"a", now), None);

        // 驱动方在新字节到来时才回到合批器 —— 这条路也必须能交付，否则
        // "输出稀疏但持续"的场景会攒到容量上限才动（延迟完全失控）。
        let batch = batcher
            .push(b"b", now + policy().max_delay)
            .expect("到点后即使走的是 push 也要交付");
        assert_eq!(batch.trigger, Trigger::Delay);
        assert_eq!(batch.bytes, b"ab", "两块的顺序必须保持");
    }

    #[test]
    fn the_clock_restarts_at_the_first_byte_of_each_batch() {
        let mut batcher = OutputBatcher::new(policy());
        let now = t0();
        let delay = policy().max_delay;

        assert_eq!(batcher.push(b"one", now), None);
        assert_eq!(batcher.deadline(), Some(now + delay), "期限锚在首字节上");
        assert!(batcher.poll_due(now + delay).is_some());

        // 新一批：从**它自己**的首字节重新起算，而不是从上一批的。
        let start = now + delay;
        assert_eq!(batcher.push(b"two", start), None);
        assert_eq!(batcher.deadline(), Some(start + delay));
        assert_eq!(
            batcher.poll_due(now + delay + delay - Duration::from_millis(1)),
            None,
            "接上一批的期限算会在这里提前交付"
        );
        assert!(batcher.poll_due(start + delay).is_some());
    }

    #[test]
    fn flush_delivers_the_tail_and_then_reports_empty() {
        let mut batcher = OutputBatcher::new(policy());

        // 一个字节、远没到点：正常路径下它还在攒着。
        assert_eq!(batcher.push(b"bye\n", t0()), None);

        let tail = batcher.flush().expect("EOF 时残批必须交出来");
        assert_eq!(tail.trigger, Trigger::Flush);
        assert_eq!(tail.bytes, b"bye\n");

        assert_eq!(batcher.flush(), None, "残批只该交一次（幂等）");
        assert_eq!(batcher.deadline(), None);
    }

    #[test]
    fn no_bytes_are_lost_or_reordered() {
        // 最要紧的不变式：无论批次怎么切，**拼起来必须与输入逐字节相同**。
        // 用一个定死的线性同余序列当"随机"输入，失败时可复现。
        //
        // 参数取**小**值（61 字节 / 5 ms）而不是规范值：这条用例要走几千步、
        // 反复跨过两个边界，用 64 KiB 的话光拷贝就是几百 MB（单测跑成秒级是负担）。
        // 边界逻辑与常量大小无关 —— 上面几条已经钉住了 64 KiB / 16 ms 本身。
        let mut batcher = OutputBatcher::new(BatchPolicy {
            max_bytes: 61,
            max_delay: Duration::from_millis(5),
        });
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        let mut sent: Vec<u8> = Vec::new();
        let mut got: Vec<u8> = Vec::new();
        let mut now = t0();

        for step in 0..3000u32 {
            // 块大小跨过 max_bytes（含一次超大块），时间步长跨过 max_delay。
            let len = (next() % 160) as usize + 1;
            let chunk: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
            sent.extend_from_slice(&chunk);

            now += Duration::from_millis(next() % 9);
            if let Some(batch) = batcher.push(&chunk, now) {
                got.extend_from_slice(&batch.bytes);
            }
            if step % 7 == 0
                && let Some(batch) = batcher.poll_due(now)
            {
                got.extend_from_slice(&batch.bytes);
            }
        }
        if let Some(batch) = batcher.flush() {
            got.extend_from_slice(&batch.bytes);
        }

        assert_eq!(got.len(), sent.len(), "字节数必须一致");
        assert_eq!(got, sent, "字节内容与顺序必须一致");
    }

    // ── 驱动：真线程 + 真时钟，但只断言内容与"最终一定会交付" ────────────────
    //
    // 这几条**故意**不用紧的时序断言（那要靠调度运气，会 flaky）：
    // 计数与切分边界留给上面的假时钟用例，这里只问"这条路径通不通"。

    /// 造一个假载体并取走它的输出流（读端真的会阻塞，正好当驱动的输入）。
    fn stream(policy: BatchPolicy) -> (FakeTransport, Receiver<Batch>) {
        let mut transport = FakeTransport::new(Capabilities::PTY, TerminalSize::DEFAULT);
        let output = transport.output_stream().expect("取输出流失败");
        (transport, spawn_batcher(output, policy))
    }

    /// 大窗口：把时间触发推到很远，让"批量"只由容量决定 —— 断言才不依赖调度时序。
    fn wide_window() -> BatchPolicy {
        BatchPolicy {
            max_bytes: BatchPolicy::DEFAULT.max_bytes,
            max_delay: Duration::from_secs(60),
        }
    }

    #[test]
    fn stream_delivers_capacity_batches_without_waiting_for_the_deadline() {
        let (transport, batches) = stream(wide_window());

        transport.feed(vec![0x41u8; BatchPolicy::DEFAULT.max_bytes]);

        let batch = batches
            .recv_timeout(Duration::from_secs(5))
            .expect("攒够一个批次应当立刻交付，不必等到 60s 的期限");
        assert_eq!(batch.trigger, Trigger::Capacity);
        assert_eq!(batch.bytes.len(), BatchPolicy::DEFAULT.max_bytes);
    }

    #[test]
    fn stream_flushes_the_tail_when_the_source_ends() {
        // 窗口极宽 → 这批只可能是 EOF 时被 flush 出来的，不可能是容量或时间触发。
        let (mut transport, batches) = stream(wide_window());

        transport.feed(b"$ echo hi\r\nhi\r\n$ ");
        transport.shutdown().expect("shutdown 失败"); // 关掉写端 = 读端 EOF

        let batch = batches
            .recv_timeout(Duration::from_secs(5))
            .expect("EOF 后的残批必须交付");
        assert_eq!(batch.trigger, Trigger::Flush);
        assert_eq!(batch.bytes, b"$ echo hi\r\nhi\r\n$ ");

        assert!(
            batches.recv_timeout(Duration::from_secs(5)).is_err(),
            "流结束后不该再有批次"
        );
    }

    #[test]
    fn stream_delivers_on_the_deadline_without_further_input() {
        // 这条是本模块存在的理由：**输出停下等人时，提示符也必须自己冒出来**。
        // 只按容量合批的实现会在这里挂住直到下次按键。
        let (transport, batches) = stream(BatchPolicy {
            max_bytes: BatchPolicy::DEFAULT.max_bytes,
            max_delay: Duration::from_millis(50),
        });

        transport.feed(b"> "); // 然后就没声了 —— 用户正等着这个提示符

        let batch = batches
            .recv_timeout(Duration::from_secs(5))
            .expect("到点必须交付，不需要新字节也不需要关闭流");
        assert_eq!(batch.trigger, Trigger::Delay);
        assert_eq!(batch.bytes, b"> ");
    }

    #[test]
    fn a_multi_byte_character_may_be_split_across_batches() {
        // 合批**不解码**，所以一个多字节字符被切开是完全正常的（`AGENTS.md` §3.2）：
        // 拼字节的活儿在解析层。这条用例把这件事钉住 —— 谁要是哪天"顺手"在
        // 这里做了 UTF-8 校验或补齐，它会立刻红。
        let utf8 = "中".as_bytes(); // 3 字节
        let (mut transport, batches) = stream(BatchPolicy {
            max_bytes: 2,
            max_delay: Duration::from_secs(60),
        });

        transport.feed(&utf8[..2]);
        transport.feed(&utf8[2..]);
        transport.shutdown().expect("shutdown 失败");
        drop(transport);

        let mut got = Vec::new();
        while let Ok(batch) = batches.recv_timeout(Duration::from_secs(5)) {
            got.extend_from_slice(&batch.bytes);
        }
        assert_eq!(got, utf8, "切开的字符必须原样拼得回来，且中间不许有'补齐'");
    }
}
