//! plan 0704 的库内验收：**并发 in-flight 的上限，以及它在并发下不许破坏的东西**
//! （ADR-0006 D6）。
//!
//! 判据（ROADMAP 原文）=「大量小文件的吞吐**显著优于**串行请求」。逐条盯着它：
//!
//! | 判据 | 断言在哪 | 手段 |
//! |---|---|---|
//! | 上限真的在 | 7 个文件、上限 3：目标端点**同时**只见到 3 个 `begin_write`，第 4 个进不来 | 闸门（确定的事实，不抢时间） |
//! | 排队中被取消**不碰目标** | 上限 1、第一个停在闸门里 → 取消 → 第二个的路径从未被打开 | 同上 |
//! | 失败不扩散 | 5 个文件、中间那个写入失败 → 另外 4 个字节正确落地 | 真盘对账 |
//! | 吞吐显著更好 | 真服务端 + 带时延链路：上限 1 与 1/2/4/8/16 各量一次墙钟 | `slow_link`（口径见下） |
//! | 同名不撞临时名（D6） | 两条并发传输写同一个最终名 → 目标目录里是**两个**临时名 | 服务端真盘 |
//! | 临时名不再靠探测 | 上传一个文件：服务端记到的 `open` 恰好一次、`stat` 里没有临时名 | 服务端记的事实 |
//!
//! ## 为什么吞吐要用一条带时延的链路
//!
//! 本机回环的一次往返在微秒级，而并发 in-flight 的收益**全部**来自往返（`scope.md` §4.1）——
//! 不放大它，"串行 vs 并发"的差距会被系统噪声淹掉，用例只能变成一条随机红。
//! `slow_link` 把每个方向的每一段字节延后 10 ms，于是串行的墙钟时间由往返次数决定
//! （每个文件 3 次往返：`open` / `write` / `close`+`rename`），并发的由"几批"决定。
//! 断言取一个宽裕的比值（`并发 × 2 < 串行`），MB/s 一类的数只打印出来进基线，不作门禁
//! （`AGENTS.md` §7：性能基线不是门禁）。
//!
//! ⚠️ **服务端是串行处理请求的**（上游 `server::run` 一个循环里读一个、答一个），所以这里量到的
//! 是"客户端不等上一条往返回来就发下一条"这件事的收益 —— 那正是 in-flight 的定义。
//!
//! ## 两半证据都在
//!
//! "哪个文件落了盘"由**测试进程**直接读服务端那棵真目录答，"同时几个在搬"由 [`InFlight`] 的
//! 读数与假端点的计数答 —— 前者是结论，后者是过程。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "ssh_support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akasha_lib::ssh::testing::{ServerOptions, slow_link, start};
use akasha_lib::ssh::transfer::{
    BoxFuture, Cancel, Endpoint, FileRead, Listing, PendingWrite, Progress, TransferRequest,
};
use akasha_lib::ssh::{
    CredentialCache, InFlight, LocalEndpoint, PinnedHostKey, SshAuth, SshConnection, SshError,
    SshTarget,
};
use support::{CountingProvider, connect_options_to};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

/// 这条用例自己用的登录口令（**不是**用户的）。
const PASSWORD: &str = "pipelining-password";
const USER: &str = "cyrene";

/// 一条传输的请求（两端各自那一侧的路径）。
fn request<'a>(source: &'a str, target: &'a str) -> TransferRequest<'a> {
    TransferRequest {
        source_path: source,
        target_path: target,
    }
}

// ── 一次性的临时目录 ─────────────────────────────────────────────────────────

/// 一个自己的临时目录，drop 时删掉。
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "akasha-pipelining-{}-{tag}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn write(&self, name: &str, content: &[u8]) -> PathBuf {
        let path = self.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 一个目录里的条目名（排序）。
fn names_of(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 临时名（`.name.part` / `.name.2.part`）—— 判据里"目标目录里不该有的那些"。
fn temps(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|name| name.starts_with('.') && name.ends_with(".part"))
        .cloned()
        .collect()
}

// ── 假目标端点：把"同时有几个在搬"变成可以计数、可以闸住的事实 ────────────────

