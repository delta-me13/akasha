//! 失败分域。
//!
//! 分域判据是**用户的下一步动作**（`AGENTS.md` §3.4）：
//!
//! | 变体 | 用户要动的地方 |
//! |---|---|
//! | [`BwError::MissingBinary`] | 装一个 `bw`，或者改用运行时下载那一轴 |
//! | [`BwError::NotInstalled`] | 点"下载" |
//! | [`BwError::UnsupportedTarget`] | 这台机器没有对应的资产 —— 换一台，或自己装 |
//! | [`BwError::Network`] | 看网络 / 代理 |
//! | [`BwError::InsecureUrl`] | 把服务器地址改成 https |
//! | [`BwError::NotLoggedIn`] | 先登录 |
//! | [`BwError::NoSession`] | 先解锁 |
//! | [`BwError::CommandFailed`] | 看那句原话（我们认不出的失败原样交给用户） |
//! | 其余 | 内部 / 磁盘 / 输出形状问题 |
//!
//! ⚠️ **只按已知信号分类**：`bw` 的错误措辞随版本变（`docs/bitwarden.md` §7.2 记的是
//! `cli-v2026.8.0` 的实测原文），所以认不出的失败一律走 [`BwError::CommandFailed`]
//! 并**原样带上它的输出**，不猜类别 —— 猜错会把用户指向错的地方。

/// 本 crate 的错误。**没有一个变体带 `source` 字段**：`thiserror` 会把名为 `source` 的字段
/// 当成错误源并改变 `Display` 的拼法（问题 #112），所以统一叫 `reason` / `message`。
#[derive(Debug, thiserror::Error)]
pub enum BwError {
    /// `host` 那一轴上没有 `bw`。
    #[error("这台机器上没有 bw：PATH 里找不到它")]
    MissingBinary,
    /// `managed` 那一轴上还没有下载过。
    #[error("还没有下载过 bw：{root} 下没有任何版本目录")]
    NotInstalled { root: String },
    /// 找是找到了，但跑不起来（权限位、损坏的可执行文件、不是这个平台的格式）。
    #[error("{path} 跑不起来：{reason}")]
    NotRunnable { path: String, reason: String },
    /// 这台机器的平台 / 架构没有对应的上游资产。
    #[error("上游没有 {platform}/{arch} 的资产")]
    UnsupportedTarget { platform: String, arch: String },
    /// 版本解析失败：拿不到列表、或者列表里没有 `cli-v*`。
    #[error("读不出上游的 CLI 版本：{reason}")]
    NoRelease { reason: String },
    /// 网络层失败（DNS、连接、超时）。
    #[error("网络失败：{message}")]
    Network { message: String },
    /// TLS 信任失败 —— 自托管的自签证书走的是这一档。
    #[error("TLS 信任失败：{message}（自签证书要在设置里指出它的 CA）")]
    Tls { message: String },
    /// 明文 HTTP：`bw` 自己也会拒（实测 `InsecureUrlNotAllowedError`），我们在设置那一步先拦。
    #[error("服务器地址必须是 https：{message}")]
    InsecureUrl { message: String },
    /// 服务器地址这一栏填得不合法（空、或者不像个地址）。
    #[error("服务器地址不合法：{message}")]
    InvalidServer { message: String },
    /// `bw` 说没有登录（实测原文 `You are not logged in.`，退出码 1）。
    #[error("还没有登录这个 vault")]
    NotLoggedIn,
    /// `bw` 以非 0 退出，而输出里的信号我们认不出来。
    #[error("bw 以 {code} 退出：{message}")]
    CommandFailed { code: i32, message: String },
    /// `bw` 的输出读不出我们要的东西。**带上原文**：形状变了要能一眼看出来。
    #[error("读不出{what}：{message}")]
    Parse { what: String, message: String },
    /// 压缩包本身或解包过程有问题。
    #[error("压缩包不可用：{message}")]
    Archive { message: String },
    /// 磁盘上的操作失败。`path` 一定要有 —— "写不进去"与"写哪儿"是两个问题。
    #[error("{path}：{reason}")]
    Io { path: String, reason: String },
    /// 子进程超过期限没有结束（我们把它杀了）。
    #[error("bw 超过 {seconds} 秒没有结束")]
    Timeout { seconds: u64 },
    /// session key 超过受保护页的上限。
    #[error("session key 有 {found} 字节，超过上限 {max}")]
    TokenTooLong { found: usize, max: usize },
    /// `bw` 报了个空 session key。
    #[error("bw 没有给出 session key")]
    EmptyToken,
    /// 要不到一块能锁住的受保护页（`mmap` / `mlock` 被拒）。
    ///
    /// 上游没有降级路径，"要不到"意味着**不能进行**，而不是悄悄改用普通堆。
    #[error("session key 放不进受保护页：{message}")]
    ProtectedPage { message: String },
    /// 需要 session key 而手上没有。
    #[error("手上没有 session key：先解锁")]
    NoSession,
}

impl BwError {
    /// 一条 `std::io::Error` 加上它发生在哪个路径上。
    pub(crate) fn io(path: impl std::fmt::Display, err: std::io::Error) -> Self {
        Self::Io {
            path: path.to_string(),
            reason: err.to_string(),
        }
    }
}
