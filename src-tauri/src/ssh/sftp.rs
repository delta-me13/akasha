//! **SFTP 会话**（plan 0701 / 0702）—— 一条流上的远端文件操作。
//!
//! 形状由 [ADR-0006](../../../../docs/adr/0006-sftp-stack-and-transfer-engine.md) 定：
//! D2 说会话承载在**一条 `AsyncRead + AsyncWrite` 流**上（与 D9 的 `SshStream` 同形），
//! D5 说"经跳板的目标"与 host↔host 的 B 档都是这同一个类型的同一种用法 ——
//! 那时这条流来自 `direct_tcpip`，但这个文件一行都不用改。
//!
//! ## 上游类型不外漏
//!
//! 对外的类型是本仓库自己的（[`Listing`] / [`Entry`]，在 [`crate::ssh::transfer`] 里定义 ——
//! 本机那一栏用的是同一份），`russh_sftp` 的类型留在私有字段与私有实现里
//! —— 同 [`crate::ssh::SshStream`] 把上游 `ChannelStream` 藏起来是同一条纪律：上游换 API 时，
//! 改动止步于这个文件。
//!
//! ## 两件事都在这里：读与写
//!
//! plan 0701 只有 [`Endpoint::list`]；plan 0702 补上 [`Endpoint::open_read`] 与
//! [`Endpoint::begin_write`]。⚠️ **远端这一侧的"临时名 + 原子重命名"落在这里**
//! （ADR-0006 D4）—— 引擎不知道目标在哪，也就不知道这件事该怎么办。
//!
//! ⚠️ 收尾那一下必须**等对端确认**：上游的 `File` 在 drop 时只把 `CLOSE` 发出去、
//! 不等回答（它自己的文档写着"pending write errors and the close status are silently
//! discarded"）。而"写完了"正是重命名之前必须成立的前提 —— 所以这里的每一条收尾路径
//! 都显式 `close()`，不靠 drop。
//!
//! ⚠️ 临时名是**一次 `CREATE|EXCLUDE` 请求抢来的**（plan 0704），不再是"先 `stat` 问一句、
//! 再 `create`"：两处请求之间那道缝会让同名文件的并发传输交错写同一个文件，而由对端保证的
//! 独占没有那道缝 —— 代价是它**分不清**"名字被占"与通用的失败（v3 没有"已存在"这一档），
//! 换下一个候选的规则因此比本机那一侧宽，见 [`SftpClient::claim_temp`]。

use std::sync::Arc;

use russh::client::Handle;
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::fs::File;
use russh_sftp::protocol::{FileType, OpenFlags, StatusCode};

use crate::ssh::error::{SshError, sftp_failed};
use crate::ssh::handshake::Handler;
use crate::ssh::target::SshTarget;
use crate::ssh::transfer::{
    BoxFuture, Endpoint, Entry, EntryKind, FileRead, Listing, PendingWrite, TEMP_ATTEMPTS,
    file_failed, temp_candidates,
};

/// 打开会话时等对端第一条回复的期限（秒）。与上游默认值一致。
///
/// 单独写出来只为一个理由：**它决定用户要等多久才知道对端没开 SFTP**（见
/// [`SftpClient::open_with_timeout`]）。调用方要调短它，走
/// [`crate::ssh::SshConnection::sftp_with_timeout`]。
pub(crate) const SFTP_REQUEST_TIMEOUT_SECS: u64 = 10;

/// 一个 SFTP 会话。
///
/// **句柄语义**：`Clone` 出来的是同一个会话（上游内部是 `Arc`），不是第二条通道 ——
/// 命令层因此可以"从会话表里取出一份句柄、放掉锁、再去 `await`"。
#[derive(Clone)]
pub struct SftpClient {
    inner: Arc<SftpSession>,
    target: SshTarget,
}

