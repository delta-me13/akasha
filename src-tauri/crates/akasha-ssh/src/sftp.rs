//! **SFTP 会话**（plan 0701）—— 一条流上的远端文件操作。
//!
//! 形状由 [ADR-0006](../../../../docs/adr/0006-sftp-stack-and-transfer-engine.md) 定：
//! D2 说会话跑在**一条 `AsyncRead + AsyncWrite` 流**上（与 D9 的 `SshStream` 同形），
//! D5 说"经跳板的目标"与 host↔host 的 B 档都是这同一个类型的同一种用法 ——
//! 那时这条流来自 `direct_tcpip`，但这个文件一行都不用改。
//!
//! ## 上游类型不外漏
//!
//! 对外的类型是本仓库自己的 [`SftpEntry`] / [`SftpListing`]，`russh_sftp` 的类型留在私有字段里
//! —— 同 [`crate::SshStream`] 把上游 `ChannelStream` 藏起来是同一条纪律：上游换 API 时，
//! 改动止步于这个文件。
//!
//! ## 本阶段只做**读**
//!
//! plan 0701 的判据是"两侧各自列目录成功"，所以这里只有 [`SftpClient::list`]。
//! 写路径（临时名 + 原子重命名、并发 in-flight、取消时的清理）属于 plan 0702–0704，
//! 它们要在**端点**那一层定形状（ADR-0006 D4），不在这里逐个加方法。

use std::sync::Arc;

use russh::client::Handle;
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::FileType;

use crate::error::{SshError, sftp_failed};
use crate::handshake::Handler;
use crate::target::SshTarget;

/// 打开会话时等对端第一条回复的期限（秒）。与上游默认值一致。
///
/// 单独写出来只为一个理由：**它决定用户要等多久才知道对端没开 SFTP**（见
/// [`SftpClient::open_with_timeout`]）。调用方要调短它，走
/// [`crate::SshConnection::sftp_with_timeout`]。
pub(crate) const SFTP_REQUEST_TIMEOUT_SECS: u64 = 10;

/// 目录条目的类型。
///
/// 自己一个枚举而不是把上游的 `FileType` 交出去：理由同模块文档（上游类型不外漏），
/// 且本仓库这一侧的取值是给界面用的稳定短名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SftpKind {
    File,
    Directory,
    Symlink,
    Other,
}

impl SftpKind {
    /// 上游的类型 → 我们的。**不做穷尽之外的猜测**：认不出来的一律 [`Self::Other`]。
    fn of(file_type: FileType) -> Self {
        match file_type {
            FileType::Dir => Self::Directory,
            FileType::File => Self::File,
            FileType::Symlink => Self::Symlink,
            FileType::Other => Self::Other,
        }
    }
}

/// 一条目录条目。本阶段只要名字与类型（大小属于传输的进度，plan 0702）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpEntry {
    pub name: String,
    pub kind: SftpKind,
}

/// 一次列目录的结果。
///
/// `path` 是**规范化之后**的路径（`realpath`）：调用方拿它当"当前目录"，
/// 于是"返回上一级"不必由谁去猜字符串（服务端返回的可能是 `~` 展开后的写法）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpListing {
    pub path: String,
    pub entries: Vec<SftpEntry>,
}

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
    /// 所以调用方走 [`crate::SshConnection::sftp`] / [`crate::SshConnection::sftp_with_timeout`]。
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

    /// 列一个目录：先 `realpath` 规范化，再读条目。
    ///
    /// 条目按**名字排序**：上游按服务端给的顺序返回（那是随机的），而调用方要拿它做
    /// 界面列表与断言 —— 不确定的顺序会让"同一个目录两次列出不一样"这类假象四处出现。
    pub async fn list(&self, path: &str) -> Result<SftpListing, SshError> {
        let target = self.target.clone();
        let dir = self
            .inner
            .canonicalize(path)
            .await
            .map_err(|err| sftp_failed(&target, err))?;
        let mut entries: Vec<SftpEntry> = self
            .inner
            .read_dir(dir.as_str())
            .await
            .map_err(|err| sftp_failed(&target, err))?
            .map(|entry| SftpEntry {
                name: entry.file_name(),
                kind: SftpKind::of(entry.file_type()),
            })
            .collect();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(SftpListing { path: dir, entries })
    }

    /// 这个会话开在哪台机器上（错误消息与日志用）。
    pub fn target(&self) -> &SshTarget {
        &self.target
    }
}
