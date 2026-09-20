//! plan 0702 的库内验收：**传输引擎的落盘不变量**（`scope.md` §4.2，ADR-0006 D4）。
//!
//! 判据（ROADMAP 原文）=「中断传输后目标目录里**没有**看似完整的文件」。逐条盯着它：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 传输中写的是**临时名** | 成功那条：传输进行中临时文件在、最终名**不在** |
//! | 成功 = **原子重命名**落地 | 最终名的字节与源逐字节相同，且临时名消失 |
//! | **取消**不留半成品 | 假源停住 → 取消 → 目标目录里既没有最终名也没有临时名 |
//! | **失败**不留半成品 | 假目标在第 2 次写入时失败 → 同上 |
//! | 临时名不撞车（D6） | 目标目录里已有一个 `.name.part` 时，它**不被覆盖**，最终名照样正确 |
//! | 真盘两向都通 | 与本机端点之间上传 / 下载，拿**服务端那棵真目录**对账 |
//!
//! ## 为什么"中断"是可等待的事件而不是一段等待
//!
//! 直觉做法是"传一个大文件，然后赶紧点取消"——那是跟机器速度赛跑，快机器上传输早就完了，
//! 用例红得毫无道理（`AGENTS.md` §7 禁止用固定等待猜异步）。这里的做法是把"传到哪一步"
//! 变成**两个可以等待的信号**：
//!
//! * 假目标端点在**临时文件建好之后**报一次 `created`；
//! * 假源端点在交出第一块之后**停住**（直到测试放行），而假目标端点在**第一次写入落地之后**
//!   报一次 `wrote`。
//!
//! ⚠️ 判据写成"临时文件里是**一段源文件的前缀**"，不写"正好一块"：`write_chunk` 返回说的是
//! "端点收下了这一块"，**不是**"它已经落在文件里"（本机端点建在 `tokio::fs` 上，它在
//! **派发**阻塞写之后立刻返回 `Ready`，真正的 `write(2)` 要等下一次 poll —— 所以那一刻文件
//! 可能是空的）。全部落地由 `commit` 保证，而那正是这条判据真正要说的事：
//! **未完成的文件永远只是一个不完整的前缀**。这一条实测红过两次（394 与 412 行），
//! 根因写在 `PendingWrite::write_chunk` 的文档里。
//!
//! 于是"此刻传输确实在跑、且还剩很多没搬"是等出来的事实，不是猜出来的。
//!
//! ⚠️ **`wrote` 这个信号是必需的，不能拿"源交出了第一块"代替**：源把字节写进管道与引擎
//! 把这一块落到临时文件之间隔着一次调度 —— 实测过的那一版就是拿"源交出去了"当判据，
//! 于是"此刻临时文件里正好是一块"这句话在 `CHUNK_BYTES` 与两倍之间随机取一个值
//! （第二次 `just ready` 时红的就是它）。判据要落在**引擎自己做过的事**上。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

#[path = "ssh_support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_lib::ssh::testing::{ServerOptions, SftpItem, start};
use akasha_lib::ssh::transfer::{
    BoxFuture, Cancel, Endpoint, EntryKind, FileRead, Listing, PendingWrite, Progress,
    TransferRequest, transfer,
};
use akasha_lib::ssh::{CredentialCache, LocalEndpoint, SshAuth, SshConnection, SshError};
use support::{CountingProvider, connect_options};
use tokio::io::AsyncWriteExt;

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
            "akasha-transfer-{}-{tag}-{}",
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

    /// **"目标目录里到底有什么"的唯一读数口** —— 判据的每一句都断言在它上面。
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn write(&self, name: &str, content: &[u8]) -> PathBuf {
        let path = self.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.join(name)).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ── 两个假端点：一个控制"传到哪一步"，一个制造"写失败" ──────────────────────

/// 假源端点：交出第一块之后**停住**，直到测试放行。
struct GatedSource {
    first: Vec<u8>,
    rest: Vec<u8>,
    /// "放行"。关着 = 传输永远停在第一块之后。
    gate: tokio::sync::watch::Receiver<bool>,
}

