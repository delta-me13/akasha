//! **传输引擎**（ADR-0006 D4）—— 把**一个端点**上的一个文件搬进**另一个端点**。
//!
//! 形状由 ADR-0006 D4 定死，三条约束互相咬合，改一条就要重答另外两条：
//!
//! 1. **引擎只认两个端点。** 它不认识 SSH、SFTP，也不认识本机路径：源与目标各是一个
//!    [`Endpoint`]，能列目录、能开一个读端、能开一个**待落盘**的写入。于是
//!    "直连目标 / 经跳板的目标 / host↔host 的 B 档"是**选哪一对端点**，
//!    引擎一行都不用改（ADR-0006 D5）。
//! 2. **临时名 + 原子重命名归目标端点**（`scope.md` §4.2）。引擎因此不知道"落盘"意味着
//!    什么 —— 本机那端是 `std::fs` 同目录写 `.name.part` 再 `rename`，远端那端是 SFTP 的
//!    `open` / `write` / `rename`。引擎只负责：要么 [`PendingWrite::commit`]，
//!    要么 [`PendingWrite::abort`]，**没有第三条出口**。
//! 3. **取消只有一条路径**（D4）：用户取消与关闭 `Session` 都推同一个 [`Cancel`]，
//!    清理由目标端点的 [`PendingWrite::abort`] 完成。取消、读失败、写失败、
//!    重命名失败 —— 四条退出路径的结尾都是同一句"临时名不留给用户"。
//!
//! ## 为什么请求/响应的形状是这样
//!
//! 端点的三个动作都是异步的，而引擎要把它们**当作 trait 对象**用（两端在运行期才决定是
//! 本机还是某台主机）。trait 里的 `async fn` 不是 dyn 兼容的，所以这里手工写
//! [`BoxFuture`] —— 一个类型别名，不是新依赖。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::watch;

use crate::ssh::error::SshError;

/// 引擎一次搬多少字节。
///
/// 32 KiB：SFTP 的包上限通常就是 32 KiB（`limits@openssh.com` 缺失时的通行值），
/// 一次读满即一次请求，既不会白白多一轮往返，也不需要扩展协商。
pub const CHUNK_BYTES: usize = 32 * 1024;

/// 引擎侧的异步返回：`Box<dyn Future>` 的别名（端点的动作要能被 `dyn` 用，见模块文档）。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ── 目录：两端共用同一份词汇 ────────────────────────────────────────────────
//
// 它放在这个模块而不是 `sftp.rs`：本机那一栏与远端那一栏在界面上是**同一种东西**，
// 两份类型只会让调用方写两次转换。

/// 一条目录条目的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// 一条目录条目。本阶段只要名字与类型（大小属于传输的进度，见 [`Progress`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
}

/// 一次列目录的结果。
///
/// `path` 是**规范化之后**的路径：调用方拿它当"当前目录"，于是"返回上一级"不必由谁去猜
/// 字符串（远端给的是 `realpath` 的结果，本机给的是 `canonicalize` 的结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub path: String,
    pub entries: Vec<Entry>,
}

// ── 端点 ────────────────────────────────────────────────────────────────────

/// 一个打开的文件读端：**带上它的大小**（进度要有分母）。
///
/// 两件事一起给，是因为它们的来路相同（都是"打开这个文件"这一步知道的）——
/// 分两次问只会多一轮往返，并让"大小读到一半才失败"成为一种可能。
pub struct FileRead {
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
    pub size: u64,
}

/// 一个**待落盘**的写入（ADR-0006 D4 的第 2 条）。
///
/// 调用方只做三件事之一：推块、[`commit`](PendingWrite::commit)、
/// [`abort`](PendingWrite::abort)。临时名怎么取、最终名怎么换来，全在这一层里面 ——
/// 这是"三种拓扑共用同一个引擎"能成立的原因。
pub trait PendingWrite: Send {
    /// 推一块。调用方按 [`CHUNK_BYTES`] 分块，不保证块边界与协议包边界对齐。
    ///
    /// ⚠️ **返回 `Ok` 说的是"端点收下了这一块"，不是"它已经落盘"**：
    /// 本机端点建在 `tokio::fs` 上，而后者在**派发**阻塞写之后立刻返回 `Ready`
    /// （真正的 `write(2)` 要等下一次 poll）；远端那一侧的写入也要等对端的回复。
    /// 全部落地由 [`commit`](PendingWrite::commit) 保证 —— 那也正是这条不变量
    /// （"看到最终名就等于成功"）唯一需要的位置。中间态因此只可能是**一段前缀**
    /// （可能短，绝不可能是别的东西、也绝不可能是完整长度）。
    fn write_chunk<'a>(&'a mut self, chunk: &'a [u8]) -> BoxFuture<'a, Result<(), SshError>>;

