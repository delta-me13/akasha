//! `akasha-ssh` 的错误：**分域**定义（`AGENTS.md` §3.4）。
//!
//! 分域的标准是"调用方**处置不同**"：连不上（可以稍后重试）、主机密钥不对
//! （**必须**让用户看见，不能重试）、认证失败（重试只会把账号锁上，ADR-0003 D13）。
//! 这三类的分法直接决定阶段 6 的重连判据，所以错误类型现在就得把它们分开 ——
//! 事后从字符串里认，是那种"看起来能用、一到边界就错"的做法。

use std::time::Duration;

use akasha_store::protected::PageError;

use crate::known_hosts::RecordedIn;
use crate::target::SshTarget;

/// 连接、认证或通道操作失败的原因。
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    /// 空口令。与 `akasha-store` 的 `Passphrase` 同一条理由：**空值造不出来**
    /// （一个能被误传的空值迟早会被误传）。
    #[error("凭据不能为空")]
    EmptyCredential,

    /// 受保护页要不到或装不下。**不是异常**：`mlock` 失败意味着"不能进行"，
    /// 而不是"悄悄不锁"（ADR-0002 D13）。
    #[error("机密页不可用：{0}")]
    Protection(#[from] PageError),

    /// 口令不是 UTF-8。`russh` 的认证接口只收字符串（`impl Into<String>`），
    /// 而 OpenSSH 的口令原则上可以是任意字节 —— 这条限制照实报出来，不静默改写。
    #[error("凭据不是 UTF-8：russh 的认证接口只收字符串")]
    CredentialNotUtf8,

    /// 问凭据的那条路自己失败了（app 侧接 IPC、用户取消 ……）。
    #[error("凭据要不到：{0}")]
    CredentialUnavailable(String),

    /// 连不上：DNS、TCP、协议协商、握手中的任意一步。
    #[error("连接 {target} 失败：{reason}")]
    Connect {
        /// 目标（`user@host:port`）。
        target: SshTarget,
        /// 底层的说法原样带上 —— 它是唯一能指认"哪一步坏了"的信息。
        reason: String,
    },

    /// 超时。**与 [`SshError::Connect`] 分开**：一个是"对端说不行"，一个是"没人应答"。
    #[error("连接 {target} 超时（{after:?}）")]
    ConnectTimeout {
        /// 目标（`user@host:port`）。
        target: SshTarget,
        /// 我们给的期限。
        after: Duration,
    },

    /// 服务端的主机密钥没被接受。**这条永远不该被自动重试**（ADR-0003 D13）。
    ///
    /// 两种情形共用一个变体：钉住的指纹对不上（[`crate::PinnedHostKey`]），
    /// 或者**用户明确否认**了一把没见过的密钥。两者对用户的下一步动作是同一个 ——
    /// 核对指纹，然后决定要不要改配置 / 重新确认。
    #[error("主机密钥未被接受：{fingerprint}")]
    HostKeyRejected {
        /// 服务端给的密钥指纹。**必须在错误里**：用户要拿它去核对，
        /// 只说"密钥不对"等于什么都没说。
        fingerprint: String,
    },

    /// 服务端给的密钥与**记下来的**不一样（ADR-0003 D11：拒绝并提示，既不静默接受、
    /// 也不静默改写）。
    ///
    /// 与 [`SshError::HostKeyRejected`] 分开是刻意的：这一条是**警报**（中间人攻击的典型
    /// 形态就是这一步），而"没见过"只是一种常态。阶段 6 的重连判据也靠这个区分
    /// —— D13 把"主机密钥不匹配"列为**不重连**。
    #[error(
        "主机密钥变了：{host}:{port} 记的是 {recorded}，现在给的是 {presented}（记录在{recorded_in}）"
    )]
    HostKeyChanged {
        host: String,
        port: u16,
        /// 记录里的那一把（用户上次确认的）。
        recorded: String,
        /// 服务端这次给的。
        presented: String,
        /// 记录在哪 —— 我们的缓存，还是用户的 `~/.ssh/known_hosts`。
        recorded_in: RecordedIn,
    },

    /// 没见过这把密钥，而**没有可问的人**（`HostKeyPrompt` 没配）。
    ///
    /// ⚠️ 它**不是**"接受"的近义词：这条路径必须由调用方翻译成一次用户提问
    /// （plan 0504），在那之前它就是一句明确的拒绝。D11 写死了这一点。
    #[error("主机密钥没见过，需要确认：{host}:{port} {fingerprint}")]
    HostKeyUnknown {
        host: String,
        port: u16,
        /// 要请用户核对的那串 `SHA256:…`。
        fingerprint: String,
    },

    /// 主机密钥**用不了**：上游给的那把我们编码不出线格式的本体。
    ///
    /// 这不是"密钥不对"，而是"我们连它是哪把都说不出" —— 而判定材料正是本体
    /// （逐字节比，见 [`crate::KnownHostsVerifier`]），所以只能拒绝。
    /// 触发它的是服务端报了本版本 `ssh-key` 不认识的算法名；实际很难遇到，
    /// 但"很难遇到"不等于可以拿一段空字节顶替（那会让判定退化成"比空"）。
    #[error("主机密钥无法编码（{algorithm}）：{reason}")]
    HostKeyUnusable { algorithm: String, reason: String },

    /// **库那一侧的 known_hosts 缓存读写失败**（plan 0504 的适配器）。
    ///
    /// 最常见的形态是"库锁着"：信任记录与密钥池都在库里，所以 app 的连接命令会先要求解锁 ——
    /// 但这条错误还要覆盖"库出错 / 写入被拒"那几种（写冲突 = 有人在我们之后记了另一把密钥）。
    ///
    /// ⚠️ **它一律意味着拒绝连接**，不是"当作未知继续"：把"读不到信任记录"降级成"没记录"
    /// 会把 D11 的三态判定退化成两态，而缺的那一态正是警报那一态。
    #[error("主机密钥缓存不可用：{0}")]
    HostKeyCache(String),

    /// 认证失败：我们有的方式全试过了，服务端还剩别的。
    #[error("认证失败：{target} 上没有可用的方式（服务端还剩 {remaining}）")]
    AuthenticationFailed {
        /// 目标（`user@host:port`）。
        target: SshTarget,
        /// 服务端最后告诉我们"还剩什么"。空串 = 它一条都没说。
        remaining: String,
    },

    /// 打开会话通道 / 请求 pty / 请求 shell 失败。
    #[error("打开会话通道失败：{0}")]
    Channel(String),

    /// `direct-tcpip`（跳板 / 转发）失败：**对端**拒绝或够不着 `host:port`（plan 0505）。
    ///
    /// ⚠️ 与 [`SshError::Connect`] **分开**是刻意的：那条说的是"我连不上这台机器"，
    /// 这条说的是"这台机器连不上那台"。两者的下一步动作不同 —— 前者查网络与端口，
    /// 后者要问的是**跳板机**能不能看见目标（`ChannelOpenFailure::ConnectFailed` 就是
    /// 最常见的那一种）。压成一句话会把排查方向指错。
    #[error("转发到 {host}:{port} 失败：{reason}")]
    Forward {
        /// 对端要去连的地址（**不是**我们连的那台）。
        host: String,
        /// 对端要去连的端口。
        port: u16,
        /// 上游的原话（`ConnectFailed` / `AdministrativelyProhibited` 一类）。
        reason: String,
    },

    /// 在 tokio 上下文里调同步门面。**返回错误而不是 panic**：
    /// `Handle::block_on` 在这里会 panic（"Cannot start a runtime from within a runtime"），
    /// 而这条路径的调用方是 app 的命令层 —— 那里 panic 会连带丢掉整个 app。
    #[error("不能在 tokio 上下文里调用同步门面（会在同一线程上自锁）")]
    BlockingInsideRuntime,
}

/// 把 `russh` 的错误折成我们的说法。
///
/// 刻意**不做穷尽匹配**：上游 0.x 的 `Error` 变体还会变，而这层要做的事只有
/// "原话带上"。真正需要分类的判定（认证失败 / 主机密钥变化）发生在它们**自己的**
/// 调用点上（`AuthResult` / `HostKeyRejected`），不靠解析这里。
pub(crate) fn connect_failed(target: &SshTarget, err: impl std::fmt::Display) -> SshError {
    SshError::Connect {
        target: target.clone(),
        reason: err.to_string(),
    }
}

/// 通道操作失败的统一说法。
pub(crate) fn channel_failed(err: impl std::fmt::Display) -> SshError {
    SshError::Channel(err.to_string())
}

/// `direct-tcpip` 那条路的统一说法（原语在 [`crate::SshConnection::direct_tcpip`]）。
pub(crate) fn forward_failed(host: &str, port: u16, err: impl std::fmt::Display) -> SshError {
    SshError::Forward {
        host: host.to_owned(),
        port,
        reason: err.to_string(),
    }
}
