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
        /// 这一条失败属于哪一类。
        ///
        /// ⚠️ 它不是"reason 的结构化版本"这种锦上添花：动态转发（plan 0603）**只能**把
        /// 这一个字节回给 SOCKS5 客户端，而客户端看到的就是这条功能的错误消息。从
        /// `reason` 那串字符串里认类别是那种"改一次上游文案就静默失效"的判据 ——
        /// 上游给的本来就是结构化的 `ChannelOpenFailure`，只是此前被 `to_string` 抹平了。
        class: ForwardFailure,
    },

    /// 本地监听绑定失败（plan 0602 的 `-L`）：端口被占用、没有权限、地址不可用。
    ///
    /// ⚠️ 与 [`SshError::Connect`] 分开是必要的：那条说的是"**对端**连不上"，
    /// 这条说的是"**本机**的端口没拿到"。用户要做的动作完全不同 ——
    /// 前者查网络与远端服务，后者腾出端口或换一个绑定地址。
    /// 它同时是**最先**可能失败的一步：绑定在握手之前，所以它一出错就不必再问凭据。
    #[error("本地监听 {address} 绑定失败：{reason}")]
    Listen {
        /// 想绑的地址（`host:port`）。
        address: String,
        /// 操作系统的原话（`Address already in use (os error 98)` 那一种）。
        reason: String,
    },

    /// 远端监听没拿到（plan 0604 的 `-R`）：服务端拒绝在 `address` 上监听。
    ///
    /// ⚠️ 与 [`SshError::Listen`] 分开是必要的：那一条说的是"**本机**的端口没拿到"，
    /// 用户腾一个端口或换一个绑定地址即可；这一条要动的地方在**服务端**
    /// （那个端口被它自己占着，或者它根本不允许远端转发）—— 在本机上做什么都没用。
    ///
    /// 服务端回的是 RFC 4254 里那句不带原因的"请求失败"，所以能说清的只有
    /// "它拒绝了"以及可能的两种情形（见 `reason`）。
    #[error("远端监听 {address} 没拿到：{reason}")]
    RemoteListen {
        /// 请求服务端监听的地址（`host:port`，`host` 按服务端那一侧解释）。
        address: String,
        /// 上游的原话，或我们对"请求被拒"的展开。
        reason: String,
    },

    /// 动态转发（SOCKS5）的监听地址不是回环地址（plan 0603 的安全项）。
    ///
    /// ⚠️ 与 [`SshError::Listen`] 分开：那一条是"这个地址没拿到"，用户换个端口就好；
    /// 这一条是"这个地址**不许**绑"，换端口没用 —— 要改的是绑定的**网卡范围**。
    ///
    /// SOCKS5 这一侧无认证（RFC 1928 的 `0x00` 是唯一接受的方法），绑到 `0.0.0.0`
    /// 等于把"经这台跳板机访问远端网络"的能力交给同网段的所有人。**故意做成拒绝而不是
    /// 警告**：警告在没有人看的日志里，而端口是真的在听。
    #[error("SOCKS5 监听不能绑到 {address}：这一侧无认证，只允许绑回环地址")]
    NotLoopback {
        /// 规则里想绑的地址。
        address: String,
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
///
/// 收 `russh::Error` 而不是 `impl Display`：类别要从**结构化**的错误里取
/// （`ChannelOpenFailure`），折成字符串之后就只剩猜了（见 [`ForwardFailure`]）。
pub(crate) fn forward_failed(host: &str, port: u16, err: &russh::Error) -> SshError {
    SshError::Forward {
        host: host.to_owned(),
        port,
        reason: err.to_string(),
        class: ForwardFailure::classify(err),
    }
}

/// 本地监听那条路的统一说法（plan 0602）。
pub(crate) fn listen_failed(address: &str, err: impl std::fmt::Display) -> SshError {
    SshError::Listen {
        address: address.to_owned(),
        reason: err.to_string(),
    }
}

/// 远端监听那条路的统一说法（plan 0604 的 `-R`）。
///
/// `RequestDenied` 在上游只是一个光秃秃的名字，而它有两种成因（端口被服务端那一侧占着 /
/// 服务端不允许远端转发），两者对用户的下一步动作不同 —— 所以在这一处展开成能据以行动的话；
/// 其余错误照旧把上游原话带上。
pub(crate) fn remote_listen_failed(address: &str, err: &russh::Error) -> SshError {
    let reason = match err {
        russh::Error::RequestDenied => {
            "服务端拒绝了这条转发请求：那个端口在它那一侧被占着，或它不允许远端转发".to_owned()
        }
        other => other.to_string(),
    };
    SshError::RemoteListen {
        address: address.to_owned(),
        reason,
    }
}

/// `direct-tcpip` 开不出来的**类别**。
///
/// 分出来只有一个理由：动态转发（plan 0603）的 SOCKS5 客户端**只能收到一个 `REP` 字节**
/// —— 那个字节就是这条功能的错误消息。把"服务端不允许转发"与"目标服务没起来"压成同一个
/// `0x01`，用户就无从下手了。
///
/// 认不出来的一律 [`Self::Other`]：**不猜**。宁可少说，不要把一个协议层的错误说成
/// "目标拒绝连接"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardFailure {
    /// 对端不允许这条转发（`AdministrativelyProhibited`）。
    ///
    /// 与其余几档分开的价值最大：它不是"这次运气不好"，而是**对端的配置**不允许，
    /// 重试一百次也一样 —— 用户该去改的是服务端的 `AllowTcpForwarding` 一类。
    Prohibited,
    /// 对端连不上目标（`ConnectFailed`）：目标服务没起来、端口写错了。
    ConnectFailed,
    /// 对端不认 `direct-tcpip` 这种通道（`UnknownChannelType`）。
    UnknownChannelType,
    /// 对端资源不够（`ResourceShortage`）。
    ResourceShortage,
    /// 其余（协议层错误、连接已断、超时、上游新增的变体）。**不猜**。
    Other,
}

impl ForwardFailure {
    /// 从上游的错误里认出类别。
    pub(crate) fn classify(err: &russh::Error) -> Self {
        let russh::Error::ChannelOpenFailure(failure) = err else {
            return Self::Other;
        };
        match failure {
            russh::ChannelOpenFailure::AdministrativelyProhibited => Self::Prohibited,
            russh::ChannelOpenFailure::ConnectFailed => Self::ConnectFailed,
            russh::ChannelOpenFailure::UnknownChannelType => Self::UnknownChannelType,
            russh::ChannelOpenFailure::ResourceShortage => Self::ResourceShortage,
            // 上游以后加变体时走这里（而不是编译不过）：那一档对我们就是"别的"。
            russh::ChannelOpenFailure::Other { .. } => Self::Other,
        }
    }
}