    /// 成功：把临时名**原子重命名**成最终名。这一刻之后，用户看到最终名就等于成功。
    fn commit<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>>;

    /// 失败 / 取消：删掉临时名。
    ///
    /// ⚠️ **必须幂等**：`commit` 失败之后调用方还会再叫一次（那时临时名可能已经不在，
    /// 也可能已经换成了最终名）—— 一次"删不掉"的报错不该变成用户看到的失败原因。
    fn abort<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>>;
}

/// **一个端点**：引擎眼里的"一端"。
///
/// 实现者有两类：本机文件系统（`crate::ssh::local::LocalEndpoint`）与一个 SFTP 会话
/// （[`crate::ssh::SftpClient`]）。host↔host 的两档（ADR-0006 D5）不新增实现 ——
/// 它们是**哪两个端点配对**的问题。
pub trait Endpoint: Send + Sync {
    /// 列一个目录。
    ///
    /// ⚠️ 生命期写成 `'a` 而不是省略：返回的 future **借用 `path`**（实现里要把它搬进
    /// 异步块），而省略规则只会把它绑到 `&self` 上 —— 于是"借了 path 却只声明借了 self"
    /// 编译不过。写成同一个 `'a` 就是"两者都借到最短的那个为止"，正是这里的语义。
    fn list<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Listing, SshError>>;

    /// 打开一个文件读。
    fn open_read<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<FileRead, SshError>>;

    /// 开一个待落盘的写入（临时名 + 原子重命名在实现里，见 [`PendingWrite`]）。
    fn begin_write<'a>(
        &'a self,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn PendingWrite>, SshError>>;
}

// ── 取消 ────────────────────────────────────────────────────────────────────

/// 一次传输的中止信号。
///
/// `Clone` 出来的是**同一个信号**（不是两个）—— 命令层、实体表与引擎各持一份，
/// 谁推它都算数。用法：命令层 [`Cancel::cancel`]，引擎拿 [`Cancel::waiter`] 去等。
///
/// ⚠️ 不用 `impl Future<Output = ()>` 那种一次性信号（`SshConnection::connect_via_until` 的
/// 做法）：那个形状只够"连一次"，而一条传输要在一个循环里问很多次"还算数吗" ——
/// 更要紧的是，推它的人（另一条命令）与等它的人不在同一个调用栈上。
#[derive(Clone)]
pub struct Cancel(Arc<CancelInner>);

struct CancelInner {
    /// `watch` 而不是 `Notify`：它自带"当前值"，于是**先推后等**不会丢信号
    /// （`Notify::notify_waiters` 只叫醒此刻已经注册的等待者，那个竞态要靠
    /// `Notified::enable` 手工绕开）。
    flag: watch::Sender<bool>,
}

impl Cancel {
    /// 一个还没被推过的信号。
    pub fn new() -> Self {
        let (flag, _) = watch::channel(false);
        Self(Arc::new(CancelInner { flag }))
    }

    /// 推它。**幂等**，且推过之后 [`Cancel::is_cancelled`] 永远为真。
    pub fn cancel(&self) {
        // ⚠️ `send_replace` 而不是 `send`：`send` 在**一个订阅者都没有**时直接返回 `Err`
        // 并且**不写入**那个值（上游实现先查订阅者数）。于是"先推信号、后拿等待端"这条路上
        // 的取消会**静默丢失** —— 表现是传输永远停不下来，而推信号的那一边毫无察觉。
        // 这一条由 `cancel_is_sticky_and_does_not_race` 实测红出来（第一版就是 `send`）。
        self.0.flag.send_replace(true);
    }