/// 把本机端点包一层：记下**同时停在 `begin_write` 里的个数**，并把每一个都闸住直到放行。
///
/// 为什么必须闸住：上限那一条判据的形态是"第 4 个进不来" —— 而"进不来"只有在
/// **前 3 个确定还停在里面**的时候才是一个事实。放它们自由跑，"同时 3 个"会变成一个
/// 靠机器速度的观察（`AGENTS.md` §7 禁止用固定等待猜异步，同一个道理）。
struct GatedTarget {
    inner: LocalEndpoint,
    /// 被调用过 `begin_write` 的目标路径，按发生顺序。
    opened: Mutex<Vec<String>>,
    /// 此刻还停在闸门里的个数。
    live: AtomicUsize,
    /// 上一条到过的最大值。
    peak: AtomicUsize,
    /// 放行开关：翻成 `true` 之后，**已经等在里面的**与**新来的**都直接通过。
    release: watch::Receiver<bool>,
}

impl GatedTarget {
    fn new(release: watch::Receiver<bool>) -> Self {
        Self {
            inner: LocalEndpoint::new(),
            opened: Mutex::new(Vec::new()),
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            release,
        }
    }

    fn opened(&self) -> Vec<String> {
        self.opened.lock().unwrap().clone()
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

impl Endpoint for GatedTarget {
    fn list<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Listing, SshError>> {
        self.inner.list(path)
    }

    fn open_read<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<FileRead, SshError>> {
        self.inner.open_read(path)
    }

    fn begin_write<'a>(
        &'a self,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn PendingWrite>, SshError>> {
        let path = path.to_owned();
        Box::pin(async move {
            let live = self.live.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(live, Ordering::SeqCst);
            self.opened.lock().unwrap().push(path.clone());
            // 先看当前值再看变化：`changed()` 只报"之后"的变化，放行发生在登记之前时会等空。
            let mut release = self.release.clone();
            if !*release.borrow() {
                let _ = release.changed().await;
            }
            let sink = self.inner.begin_write(&path).await;
            self.live.fetch_sub(1, Ordering::SeqCst);
            sink
        })
    }
}

/// 假源端点：把 `content` 交给读端之后**停住**（读端因此停在"临时文件建好了、还没提交"）。
struct GatedSource {
    content: Vec<u8>,
    gate: watch::Receiver<bool>,
}

impl Endpoint for GatedSource {
    fn list<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<Listing, SshError>> {
        panic!("假源端点只当源：被调到就是用例写错了")
    }

    fn open_read<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<FileRead, SshError>> {
        let content = self.content.clone();
        let size = content.len() as u64;
        let mut gate = self.gate.clone();
        Box::pin(async move {
            let (mut writer, reader) = tokio::io::duplex(64 * 1024);
            tokio::spawn(async move {
                writer.write_all(&content).await.unwrap();
                // 闸门关着 = 写端不放 EOF，读端因此读不到结尾（传输停在最后一块之后）。
                let _ = gate.changed().await;
            });
            Ok(FileRead {
                reader: Box::new(reader),
                size,
            })
        })
    }

    fn begin_write<'a>(
        &'a self,
        _path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn PendingWrite>, SshError>> {
        panic!("假源端点只当源：被调到就是用例写错了")
    }
}

/// 假目标端点：目标路径里带 `bad` 的那一条在第 `fail_at` 次写入时失败（"失败不扩散"的刺激）。
struct SometimesFailing {
    inner: LocalEndpoint,
    fail_at: usize,
}

impl Endpoint for SometimesFailing {
    fn list<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Listing, SshError>> {
        self.inner.list(path)
    }

    fn open_read<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<FileRead, SshError>> {
        self.inner.open_read(path)
    }

    fn begin_write<'a>(
        &'a self,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn PendingWrite>, SshError>> {
        let failing = path.contains("bad");
        let fail_at = self.fail_at;
        Box::pin(async move {
            let sink = self.inner.begin_write(path).await?;
            if !failing {
                return Ok(sink);
            }
            Ok(Box::new(FailingSink {
                inner: sink,
                remaining: fail_at,
            }) as Box<dyn PendingWrite>)
        })
    }
}

/// 数着写入次数、到点报错的写入端。
struct FailingSink {
    inner: Box<dyn PendingWrite>,
    remaining: usize,
}

impl PendingWrite for FailingSink {
    fn write_chunk<'a>(&'a mut self, chunk: &'a [u8]) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(async move {
            self.remaining -= 1;
            if self.remaining == 0 {
                return Err(SshError::File {
                    path: "（假目标）".to_owned(),
                    reason: "测试制造的写失败".to_owned(),
                });
            }
            self.inner.write_chunk(chunk).await
        })
    }

    fn commit<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(self.inner.commit())
    }

    fn abort<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(self.inner.abort())
    }
}