impl SftpClient {
    /// 在一条**已认证**的连接上打开 `sftp` 子系统；`timeout_secs` 是"等对端第一条回复"
    /// 的期限。
    ///
    /// 三步都要**在这里**失败得清楚：开通道、请求子系统、初始化会话（版本交换）。
    /// 尤其第三步：对端没开 SFTP 时它不会回 `VERSION`，于是这一句等满期限才报错 ——
    /// 所以错误消息要把"多半是它没开 SFTP"说出来（等第一条请求超时再说"超时"，
    /// 会把人引到网络上去）。
    ///
    /// ⚠️ 这个入口收的是**连接内部的句柄**（`Handle<Handler>` 不在公开 API 里），
    /// 所以调用方走 [`crate::ssh::SshConnection::sftp`] / [`crate::ssh::SshConnection::sftp_with_timeout`]。
    pub(crate) async fn open_with_timeout(
        session: &Handle<Handler>,
        target: &SshTarget,
        timeout_secs: u64,
    ) -> Result<Self, SshError> {
        let channel = session
            .channel_open_session()
            .await
            .map_err(|err| sftp_failed(target, err))?;
        // ⚠️ 上游这一步**只发送、不等回复**（`russh` 的 `channels/mod.rs` 里
        // `request_subsystem` 走的是 `send_msg`）。所以"对端认没认下"看不出在这一句上：
        // 没开 SFTP 的服务端不会回 `VERSION`，客户端在**下面这一句**等到期限为止。
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|err| sftp_failed(target, err))?;
        let config = russh_sftp::client::Config {
            request_timeout_secs: timeout_secs,
            ..russh_sftp::client::Config::default()
        };
        let inner = SftpSession::new_with_config(channel.into_stream(), config)
            .await
            .map_err(|err| match err {
                // 这一档要么是"对端没有这个子系统"，要么是它太慢 —— 两种情形下用户的
                // 下一步动作是同一个（去对端看它有没有开 SFTP），所以给一句能据以行动的话。
                SftpError::Timeout => sftp_failed(
                    target,
                    format!(
                        "对端没有在 {timeout_secs} 秒内应答 sftp 子系统（它那一侧多半没有开 SFTP）"
                    ),
                ),
                other => sftp_failed(target, other),
            })?;
        Ok(Self {
            inner: Arc::new(inner),
            target: target.clone(),
        })
    }

    /// 这个会话开在哪台机器上（错误消息与日志用）。
    pub fn target(&self) -> &SshTarget {
        &self.target
    }

    /// 列一个目录：先 `realpath` 规范化，再读条目。
    ///
    /// 条目按**名字排序**：上游按服务端给的顺序返回（那是随机的），而调用方要拿它做
    /// 界面列表与断言 —— 不确定的顺序会让"同一个目录两次列出不一样"这类假象四处出现。
    async fn list_impl(&self, path: &str) -> Result<Listing, SshError> {
        let dir = self
            .inner
            .canonicalize(path)
            .await
            .map_err(|err| file_failed(path, err))?;
        let mut entries: Vec<Entry> = self
            .inner
            .read_dir(dir.as_str())
            .await
            .map_err(|err| file_failed(path, err))?
            .map(|entry| Entry {
                name: entry.file_name(),
                kind: kind_of(entry.file_type()),
            })
            .collect();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Listing { path: dir, entries })
    }
}

impl Endpoint for SftpClient {
    fn list<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Listing, SshError>> {
        Box::pin(self.list_impl(path))
    }

    fn open_read<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<FileRead, SshError>> {
        let path = path.to_owned();
        Box::pin(async move {
            let file = self
                .inner
                .open(path.as_str())
                .await
                .map_err(|err| file_failed(&path, err))?;
            // 大小从**已经打开的那个句柄**上量（`fstat`）：换一条路径去 `metadata` 会多一次
            // 可能失败的往返，还会给出"打开的是这个、量的是那个"这种说不清的状态。
            let size = file
                .metadata()
                .await
                .map_err(|err| file_failed(&path, err))?
                .size
                .unwrap_or(0);
            Ok(FileRead {
                reader: Box::new(file),
                size,
            })
        })
    }

    fn begin_write<'a>(
        &'a self,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn PendingWrite>, SshError>> {
        let path = path.to_owned();
        Box::pin(async move {
            let (dir, name) = split_path(&path);
            if name.is_empty() {
                return Err(file_failed(&path, "这个路径没有文件名，写不进一个临时名"));
            }
            let (file, temp) = self.claim_temp(&dir, &name, &path).await?;
            Ok(Box::new(RemoteWrite {
                file: Some(file),
                client: self.clone(),
                temp,
                target: path.clone(),
                path,
            }) as Box<dyn PendingWrite>)
        })
    }
}

/// 抢占临时名用的打开标志（ADR-0006 D6）。
///
/// `CREATE|EXCLUDE` 就是 SFTP 协议里的 `O_EXCL`：由**对端**保证"这个名字只给一个人"。
/// 为什么不用 `TRUNCATE`：抢占的前提是"这个名字**原来不存在**"，而 `TRUNCATE` 表达的是
/// "存在也没关系，把内容丢掉" —— 那正好是 0702 那条判据（"别覆盖已存在的临时名"）要防的事。
const TEMP_FLAGS: OpenFlags = OpenFlags::WRITE
    .union(OpenFlags::CREATE)
    .union(OpenFlags::EXCLUDE);