impl Endpoint for GatedSource {
    fn list<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<Listing, SshError>> {
        // 这条用例里它只当源：被调到就是用例写错了，直接失败比返回一个空列表好。
        panic!("假源端点不提供列目录")
    }

    fn open_read<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<FileRead, SshError>> {
        let first = self.first.clone();
        let rest = self.rest.clone();
        let mut gate = self.gate.clone();
        let size = (first.len() + rest.len()) as u64;
        Box::pin(async move {
            // 一条内存管道：写端由一条任务推，读端交给引擎当普通 `AsyncRead`。
            let (mut writer, reader) = tokio::io::duplex(64 * 1024);
            tokio::spawn(async move {
                writer.write_all(&first).await.unwrap();
                // `changed()` 在发送端被丢掉时返回 `Err` —— 那时也放行（用例结束了）。
                let _ = gate.changed().await;
                writer.write_all(&rest).await.unwrap();
                // 写端 drop = 读端看到 EOF，传输正常结束。
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
        panic!("假源端点不提供写入")
    }
}

/// 假目标端点：把本机端点包一层，在**临时文件建好之后**与**第一次写入落地之后**各报一次信号。
///
/// 单为这一条而存在：证明"传输进行中写的是临时名"需要一个**确定**的时刻 ——
/// 即"临时文件已经建好、里面正好是引擎搬过的那一块、而最终名还不该存在"。
struct SignalingTarget {
    inner: LocalEndpoint,
    /// 临时文件已经建好（此刻它是空的）。
    created: tokio::sync::mpsc::UnboundedSender<()>,
    /// 引擎的第一块已经落在临时文件里（此刻它的长度就是一块）。
    wrote: tokio::sync::mpsc::UnboundedSender<()>,
}

impl Endpoint for SignalingTarget {
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
        let created = self.created.clone();
        let wrote = self.wrote.clone();
        Box::pin(async move {
            let sink = self.inner.begin_write(path).await?;
            // 到这里临时文件一定已经建出来了（`LocalEndpoint::begin_write` 返回前就
            // `create` 过它），而最终名要等 `commit` —— 于是这一刻正是判据要的时刻。
            let _ = created.send(());
            Ok(Box::new(SignalingSink { inner: sink, wrote }) as Box<dyn PendingWrite>)
        })
    }
}

/// 把引擎的写入转给真正的目标，并在**第一块落地之后**报一次。
struct SignalingSink {
    inner: Box<dyn PendingWrite>,
    wrote: tokio::sync::mpsc::UnboundedSender<()>,
}

impl PendingWrite for SignalingSink {
    fn write_chunk<'a>(&'a mut self, chunk: &'a [u8]) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(async move {
            self.inner.write_chunk(chunk).await?;
            // ⚠️ 报在**端点收下之后**（不是"源交出去了"）：见文件头关于 `write_chunk` 语义的说明。
            let _ = self.wrote.send(());
            Ok(())
        })
    }

    fn commit<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(self.inner.commit())
    }

    fn abort<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(self.inner.abort())
    }
}

/// 假目标端点：第 `fail_at` 次写入时报错（"写失败"那条退出路径的刺激）。
struct FailingTarget {
    inner: LocalEndpoint,
    fail_at: usize,
}