    /// 推过没有。
    pub fn is_cancelled(&self) -> bool {
        *self.0.flag.borrow()
    }

    /// 拿一个等待端（**持有一份 [`Cancel`]**：于是只要还有人在等，信号就不会被丢掉）。
    pub fn waiter(&self) -> CancelWaiter {
        CancelWaiter {
            flag: self.0.flag.subscribe(),
            _owner: self.clone(),
        }
    }
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

/// [`Cancel::waiter`] 拿到的等待端。
pub struct CancelWaiter {
    flag: watch::Receiver<bool>,
    /// 只为了让 `Cancel` 活得比等待端长（见 [`Cancel::waiter`]）。
    _owner: Cancel,
}

impl CancelWaiter {
    /// 等到被推；**已经推过就立刻返回**。
    pub async fn wait(&mut self) {
        if *self.flag.borrow() {
            return;
        }
        // `Err` 只可能是"发送端没了" —— 而等待端自己持有一份 `Cancel`（`_owner`），
        // 所以这一支不可达；真到了那里也只能返回（没有任何人能再推它）。
        let _ = self.flag.changed().await;
    }
}

// ── 进度 ────────────────────────────────────────────────────────────────────

/// 一次传输的字节读数：**引擎写它，命令层与探针读它**。
///
/// 原子而不是 `Mutex`：读的人（IPC、探针）与写的人（搬字节的那条任务）在两条线上，
/// 而这里要的只是两个数，锁一次只为读两个 `u64` 不划算。
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    /// 已经搬过去的字节数。
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    /// 源文件的大小（`open_read` 之后才知道，之前是 0）。
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    fn advance(&self, bytes: u64) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
    }
}

// ── 引擎 ────────────────────────────────────────────────────────────────────

/// 一次搬运的输入：源路径与目标路径（各自在**它自己那个端点**上的写法）。
#[derive(Debug, Clone, Copy)]
pub struct TransferRequest<'a> {
    pub source_path: &'a str,
    pub target_path: &'a str,
}

/// 把源端点的一个文件搬进目标端点，返回搬过去的字节数。
///
/// ⚠️ **成功的定义是"目标端点已经把临时名换成了最终名"**，不是"字节写完了"：
/// 只有前者才让"用户看到最终名就等于成功"成立（`scope.md` §4.2）。
/// 其余每一条退出路径（取消 / 读失败 / 写失败 / 重命名失败）都先把临时名删掉再返回。
///
/// 端点的失败**原样传出去**（`SshError::File` 已经带着路径与端点自己的说法）——
/// 引擎不重写它，也就不会把"对端怎么说"换成"引擎觉得怎么样"。
///
/// 取消是**协作式**的：它只在两次"推块"之间被检查（`select!` 的两条臂，见 [`copy`]）。
/// 已经在路上的一次读或写会走完（SFTP 的请求超时是它的上界），随后才收尾 ——
/// 于是"关会话"最多等一次往返，而不会等整个文件搬完。
pub async fn transfer(
    source: &dyn Endpoint,
    target: &dyn Endpoint,
    request: &TransferRequest<'_>,
    progress: &Progress,
    mut cancel: CancelWaiter,
) -> Result<u64, SshError> {
    let FileRead { mut reader, size } = source.open_read(request.source_path).await?;
    progress.set_total(size);

    let mut sink = target.begin_write(request.target_path).await?;

    let copied = match copy(&mut *reader, &mut *sink, request, progress, &mut cancel).await {
        Ok(copied) => copied,
        Err(err) => {
            discard(&mut *sink, request, &err).await;
            return Err(err);
        }
    };

    if let Err(err) = sink.commit().await {
        // 重命名失败之后临时名多半还在 —— 但它也可能已经被换掉了，所以这里是**尽力而为**
        // （`abort` 幂等，见它的文档）。
        discard(&mut *sink, request, &err).await;
        return Err(err);
    }
    Ok(copied)
}