/// 等到条件成立；超时即失败并说明等的是什么（不用固定等待猜异步）。
async fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < deadline, "等不到：{what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// ── 用例 ────────────────────────────────────────────────────────────────────

/// 上限真的在：7 个文件、上限 3 —— 目标端点**同时**只见到 3 个 `begin_write`。
#[tokio::test]
async fn the_bound_holds_and_the_fourth_file_waits() {
    const FILES: usize = 7;
    const LIMIT: u32 = 3;

    let source = Scratch::new("bound-src");
    let target = Scratch::new("bound-dst");
    let (release_tx, release_rx) = watch::channel(false);
    let to = Arc::new(GatedTarget::new(release_rx));
    let in_flight = Arc::new(InFlight::new(LIMIT));

    let mut tasks = Vec::new();
    for index in 0..FILES {
        let content = format!("文件 {index} 的内容").into_bytes();
        let from = source.write(&format!("f{index}.txt"), &content);
        let into = target.join(&format!("f{index}.txt"));
        let from = from.to_string_lossy().into_owned();
        let into = into.to_string_lossy().into_owned();
        let to = Arc::clone(&to);
        let in_flight = Arc::clone(&in_flight);
        tasks.push(tokio::spawn(async move {
            let request = request(&from, &into);
            in_flight
                .transfer(
                    &LocalEndpoint::new(),
                    &*to,
                    &request,
                    &Progress::default(),
                    Cancel::new().waiter(),
                )
                .await
        }));
    }

    // 三个空位正好被三个文件占着 —— 这一刻第 4 个连 `begin_write` 都进不来。
    wait_for("三个文件同时停在目标端点里", || {
        to.peak() == LIMIT as usize
    })
    .await;
    assert_eq!(
        to.opened().len(),
        LIMIT as usize,
        "只会（且只会）有上限那么多个文件在目标端点里等着：{:?}",
        to.opened()
    );
    assert_eq!(in_flight.live(), LIMIT, "读数也要与事实一致");
    assert_eq!(in_flight.peak(), LIMIT);

    // 放行：剩下的文件依次拿到空位，绝不会同时进去更多。
    release_tx.send_replace(true);
    for task in tasks {
        task.await.unwrap().expect("每一个文件都该成功");
    }

    assert_eq!(
        to.peak(),
        LIMIT as usize,
        "上限是整个会话的：从头到尾没有第 4 个同时进来"
    );
    assert_eq!(in_flight.live(), 0, "搬完之后空位全部归还");
    assert_eq!(
        names_of(target.path()).len(),
        FILES,
        "7 个文件都该落地：{:?}",
        names_of(target.path())
    );
    assert!(temps(&names_of(target.path())).is_empty());
}

/// 排队中被取消：**一个端点都没碰过**（目标目录里从来没有这个文件）。
#[tokio::test]
async fn a_queued_transfer_cancelled_while_waiting_never_touches_the_target() {
    let source = Scratch::new("queue-src");
    let target = Scratch::new("queue-dst");
    let (release_tx, release_rx) = watch::channel(false);
    let to = Arc::new(GatedTarget::new(release_rx));
    // 上限 1：第二个文件只能在队里等着，而它在队里就被取消。
    let in_flight = Arc::new(InFlight::new(1));

    let mut tasks = Vec::new();
    for index in 0..2 {
        let content = vec![b'q'; 4096];
        let from = source.write(&format!("q{index}.txt"), &content);
        let into = target.join(&format!("q{index}.txt"));
        let from = from.to_string_lossy().into_owned();
        let to = Arc::clone(&to);
        let in_flight = Arc::clone(&in_flight);
        let cancel = Cancel::new();
        let waiter = cancel.waiter();
        tasks.push((
            into.clone(),
            cancel,
            tokio::spawn(async move {
                let into = into.to_string_lossy().into_owned();
                let request = request(&from, &into);
                in_flight
                    .transfer(
                        &LocalEndpoint::new(),
                        &*to,
                        &request,
                        &Progress::default(),
                        waiter,
                    )
                    .await
            }),
        ));
    }

    wait_for("第一个文件停在目标端点里", || {
        to.opened().len() == 1
    })
    .await;
    assert_eq!(in_flight.live(), 1, "第二个文件此刻在排队（还没占空位）");
    // 谁占了空位由端点记的事实说了算（不是由 spawn 的顺序猜）。
    let inside = PathBuf::from(to.opened().pop().unwrap());

    // 两个都取消：进去的那个还在闸门里面（放行之后才收尾），排队的那个在队里就被叫停。
    for (_, cancel, _) in &tasks {
        cancel.cancel();
    }
    let queued_at = tasks
        .iter()
        .position(|(path, _, _)| *path != inside)
        .expect("两条传输里必有一条在排队");
    let (queued_path, _, queued_task) = tasks.remove(queued_at);
    let failure = queued_task
        .await
        .unwrap()
        .expect_err("排队中被取消的那条不该成功");
    assert!(
        matches!(failure, SshError::Cancelled),
        "排队中被取消应当是「被中止」那一档：{failure:?}"
    );
    assert_eq!(
        to.opened(),
        vec![inside.to_string_lossy().into_owned()],
        "排队中的那条**一个端点都没碰过**（{} 从未被打开）",
        queued_path.display()
    );

    // 放行进去的那个：它在闸门之后才看到取消，于是走收尾那一半（临时名删掉）。
    release_tx.send_replace(true);
    let (_, _, first) = tasks.pop().unwrap();
    let failure = first.await.unwrap().expect_err("被取消的那条不该成功");
    assert!(matches!(failure, SshError::Cancelled), "{failure:?}");
    assert!(
        names_of(target.path()).is_empty(),
        "取消之后目标目录必须是空的（既没有最终名，也没有临时名）：{:?}",
        names_of(target.path())
    );
}