impl Endpoint for FailingTarget {
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
        let fail_at = self.fail_at;
        Box::pin(async move {
            let sink = self.inner.begin_write(path).await?;
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

// ── 用例 ────────────────────────────────────────────────────────────────────

/// 成功那一半：传输中目标是**临时名**，成功之后是**最终名**，字节逐一对上。
#[tokio::test]
async fn a_transfer_writes_a_temp_name_and_lands_by_rename() {
    let source = Scratch::new("src");
    let target = Scratch::new("dst");
    let content: Vec<u8> = (0u8..=255).cycle().take(300 * 1024).collect();
    source.write("payload.bin", &content);

    let (created_tx, mut created_rx) = tokio::sync::mpsc::unbounded_channel();
    let (wrote_tx, mut wrote_rx) = tokio::sync::mpsc::unbounded_channel();
    let (gate_tx, gate_rx) = tokio::sync::watch::channel(false);

    // 头一块给两倍于 `CHUNK_BYTES` 的量：源那边一次就能交出来，
    // 于是"引擎读了一块就停住"这件事与"源还没交完"分得开。
    let from = GatedSource {
        first: content[..2 * akasha_lib::ssh::transfer::CHUNK_BYTES].to_vec(),
        rest: content[2 * akasha_lib::ssh::transfer::CHUNK_BYTES..].to_vec(),
        gate: gate_rx,
    };
    let to = SignalingTarget {
        inner: LocalEndpoint::new(),
        created: created_tx,
        wrote: wrote_tx,
    };

    let progress = Arc::new(Progress::default());
    let task = tokio::spawn({
        let progress = Arc::clone(&progress);
        let source_path = source.join("payload.bin").to_string_lossy().into_owned();
        let target_path = target.join("payload.bin").to_string_lossy().into_owned();
        async move {
            transfer(
                &from,
                &to,
                &request(&source_path, &target_path),
                &progress,
                Cancel::new().waiter(),
            )
            .await
        }
    });

    // ① 临时文件已经建好 —— **此刻最终名不该存在**（这就是"先写临时名"）。
    created_rx
        .recv()
        .await
        .expect("假目标端点应当报过临时文件建好");
    assert_eq!(
        target.names(),
        vec![".payload.bin.part".to_owned()],
        "传输中目标目录里只该有临时名"
    );

    // ② 引擎的第一块已经落在临时文件里 —— 传输确实在跑，且还剩很多没搬。
    wrote_rx.recv().await.expect("假目标端点应当报过第一块写入");
    assert_eq!(
        target.names(),
        vec![".payload.bin.part".to_owned()],
        "搬了一半也不该出现最终名"
    );
    let partial = std::fs::read(target.join(".payload.bin.part")).unwrap();
    assert!(
        partial.len() < content.len() && content.starts_with(&partial),
        "临时文件里只可能是源文件的一段**前缀**（已经搬过 {} 字节，总长 {}）—— \
         未完成的文件永远不该是别的东西，也永远不该有完整长度",
        partial.len(),
        content.len()
    );

    // ③ 放行，让它跑完。
    gate_tx.send(true).unwrap();
    let copied = task.await.unwrap().expect("放行之后传输应当成功");
    assert_eq!(copied, content.len() as u64);

    // ④ 成功 = 最终名在、临时名没了、字节一模一样。
    assert_eq!(target.names(), vec!["payload.bin".to_owned()]);
    assert_eq!(
        target.read("payload.bin"),
        content,
        "落盘的字节必须与源相同"
    );
    assert_eq!(progress.done(), content.len() as u64);
    assert_eq!(progress.total(), content.len() as u64);
}

/// 取消那一半：目标目录里既没有最终名，也没有剩下的临时名。
#[tokio::test]
async fn a_cancelled_transfer_leaves_nothing_behind() {
    let source = Scratch::new("cancel-src");
    let target = Scratch::new("cancel-dst");
    let content = vec![b'x'; 512 * 1024];
    source.write("payload.bin", &content);

    let (created_tx, mut created_rx) = tokio::sync::mpsc::unbounded_channel();
    let (wrote_tx, mut wrote_rx) = tokio::sync::mpsc::unbounded_channel();
    // ⚠️ 这个闸门**永远不放行** —— 传输因此确定地停在第一块之后，不会自己跑完。
    let (_gate_tx, gate_rx) = tokio::sync::watch::channel(false);

    let from = GatedSource {
        first: content[..2 * akasha_lib::ssh::transfer::CHUNK_BYTES].to_vec(),
        rest: content[2 * akasha_lib::ssh::transfer::CHUNK_BYTES..].to_vec(),
        gate: gate_rx,
    };
    let to = SignalingTarget {
        inner: LocalEndpoint::new(),
        created: created_tx,
        wrote: wrote_tx,
    };

    let cancel = Cancel::new();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        let source_path = source.join("payload.bin").to_string_lossy().into_owned();
        let target_path = target.join("payload.bin").to_string_lossy().into_owned();
        async move {
            transfer(
                &from,
                &to,
                &request(&source_path, &target_path),
                &Progress::default(),
                cancel.waiter(),
            )
            .await
        }
    });

    created_rx.recv().await.expect("临时文件应当已经建好");
    wrote_rx.recv().await.expect("引擎应当已经写下第一块");
    // 取消之前临时名确实在（否则这条用例什么都没验到）；内容只可能是源的一段前缀。
    let partial = std::fs::read(target.join(".payload.bin.part")).unwrap();
    assert!(
        partial.len() < content.len() && content.starts_with(&partial),
        "取消之前临时文件里只可能是源文件的一段前缀（{} / {} 字节）",
        partial.len(),
        content.len()
    );

    cancel.cancel();
    let failure = match task.await.unwrap() {
        Ok(copied) => panic!("被取消的传输不该报告成功（搬了 {copied} 字节）"),
        Err(err) => err,
    };
    assert!(
        matches!(failure, SshError::Cancelled),
        "失败应当是「被中止」那一档：{failure:?}"
    );

    assert!(
        target.names().is_empty(),
        "取消之后目标目录必须是空的（既没有最终名，也没有临时名）：{:?}",
        target.names()
    );
}

/// 失败那一半：写到第 2 块时报错，临时名照样被删掉。
#[tokio::test]
async fn a_failed_write_leaves_nothing_behind() {
    let source = Scratch::new("fail-src");
    let target = Scratch::new("fail-dst");
    let content = vec![b'y'; 300 * 1024];
    source.write("payload.bin", &content);

    let to = FailingTarget {
        inner: LocalEndpoint::new(),
        // 第 2 次写入失败：第 1 块已经落进临时文件，于是"删掉临时文件"这件事有东西可删。
        fail_at: 2,
    };
    let failure = transfer(
        &LocalEndpoint::new(),
        &to,
        &request(
            &source.join("payload.bin").to_string_lossy(),
            &target.join("payload.bin").to_string_lossy(),
        ),
        &Progress::default(),
        Cancel::new().waiter(),
    )
    .await
    .expect_err("假目标端点应当在第 2 次写入时失败");

    assert!(
        matches!(failure, SshError::File { .. }),
        "失败应当是文件操作那一档：{failure:?}"
    );
    assert!(
        target.names().is_empty(),
        "写失败之后目标目录必须是空的：{:?}",
        target.names()
    );
}

/// ADR-0006 D6：临时名不能撞车 —— 已经有一个 `.name.part` 时**不许覆盖它**。
#[tokio::test]
async fn an_existing_temp_name_is_not_overwritten() {
    let source = Scratch::new("collide-src");
    let target = Scratch::new("collide-dst");
    source.write("b.txt", b"new");
    // 另一个传输（或上一次被强杀的进程）留下的同名临时文件。
    target.write(".b.txt.part", b"old");

    transfer(
        &LocalEndpoint::new(),
        &LocalEndpoint::new(),
        &request(
            &source.join("b.txt").to_string_lossy(),
            &target.join("b.txt").to_string_lossy(),
        ),
        &Progress::default(),
        Cancel::new().waiter(),
    )
    .await
    .expect("传输本身应当成功");

    assert_eq!(target.read("b.txt"), b"new");
    assert_eq!(
        target.read(".b.txt.part"),
        b"old",
        "已经存在的临时名被覆盖了 —— 那正是 D6 说不能发生的事"
    );
    assert_eq!(
        target.names(),
        vec![".b.txt.part".to_owned(), "b.txt".to_owned()],
        "第二次用的应当是另一个临时名，而且它必须已经被重命名掉"
    );
}

/// 端点返回的**规范化**路径（这条判据的对照）。
///
/// ⚠️ 不能直接拿 `std::fs::canonicalize` 比（问题 #168）：Windows 上它给的带 verbatim 前缀，
/// 而端点在返回之前**有意**把那个前缀去掉（`ssh/local.rs` 的 `tidy` —— 那是原生路径的转义
/// 写法，不是用户认得的路径：界面上那一栏的当前目录与“返回上一级”拼出来的字符串都不该带它）。
/// 所以这里对照的是同一条规则：**先规范化、再去前缀**。
fn canonical(path: &Path) -> String {
    let text = std::fs::canonicalize(path)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    #[cfg(windows)]
    {
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return rest.to_owned();
        }
    }
    text
}
/// 本机端点自己不经过引擎的那一部分：列目录、认类型。
#[tokio::test]
async fn the_local_endpoint_lists_a_directory() {
    let dir = Scratch::new("list");
    dir.write("a.txt", b"a");
    std::fs::create_dir(dir.join("sub")).unwrap();

    let listing = LocalEndpoint::new()
        .list(dir.path().to_string_lossy().as_ref())
        .await
        .expect("列目录应当成功");

    assert_eq!(
        listing.path,
        canonical(dir.path()),
        "返回的应当是**规范化之后**的路径（调用方拿它当当前目录）"
    );
    let names: Vec<&str> = listing
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, vec!["a.txt", "sub"], "条目应当按名字排序");
    let kind = |name: &str| {
        listing
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.kind)
    };
    assert_eq!(kind("a.txt"), Some(EntryKind::File));
    assert_eq!(kind("sub"), Some(EntryKind::Directory));
}

