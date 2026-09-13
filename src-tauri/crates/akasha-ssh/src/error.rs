//! `akasha-ssh` 的错误：**分域**定义（`AGENTS.md` §3.4）。
//!
//! 分域的标准是"调用方**处置不同**"：连不上（可以稍后重试）、主机密钥不对
//! （**必须**让用户看见，不能重试）、认证失败（重试只会把账号锁上，ADR-0003 D13）。
//! 这三类的分法直接决定阶段 6 的重连判据，所以错误类型现在就得把它们分开 ——
//! 事后从字符串里认，是那种"看起来能用、一到边界就错"的做法。

use std::time::Duration;

use akasha_store::protected::PageError;

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
    #[error("主机密钥未被接受：{fingerprint}")]
    HostKeyRejected {
        /// 服务端给的密钥指纹。**必须在错误里**：用户要拿它去核对，
        /// 只说"密钥不对"等于什么都没说。
        fingerprint: String,
    },

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