/// 失败不扩散：一个文件失败，别的照样落地，而失败那个不留临时名。
#[tokio::test]
async fn one_failed_file_does_not_stop_the_others() {
    let source = Scratch::new("isolate-src");
    let target = Scratch::new("isolate-dst");
    let to = Arc::new(SometimesFailing {
        inner: LocalEndpoint::new(),
        // 第 2 次写入失败：第 1 块已经落进临时文件，于是"删掉临时文件"有东西可删。
        fail_at: 2,
    });
    let in_flight = Arc::new(InFlight::new(2));

    let mut tasks = Vec::new();
    for index in 0..5 {
        // 300 KiB 分几块，好让"第 2 次写入失败"有确定的位置。
        let content: Vec<u8> = (0u8..=255).cycle().take(300 * 1024).collect();
        let name = if index == 2 {
            "bad.txt".to_owned()
        } else {
            format!("ok{index}.txt")
        };
        let from = source.write(&name, &content);
        let into = target.join(&name);
        let from = from.to_string_lossy().into_owned();
        let in_flight = Arc::clone(&in_flight);
        let to = Arc::clone(&to);
        tasks.push((
            into.clone(),
            tokio::spawn(async move {
                let into = into.to_string_lossy().into_owned();
                let request = request(&from, &into);
                in_flight
                    .transfer(
                        &LocalEndpoint::new(),
                        &*to,
                        &request,
                        &Progress::default(),
                        Cancel::new().waiter(),
                    )
                    .await
            }),
        ));
    }

    let mut failed = 0;
    for (into, task) in tasks {
        match task.await.unwrap() {
            Ok(_) => assert_ne!(
                into.file_name().unwrap().to_string_lossy(),
                "bad.txt",
                "制造失败的那个文件不该成功"
            ),
            Err(err) => {
                failed += 1;
                assert!(matches!(err, SshError::File { .. }), "{err:?}");
            }
        }
    }
    assert_eq!(failed, 1, "只有一个文件该失败");

    let names = names_of(target.path());
    assert_eq!(
        names,
        vec!["ok0.txt", "ok1.txt", "ok3.txt", "ok4.txt"],
        "另外 4 个必须都落地（失败不扩散）"
    );
    assert!(
        std::fs::read(target.join("ok0.txt")).unwrap().len() == 300 * 1024,
        "落地的是完整内容"
    );
}