/// 与本机端点之间的**两向**都通，而且拿服务端那棵**真目录**对账。
///
/// 这一条覆盖的是 `SftpClient` 的读写路径（引擎的另一半）：下载用 `open_read`，
/// 上传用 `begin_write`（临时名 + `rename` 全在远端那一侧）。
#[tokio::test]
async fn files_go_both_ways_against_a_real_remote_directory() {
    const PASSWORD: &str = "transfer-round-trip";
    const USER: &str = "cyrene";
    let payload: Vec<u8> = (0u8..=255).cycle().take(200 * 1024).collect();

    let mut options = ServerOptions::password(PASSWORD);
    options.sftp = Some(vec![
        SftpItem::file_with("remote.bin", payload.clone()),
        SftpItem::dir("sub"),
    ]);
    let server = start(options).await;
    let root = server
        .sftp_root()
        .expect("这台服务端应当提供 SFTP")
        .to_path_buf();

    let cache = Arc::new(CredentialCache::new());
    let provider = Arc::new(CountingProvider::new(PASSWORD));
    let mut connect = connect_options(&server, USER, SshAuth::keys(Vec::new()), cache, provider);
    let connection = SshConnection::connect(&mut connect)
        .await
        .expect("连接测试服务端失败");
    let client = connection.sftp().await.expect("SFTP 会话应当建得起来");

    let scratch = Scratch::new("round-trip");
    let local = LocalEndpoint::new();

    // ① 下载：远端 `/remote.bin` → 本机
    let downloaded = scratch.join("downloaded.bin");
    let copied = transfer(
        &client,
        &local,
        &request("/remote.bin", downloaded.to_string_lossy().as_ref()),
        &Progress::default(),
        Cancel::new().waiter(),
    )
    .await
    .expect("下载应当成功");
    assert_eq!(copied, payload.len() as u64);
    assert_eq!(std::fs::read(&downloaded).unwrap(), payload);

    // ② 上传：本机 → 远端 `/uploaded.bin`
    let upload = vec![b'z'; 100 * 1024];
    let local_source = scratch.write("upload.bin", &upload);
    transfer(
        &local,
        &client,
        &request(local_source.to_string_lossy().as_ref(), "/uploaded.bin"),
        &Progress::default(),
        Cancel::new().waiter(),
    )
    .await
    .expect("上传应当成功");

    // ③ 对端**真盘**对账：最终名的字节对得上，而且没有留下任何临时名。
    assert_eq!(
        std::fs::read(root.join("uploaded.bin")).unwrap(),
        upload,
        "对端盘上的字节必须与源相同"
    );
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "remote.bin".to_owned(),
            "sub".to_owned(),
            "uploaded.bin".to_owned()
        ],
        "对端目录里不该多出临时名"
    );

    connection.disconnect().await;
}