impl SftpClient {
    /// 在 `dir` 里**抢占**一个临时名（候选见 [`temp_candidates`]，最多 [`TEMP_ATTEMPTS`] 个）。
    ///
    /// ⚠️ **抢占是一次请求**：`CREATE|EXCLUDE` 要么把这个名字给我们，要么这一步就失败 ——
    /// 没有"先 `stat` 问一句、再 `open`"那道缝（plan 0704 修的就是它，ADR-0006 D6）。
    ///
    /// ⚠️ **换下一个候选的唯一理由是"这个名字没能独占"**，而它在这里只能表现为**通用的
    /// `Failure`**：SFTP v3 的状态码只有 8 个取值（`russh-sftp` 的 `StatusCode` 就是那 8 个），
    /// 没有"文件已存在"这一档（那是 v4 才补的 `FILE_ALREADY_EXISTS`）。所以：
    ///
    /// * `StatusCode::Failure` → 可能是名字被占，也可能是目录写满一类的通用失败 —— 换下一个；
    /// * **别的错误一律当场报出去**：`PermissionDenied` / `NoSuchFile` 是确定的失败，
    ///   换 16 个候选只是把同一次失败问 16 遍；超时与断连更是（那会变成 16 倍等待）。
    async fn claim_temp(
        &self,
        dir: &str,
        name: &str,
        path: &str,
    ) -> Result<(File, String), SshError> {
        let mut occupied: Option<SshError> = None;
        for candidate in temp_candidates(name).take(TEMP_ATTEMPTS) {
            let full = under(dir, &candidate);
            match self.inner.open_with_flags(full.as_str(), TEMP_FLAGS).await {
                Ok(file) => return Ok((file, full)),
                Err(err) => match &err {
                    SftpError::Status(status) if status.status_code == StatusCode::Failure => {
                        occupied = Some(file_failed(path, err));
                    }
                    _ => return Err(file_failed(path, err)),
                },
            }
        }
        // 到顶了：把**最后一次**的错误报出去（不编一句"名字都被占用了" —— 上面那个 `Failure`
        // 也可能来自"目录写满"，编一句会在那种情形下说假话）。原因进日志，错误本身照原样。
        tracing::warn!(
            path,
            candidates = TEMP_ATTEMPTS,
            "sftp temp name claim exhausted"
        );
        Err(occupied.unwrap_or_else(|| file_failed(path, "没有可用的临时名")))
    }
}

/// 远端这一侧的待落盘写入。
struct RemoteWrite {
    /// `Option` 是为了在重命名 / 删除之前**把远端句柄显式关掉**（见模块文档）。
    file: Option<File>,
    /// 会话句柄（`commit` / `abort` 要拿它发 `rename` / `remove`）。
    client: SftpClient,
    /// 临时名的完整路径。
    temp: String,
    /// 最终名的完整路径（`commit` 的把子）。
    target: String,
    /// 出错消息里那个"用户眼中的路径"（就是 `begin_write` 收到的那个）。
    path: String,
}

impl RemoteWrite {
    /// 关掉远端句柄，**等对端确认**（见模块文档：drop 会把写入的结局丢掉）。
    async fn close_file(&mut self) -> Result<(), SshError> {
        if let Some(file) = self.file.take() {
            file.close()
                .await
                .map_err(|err| file_failed(&self.path, err))?;
        }
        Ok(())
    }
}

impl PendingWrite for RemoteWrite {
    fn write_chunk<'a>(&'a mut self, chunk: &'a [u8]) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(async move {
            let file = self
                .file
                .as_mut()
                .ok_or_else(|| file_failed(&self.path, "临时文件已经关掉了"))?;
            tokio::io::AsyncWriteExt::write_all(file, chunk)
                .await
                .map_err(|err| file_failed(&self.path, err))
        })
    }

    fn commit<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(async move {
            // ⚠️ 顺序不能换：句柄没关就重命名，等于在用的还是那个临时文件。
            self.close_file().await?;
            self.client
                .inner
                .rename(self.temp.as_str(), self.target.as_str())
                .await
                .map_err(|err| file_failed(&self.path, err))
        })
    }

    fn abort<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(async move {
            self.close_file().await?;
            match self.client.inner.remove_file(self.temp.as_str()).await {
                Ok(()) => Ok(()),
                // ⚠️ **幂等**：临时名已经不在（`commit` 成功之后再叫一次）不是失败。
                // SFTP 说"没有这个文件"的方式是状态码，不是 io 的 `NotFound`。
                Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
                    Ok(())
                }
                Err(err) => Err(file_failed(&self.path, err)),
            }
        })
    }
}

/// 上游的类型 → 我们的。**不做穷尽之外的猜测**：认不出来的一律 [`EntryKind::Other`]。
fn kind_of(file_type: FileType) -> EntryKind {
    match file_type {
        FileType::Dir => EntryKind::Directory,
        FileType::File => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        FileType::Other => EntryKind::Other,
    }
}

/// 把一个 SFTP 路径拆成"目录"与"文件名"。
///
/// 手写而不是用 `std::path`：SFTP 的路径规范是 POSIX 的（`/` 分隔，与**本机**是什么系统
/// 无关），拿平台路径类型去解析会在 Windows 上把 `C:` 之类的当盘符。
fn split_path(path: &str) -> (String, String) {
    match path.rfind('/') {
        Some(0) => ("/".to_owned(), path[1..].to_owned()),
        Some(index) => (path[..index].to_owned(), path[index + 1..].to_owned()),
        None => (String::new(), path.to_owned()),
    }
}

/// 把 `name` 放到 `dir` 下面（`dir` 为空 = 相对路径，`/` 不重复）。
fn under(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        return name.to_owned();
    }
    if dir == "/" {
        return format!("/{name}");
    }
    format!("{dir}/{name}")
}