/// 并发下 D6 那句话：**两个同名文件不能撞在同一个临时名上**（真服务端 + 真盘）。
#[tokio::test]
async fn two_transfers_of_the_same_name_do_not_share_a_temp_name() {
    let server = start(ServerOptions {
        sftp: Some(Vec::new()),
        ..ServerOptions::password(PASSWORD)
    })
    .await;
    let root = server
        .sftp_root()
        .expect("这台服务端应当提供 SFTP")
        .to_path_buf();
    let client = connect_sftp(&server).await;

    let a = vec![b'a'; 1000];
    let b = vec![b'b'; 2000];
    let (gate_tx, gate_rx) = watch::channel(false);
    let from_a = Arc::new(GatedSource {
        content: a.clone(),
        gate: gate_rx.clone(),
    });
    let from_b = Arc::new(GatedSource {
        content: b.clone(),
        gate: gate_rx,
    });
    // 上限 2：两条都在空位上（这条用例要的是"同时"，不是"排队"）。
    let in_flight = Arc::new(InFlight::new(2));

    let mut tasks = Vec::new();
    for source in [Arc::clone(&from_a), Arc::clone(&from_b)] {
        let source: Arc<dyn Endpoint> = source;
        let client = client.clone();
        let in_flight = Arc::clone(&in_flight);
        tasks.push(tokio::spawn(async move {
            in_flight
                .transfer(
                    &*source,
                    &client,
                    &request("/local/a.bin", "/same.bin"),
                    &Progress::default(),
                    Cancel::new().waiter(),
                )
                .await
        }));
    }

    // 两条都在目标端点里了：真盘上应当是**两个**临时名（老实现下它们是同一个）。
    wait_for("两条传输都抢到了临时名", || {
        temps(&names_of(&root)).len() == 2
    })
    .await;

    // 判据就断言在这里。
    let during = names_of(&root);
    let temp_names = temps(&during);
    assert_eq!(
        temp_names.len(),
        2,
        "两个同名文件的临时名必须互不相同：{during:?}"
    );
    assert_ne!(temp_names[0], temp_names[1]);
    assert!(
        !during.contains(&"same.bin".to_owned()),
        "还没提交就不该有最终名：{during:?}"
    );
    assert!(
        server.shared.observed().sftp_opens.len() >= 2,
        "两次抢占各是一次 `open`：{:?}",
        server.shared.observed().sftp_opens
    );

    gate_tx.send_replace(true);
    for task in tasks {
        task.await.unwrap().expect("两条都该成功");
    }

    // 最终名只有一个，而它的内容是**其中一条**源的完整内容（交错写的表现是既不是 A 也不是 B）。
    assert_eq!(names_of(&root), vec!["same.bin".to_owned()]);
    let landed = std::fs::read(root.join("same.bin")).unwrap();
    assert!(
        landed == a || landed == b,
        "落盘的必须是两条源之一（长度 {}，A {} / B {}）",
        landed.len(),
        a.len(),
        b.len()
    );
}

/// 临时名不再靠探测：一次 `open` 抢到，**没有**那一次多余的 `stat`。
#[tokio::test]
async fn a_temp_name_is_claimed_with_one_request_and_no_probe() {
    let server = start(ServerOptions {
        sftp: Some(Vec::new()),
        ..ServerOptions::password(PASSWORD)
    })
    .await;
    let client = connect_sftp(&server).await;

    let scratch = Scratch::new("probe");
    let payload = vec![b'p'; 4096];
    let from = scratch.write("probe.bin", &payload);

    let copied = in_flight_transfer(
        &InFlight::new(1),
        &LocalEndpoint::new(),
        &client,
        &from.to_string_lossy(),
        "/probe.bin",
    )
    .await
    .expect("上传应当成功");
    assert_eq!(copied, payload.len() as u64);

    let observed = server.shared.observed();
    assert_eq!(
        observed.sftp_opens,
        vec!["/.probe.bin.part".to_owned()],
        "临时名是**一次** `open` 抢到的（`CREATE|EXCLUDE`）"
    );
    assert!(
        observed.sftp_stats.is_empty(),
        "临时名的占用不再靠 `stat` 探测：{:?}",
        observed.sftp_stats
    );
}