/// 搬字节：读一块、写一块、记一次进度，直到读完或被推取消。
///
/// `select!` 的两条臂都是取消安全的：读没读成时丢掉那个 future 是无损的（下一圈重读
/// 同一块），而取消那一臂一旦就绪就再也不等。
async fn copy(
    reader: &mut (dyn AsyncRead + Send + Unpin),
    sink: &mut dyn PendingWrite,
    request: &TransferRequest<'_>,
    progress: &Progress,
    cancel: &mut CancelWaiter,
) -> Result<u64, SshError> {
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut copied = 0u64;
    loop {
        let filled = tokio::select! {
            read = reader.read(&mut buffer) => read.map_err(|err| file_failed(request.source_path, err))?,
            () = cancel.wait() => return Err(SshError::Cancelled),
        };
        // 读完的标记是"这一块一个字节都没有"，不是某个长度。
        if filled == 0 {
            return Ok(copied);
        }
        sink.write_chunk(&buffer[..filled])
            .await
            .map_err(|err| file_failed(request.target_path, err))?;
        copied += filled as u64;
        progress.advance(filled as u64);
    }
}

/// 文件操作失败：把路径与端点自己的说法放在一起。
///
/// 位置放在这一层是因为**两个端点都要用它**（本机与远端各写一份只会慢慢漂移），
/// 而它要说清的正是"哪一个文件"—— 端点的原话只说得出"它那一边怎么坏的"。
pub(crate) fn file_failed(path: &str, reason: impl std::fmt::Display) -> SshError {
    SshError::File {
        path: path.to_owned(),
        reason: reason.to_string(),
    }
}

/// 失败 / 取消的收尾：删掉临时名。
///
/// ⚠️ **原始错误优先**：清理自己失败只记一条警告。把那句警告当成这次传输的失败原因，
/// 用户看到的就会是一句关于 `.part` 的话，而"为什么没成"消失了。
async fn discard(sink: &mut dyn PendingWrite, request: &TransferRequest<'_>, cause: &SshError) {
    if let Err(err) = sink.abort().await {
        tracing::warn!(
            source_path = request.source_path,
            target_path = request.target_path,
            cause = %cause,
            error = %err,
            "transfer temp file cleanup failed"
        );
    }
}

/// 临时名的候选：`.name.part`，已被占用时退到 `.name.N.part`（N 从 2 起）。
///
/// 由引擎这一层定义而不是各端点各写一份：`scope.md` §4.2 举的例子就是 `.name.part`，
/// 而 ADR-0006 D6 要求同一批里两个同名文件不能撞在同一个临时名上 ——
/// 端点负责在候选里挑第一个**抢得到**的（怎么算"抢到"由端点的介质决定：本机是 `O_EXCL`，
/// 远端是 `CREATE|EXCLUDE`，见 [`TEMP_ATTEMPTS`]）。
pub fn temp_candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    std::iter::once(format!(".{name}.part")).chain((2..).map(move |n| format!(".{name}.{n}.part")))
}

/// 一条传输最多试几个临时名。
///
/// 候选本身是无限的（`.name.2.part`、`.name.3.part`、…），但真正的冲突只会是少数几个 ——
/// 无条件遍历下去等于把"抢不到名字"变成一次死循环。到顶之后报出来的必须是一条**真的**
/// 错误（端点的原话），不能编一句"名字都被占用了"：远端那种介质分不出"被占"与"权限不足"
/// （见 `crate::ssh::sftp` 的 `claim_temp`）。
pub(crate) const TEMP_ATTEMPTS: usize = 16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_names_start_at_the_documented_shape() {
        let names: Vec<String> = temp_candidates("report.pdf").take(3).collect();
        assert_eq!(
            names,
            vec![
                ".report.pdf.part",
                ".report.pdf.2.part",
                ".report.pdf.3.part"
            ]
        );
    }

    /// 取消是"推过就永远为真"，而且**先推后等**不会丢（`watch` 带来的性质）。
    #[tokio::test]
    async fn cancel_is_sticky_and_does_not_race() {
        let cancel = Cancel::new();
        assert!(!cancel.is_cancelled());

        // 先推，再拿等待端 —— 等待端必须立刻返回，而不是等一个永远不会再来的变化。
        cancel.cancel();
        let mut waiter = cancel.waiter();
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter.wait())
            .await
            .expect("已经推过的信号必须立刻被等到");

        assert!(cancel.is_cancelled());
        // 幂等：再推一次不改变任何事。
        cancel.cancel();
        assert!(cancel.is_cancelled());
    }
}