/// 吞吐：真服务端 + 带时延链路，上限 1 与 1/2/4/8/16 各量一次。
///
/// 这一条同时是**默认值的来源**：分档数字打印在下面（也记进 plan 0704 的实施记录），
/// 而断言只有一条宽裕的比值 —— 性能数字本身不是门禁（`AGENTS.md` §7）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_small_files_are_faster_when_they_are_in_flight() {
    const FILES: usize = 12;
    const LIMITS: [u32; 5] = [1, 2, 4, 8, 16];

    let server = start(ServerOptions {
        sftp: Some(Vec::new()),
        ..ServerOptions::password(PASSWORD)
    })
    .await;
    let root = server
        .sftp_root()
        .expect("这台服务端应当提供 SFTP")
        .to_path_buf();
    // 每个方向的每一段延后 10 ms：一次往返 ≈ 20 ms，而一个文件 3 次往返。
    let link = slow_link(server.addr, Duration::from_millis(10)).await;
    let client = connect_sftp_via(&server, link.addr.port()).await;

    let scratch = Scratch::new("throughput");
    let mut sources = Vec::new();
    for index in 0..FILES {
        let content: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
        sources.push(scratch.write(&format!("small{index}.bin"), &content));
    }

    let mut measured = Vec::new();
    for limit in LIMITS {
        let in_flight = Arc::new(InFlight::new(limit));
        let started = Instant::now();
        let mut tasks = Vec::new();
        for (index, from) in sources.iter().enumerate() {
            let client = client.clone();
            let in_flight = Arc::clone(&in_flight);
            let from = from.to_string_lossy().into_owned();
            let to = format!("/limit{limit}-{index}.bin");
            tasks.push(tokio::spawn(async move {
                let request = request(&from, &to);
                in_flight
                    .transfer(
                        &LocalEndpoint::new(),
                        &client,
                        &request,
                        &Progress::default(),
                        Cancel::new().waiter(),
                    )
                    .await
            }));
        }
        for task in tasks {
            task.await.unwrap().expect("每个文件都该成功");
        }
        let elapsed = started.elapsed();

        let peak = in_flight.peak();
        let per_file = elapsed.as_secs_f64() * 1000.0 / FILES as f64;
        eprintln!(
            "上限 {limit:2}：{FILES} 个 1 KiB 文件 {elapsed:.3?}（每个 {per_file:.1} ms，peak {peak}，链路搬了 {} 段）",
            link.chunks()
        );
        assert!(
            peak <= limit,
            "上限 {limit} 之下 peak 不该超过它（实测 {peak}）"
        );
        measured.push(elapsed);
    }

    let serial = measured[0];
    let fastest = *measured.last().unwrap();
    eprintln!(
        "串行 {serial:.3?} vs 上限 {} 的 {fastest:.3?} —— 快 {:.1} 倍",
        LIMITS[LIMITS.len() - 1],
        serial.as_secs_f64() / fastest.as_secs_f64()
    );
    assert!(
        fastest * 2 < serial,
        "并发的墙钟时间应当显著短于串行：串行 {serial:.3?}、最快 {fastest:.3?}"
    );

    // 全部落盘，而且**一个临时名都不剩**（并发不会把落盘不变量弄丢）。
    let names = names_of(&root);
    assert_eq!(
        names.len(),
        FILES * LIMITS.len(),
        "每一次都该留下 {FILES} 个文件"
    );
    assert!(
        temps(&names).is_empty(),
        "不该留下临时名：{:?}",
        temps(&names)
    );
    for index in 0..FILES {
        for limit in LIMITS {
            assert_eq!(
                std::fs::read(root.join(format!("limit{limit}-{index}.bin")))
                    .unwrap()
                    .len(),
                1024,
                "每个文件的字节数都该与源相同"
            );
        }
    }
}

// ── 连服务端的两个辅助 ───────────────────────────────────────────────────────

/// 一条传输（走有界入口），省得每条用例都写一遍这五样参数。
async fn in_flight_transfer(
    in_flight: &InFlight,
    source: &dyn Endpoint,
    target: &dyn Endpoint,
    from: &str,
    to: &str,
) -> Result<u64, SshError> {
    in_flight
        .transfer(
            source,
            target,
            &request(from, to),
            &Progress::default(),
            Cancel::new().waiter(),
        )
        .await
}

/// 连上测试服务端并开一个 SFTP 会话。
async fn connect_sftp(server: &akasha_lib::ssh::testing::Running) -> akasha_lib::ssh::SftpClient {
    connect_sftp_via(server, server.addr.port()).await
}

/// 同上，但客户端连的是**另一个端口**（带时延链路的入口）。
async fn connect_sftp_via(
    server: &akasha_lib::ssh::testing::Running,
    port: u16,
) -> akasha_lib::ssh::SftpClient {
    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new(PASSWORD));
    let mut connect = connect_options_to(
        SshTarget::new("127.0.0.1", port, USER),
        Arc::new(PinnedHostKey::new(server.fingerprint.clone())),
        SshAuth::keys(Vec::new()),
        cache,
        provider,
        support::CONNECT_TIMEOUT,
    );
    let connection = SshConnection::connect(&mut connect)
        .await
        .expect("连接测试服务端失败");
    connection.sftp().await.expect("SFTP 会话应当建得起来")
}
