//! **动态转发 `-D` 的 SOCKS5 服务端**（plan 0603，RFC 1928）。
//!
//! 只做**无认证的 `CONNECT`**：浏览器与 `curl` 用的是这一条，而其余三种形态
//! （认证协商、`BIND`、UDP associate）在这里**明确拒绝**并回对应的 `REP`，
//! 不是装作没看见。协议错一个字节，客户端报的就是一句无从下手的失败 ——
//! 所以本模块的每一步都按 RFC 的字节序写，且不替客户端猜。
//!
//! ## 目标由客户端说，解析权在对端
//!
//! 握手交出来的 [`ForwardTarget`] 会被**原样**送进 `direct_tcpip`：域名不做本机解析
//! （本机解析等于绕开跳板机，与 `-L` 同一条理由）。`ATYP` 是域名时因此**不得**先
//! `to_socket_addrs` 再送 IP —— 那样目标就从"对端网络里的名字"变成了"我们网络里的地址"。
//!
//! ## 回给客户端的那个字节
//!
//! 成功要等**通道开出来之后**才回（RFC 1928 §6 的顺序），失败则回一个**分类过的**
//! `REP`：客户端能收到的就只有这一个字节，它承担了整条功能的错误消息。分类由
//! [`crate::ForwardFailure`]（上游 `ChannelOpenFailure` 的结构化结果）翻过来，
//! 不从错误字符串里猜。
//!
//! `BND.ADDR` / `BND.PORT` 只能是占位 `0.0.0.0:0`：SSH 的通道确认报文里没有对端的
//! 绑定地址（RFC 4254 的确认只有通道号与窗口），编一个"看起来像"的地址就是撒谎。
//! 客户端在 `CONNECT` 这一档允许忽略它们（RFC 1928 §6 的原话是"客户端可以忽略"）。

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

use crate::error::SshError;
use crate::relay::ForwardTarget;

/// 协议版本（RFC 1928 §3）。
const VERSION: u8 = 0x05;
/// 唯一接受的认证方法：无认证。
const METHOD_NO_AUTH: u8 = 0x00;
/// "没有一个方法可用"（RFC 1928 §3 规定的那一个）。
const METHOD_NONE_ACCEPTABLE: u8 = 0xFF;
/// `CONNECT`（RFC 1928 §4）。
const CMD_CONNECT: u8 = 0x01;
/// `ATYP` 的三种取值（RFC 1928 §5）。
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
/// 一次握手的读上限。
///
/// 没有它会留下一条**低成本**的挂死：客户端连上来发一个字节就再也不说话，
/// 那条连接与那条 `direct_tcpip` 通道都不必开，但任务一直挂着（一条 TCP 就能占一个任务）。
/// 10 s 远大于任何真实客户端的握手时间，也让"半截报文"这类用例能在合理时间内结束。
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);

/// 握手失败的形态 —— **要不要回东西、回哪一个字节**都在这里定死。
///
/// 分三档而不是一个 `io::Error`：`REP` 的字节是客户端唯一的解释，而"连版本都不对"
/// 与"没有可用的认证方法"该回的东西**不同**（前者什么都不回，后者回 `05 FF`）。
#[derive(Debug, thiserror::Error)]
pub enum SocksError {
    /// 版本不对。
    ///
    /// **什么都不回**：客户端说的不是 SOCKS5，它读不懂我们的回复，回一个 5 字节的东西
    /// 只会让它把我们的回复当成别的协议的字节。
    #[error("不是 SOCKS5 的问候（版本 {version}）")]
    BadVersion { version: u8 },

    /// 客户端没有提供无认证这一个方法。
    ///
    /// 与 [`Self::BadVersion`] 分开：这一条**必须**回 `05 FF`（RFC 1928 §3），
    /// 客户端据此才知道"代理不支持你要的方法"，而不是把连接被关当成网络故障。
    #[error("客户端不支持无认证（它提供了 {methods} 种方法）")]
    NoAcceptableMethod { methods: usize },

    /// 请求阶段被拒 —— **必须**回 [`Self::Rejected::reply`]。
    #[error("SOCKS5 请求被拒（REP {code}）：{reason}")]
    Rejected {
        /// 回给客户端的 `REP`。
        reply: Reply,
        /// `REP` 的字节值（日志与错误消息里要看得见它）。
        code: u8,
        /// 给人看的理由。
        reason: String,
    },

    /// 读写出错（客户端半途走了、连接断了、超过 [`HANDSHAKE_DEADLINE`]）。
    #[error("SOCKS5 握手读写失败：{0}")]
    Io(#[from] std::io::Error),
}

impl SocksError {
    /// 该回给客户端的 `REP`。`None` = 什么都不回（见 [`SocksError::BadVersion`]）。
    pub const fn reply(&self) -> Option<Reply> {
        match self {
            Self::Rejected { reply, .. } => Some(*reply),
            // `NoAcceptableMethod` 回的是问候阶段的 `05 FF`，不是 `REP`。
            Self::BadVersion { .. } | Self::NoAcceptableMethod { .. } | Self::Io(_) => None,
        }
    }
}

/// 一个 `REP`（RFC 1928 §6）。**值就是给客户端的字节**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// `0x00` 成功。
    Success,
    /// `0x01` 通用失败。
    GeneralFailure,
    /// `0x02` 规则不允许。
    NotAllowed,
    /// `0x05` 目标拒绝连接。
    ConnectionRefused,
    /// `0x07` 不支持这个命令（只有 `CONNECT` 被支持）。
    CommandNotSupported,
    /// `0x08` 不支持这种地址形态。
    AddressTypeNotSupported,
}

impl Reply {
    /// 给客户端的字节。
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0x00,
            Self::GeneralFailure => 0x01,
            Self::NotAllowed => 0x02,
            Self::ConnectionRefused => 0x05,
            Self::CommandNotSupported => 0x07,
            Self::AddressTypeNotSupported => 0x08,
        }
    }
}

/// 完成一次握手，交出客户端要去的目标。
///
/// 交出来的只是**目标**：连接本身在这一步之后才建（`direct_tcpip`），
/// 而"连上了"那一个成功 `REP` 由 [`confirm`] 发 —— 顺序是 RFC 1928 §6 要求的。
pub(crate) async fn negotiate<S>(stream: &mut S) -> Result<ForwardTarget, SocksError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    timeout(HANDSHAKE_DEADLINE, greeting(stream))
        .await
        .map_err(|_| {
            SocksError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "SOCKS5 握手超时",
            ))
        })?
}

/// 问候 → 读请求（超时包在外面）。
async fn greeting<S>(stream: &mut S) -> Result<ForwardTarget, SocksError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let version = stream.read_u8().await?;
    if version != VERSION {
        return Err(SocksError::BadVersion { version });
    }
    let count = usize::from(stream.read_u8().await?);
    if count == 0 {
        // `NMETHODS = 0` 是客户端在说"我一个方法都不提供"。RFC 没有为它定义回复，
        // 而"一个方法都没有"与"提供的方法里没有无认证"对客户端是同一件事。
        stream.write_all(&[VERSION, METHOD_NONE_ACCEPTABLE]).await?;
        return Err(SocksError::NoAcceptableMethod { methods: 0 });
    }
    let mut methods = vec![0u8; count];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&METHOD_NO_AUTH) {
        stream.write_all(&[VERSION, METHOD_NONE_ACCEPTABLE]).await?;
        return Err(SocksError::NoAcceptableMethod { methods: count });
    }
    stream.write_all(&[VERSION, METHOD_NO_AUTH]).await?;

    let version = stream.read_u8().await?;
    if version != VERSION {
        return Err(SocksError::BadVersion { version });
    }
    let command = stream.read_u8().await?;
    // 保留字节（`RSV`）。RFC 要求发 0，收到非 0 没有定义 —— 照收，不据此拒绝：
    // 它不改变请求的含义，为此断掉一条本来能用的连接不值得。
    let _reserved = stream.read_u8().await?;

    let atyp = stream.read_u8().await?;
    let host = match atyp {
        ATYP_IPV4 => {
            let mut raw = [0u8; 4];
            stream.read_exact(&mut raw).await?;
            Ipv4Addr::from(raw).to_string()
        }
        ATYP_IPV6 => {
            let mut raw = [0u8; 16];
            stream.read_exact(&mut raw).await?;
            Ipv6Addr::from(raw).to_string()
        }
        ATYP_DOMAIN => {
            let len = usize::from(stream.read_u8().await?);
            if len == 0 {
                // 空域名不是一个地址，而 `direct_tcpip` 拿它去连只会得到对端的一句
                // 难懂的话 —— 在这里挡掉，理由说清楚。
                return reject(
                    stream,
                    Reply::AddressTypeNotSupported,
                    "域名长度是 0".to_owned(),
                )
                .await;
            }
            let mut raw = vec![0u8; len];
            stream.read_exact(&mut raw).await?;
            // 域名走 UTF-8 解码：SOCKS5 没有规定域名的编码，而实际上它总是 ASCII
            // （国际化域名在客户端那一侧就已经转成 punycode）。解不出来就报错，
            // 不 lossy 改写 —— 改写出来的名字连的是**另一个**主机。
            match String::from_utf8(raw) {
                Ok(host) => host,
                Err(_) => {
                    return reject(
                        stream,
                        Reply::AddressTypeNotSupported,
                        "域名不是 UTF-8".to_owned(),
                    )
                    .await;
                }
            }
        }
        other => {
            return reject(
                stream,
                Reply::AddressTypeNotSupported,
                format!("不认的地址形态 ATYP {other:#04x}"),
            )
            .await;
        }
    };
    let port = stream.read_u16().await?;

    if command != CMD_CONNECT {
        // `BIND`（0x02）与 UDP associate（0x03）在这里被明确拒绝 —— 那两种形态
        // 本版本不做（plan 0603 的非目标），而"回了 0x07"比"静默把 BIND 当 CONNECT
        // 处理"要好得多：后者会让客户端以为端口在听。
        return reject(
            stream,
            Reply::CommandNotSupported,
            format!("命令 {command:#04x} 不支持（只做 CONNECT）"),
        )
        .await;
    }

    if host.is_empty() {
        return reject(
            stream,
            Reply::AddressTypeNotSupported,
            "目标地址是空的".to_owned(),
        )
        .await;
    }

    tracing::debug!(host, port, atyp, "socks5 connect requested");
    Ok(ForwardTarget::new(host, port))
}

/// 发一个失败 `REP`，然后把这个失败原样交出去（调用方负责关连接）。
async fn reject<S>(
    stream: &mut S,
    reply: Reply,
    reason: String,
) -> Result<ForwardTarget, SocksError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // 回不出去（对端已经走了）不该把"为什么拒绝"盖掉：那是给日志与用例看的。
    let _ = write_reply(stream, reply).await;
    Err(SocksError::Rejected {
        reply,
        code: reply.code(),
        reason,
    })
}

/// 通道开出来了：回成功的 `REP`。
///
/// 与 [`reject`] 分开的理由是顺序：RFC 1928 §6 要求"连上目标之后"才回 `0x00`，
/// 而"连上"是 SSH 那一侧的事 —— 所以这个函数只在 `direct_tcpip` 成功之后被调用。
pub(crate) async fn confirm<S>(stream: &mut S) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    write_reply(stream, Reply::Success).await
}

/// 通道开不出来：回对应的 `REP`（客户端能收到的唯一解释）。
pub(crate) async fn refuse<S>(stream: &mut S, reply: Reply) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    write_reply(stream, reply).await
}

/// 写一条 `REP` 报文（`VER REP RSV ATYP BND.ADDR BND.PORT`）。
///
/// `BND.ADDR` 固定是 `0.0.0.0`：见模块文档的"回给客户端的那个字节"。
async fn write_reply<S>(stream: &mut S, reply: Reply) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&[VERSION, reply.code(), 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
        .await
}

/// 把一条通道开不出来的**类别**翻成 `REP`。
///
/// 这张表就是"用户能看到的分类"：压成同一个 `0x01` 会让"服务端不允许转发"与
/// "目标服务没起来"变得无法区分（见 [`crate::ForwardFailure`]）。
pub(crate) const fn reply_for(failure: crate::ForwardFailure) -> Reply {
    use crate::ForwardFailure as F;
    match failure {
        F::Prohibited => Reply::NotAllowed,
        F::ConnectFailed => Reply::ConnectionRefused,
        F::UnknownChannelType => Reply::CommandNotSupported,
        F::ResourceShortage | F::Other => Reply::GeneralFailure,
    }
}

/// 保证一个绑定地址是**回环**的 —— 非回环就拒绝（plan 0603 的安全项）。
///
/// 允许 `127.0.0.1` / `[::1]` 这类字面量，以及 `localhost`：它在正常机器上就是回环，
/// 而在"`localhost` 被指到别处"的机器上，用户要动的是 `/etc/hosts`，不是这里的判断。
pub(crate) fn ensure_loopback(host: &str) -> Result<(), SshError> {
    let trimmed = host.trim();
    if trimmed.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }
    // `[::1]` 这种带方括号的写法（`bind_host` 里可以这么写）先剥掉括号再解析。
    let bare = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(trimmed);
    if bare
        .parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
    {
        return Ok(());
    }
    Err(SshError::NotLoopback {
        address: trimmed.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn a_no_auth_greeting_is_accepted_and_a_domain_request_yields_the_target() {
        let (mut client, mut server) = duplex(1024);
        let serving = tokio::spawn(async move {
            let target = negotiate(&mut server).await.unwrap();
            confirm(&mut server).await.unwrap();
            target
        });

        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut chosen = [0u8; 2];
        client.read_exact(&mut chosen).await.unwrap();
        assert_eq!(chosen, [0x05, 0x00], "必须选中无认证");

        let mut request = vec![0x05, 0x01, 0x00, 0x03, 0x0c];
        request.extend_from_slice(b"target.local");
        request.extend_from_slice(&8443u16.to_be_bytes());
        client.write_all(&request).await.unwrap();

        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(
            reply,
            [0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0],
            "成功 REP 与占位的 BND.ADDR"
        );

        let target = serving.await.unwrap();
        assert_eq!(target.host(), "target.local");
        assert_eq!(target.port(), 8443);
    }

    #[tokio::test]
    async fn an_ipv4_and_an_ipv6_target_are_read_in_their_wire_forms() {
        // IPv4：`ATYP 0x01` + 4 字节。
        let (mut client, mut server) = duplex(1024);
        let serving = tokio::spawn(async move { negotiate(&mut server).await.unwrap() });
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut chosen = [0u8; 2];
        client.read_exact(&mut chosen).await.unwrap();
        let mut request = vec![0x05, 0x01, 0x00, 0x01, 10, 0, 0, 7];
        request.extend_from_slice(&80u16.to_be_bytes());
        client.write_all(&request).await.unwrap();
        let target = serving.await.unwrap();
        assert_eq!(target.host(), "10.0.0.7");
        assert_eq!(target.port(), 80);

        // IPv6：`ATYP 0x04` + 16 字节。
        let (mut client, mut server) = duplex(1024);
        let serving = tokio::spawn(async move { negotiate(&mut server).await.unwrap() });
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut chosen = [0u8; 2];
        client.read_exact(&mut chosen).await.unwrap();
        let mut request = vec![0x05, 0x01, 0x00, 0x04];
        request.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        request.extend_from_slice(&443u16.to_be_bytes());
        client.write_all(&request).await.unwrap();
        let target = serving.await.unwrap();
        assert_eq!(target.host(), "::1");
        assert_eq!(target.port(), 443);
    }

    #[tokio::test]
    async fn a_client_without_no_auth_is_refused_with_ff() {
        let (mut client, mut server) = duplex(1024);
        let serving = tokio::spawn(async move { negotiate(&mut server).await.unwrap_err() });

        // 只提供"用户名/口令"（0x02）—— 我们不做认证协商。
        client.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
        let mut reply = [0u8; 2];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [0x05, 0xFF]);

        let err = serving.await.unwrap();
        assert!(
            matches!(err, SocksError::NoAcceptableMethod { methods: 1 }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn extra_methods_do_not_hide_the_supported_one() {
        let (mut client, mut server) = duplex(1024);
        let serving = tokio::spawn(async move { negotiate(&mut server).await });
        client
            .write_all(&[0x05, 0x03, 0x02, 0x80, 0x00])
            .await
            .unwrap();
        let mut chosen = [0u8; 2];
        client.read_exact(&mut chosen).await.unwrap();
        assert_eq!(chosen, [0x05, 0x00], "0x00 在第三个也要被选中");
        drop(client);
        // 客户端走了 → 读请求得到 EOF。
        assert!(matches!(
            serving.await.unwrap(),
            Err(SocksError::Io(_)) | Err(SocksError::BadVersion { .. })
        ));
    }

    #[tokio::test]
    async fn a_bind_command_is_refused_with_command_not_supported() {
        let (mut client, mut server) = duplex(1024);
        let serving = tokio::spawn(async move { negotiate(&mut server).await.unwrap_err() });
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut chosen = [0u8; 2];
        client.read_exact(&mut chosen).await.unwrap();
        let mut request = vec![0x05, 0x02, 0x00, 0x01, 127, 0, 0, 1];
        request.extend_from_slice(&0u16.to_be_bytes());
        client.write_all(&request).await.unwrap();

        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], 0x07, "BIND 必须回 0x07");
        assert_eq!(reply[0], 0x05);
        match serving.await.unwrap() {
            SocksError::Rejected { reply, .. } => assert_eq!(reply, Reply::CommandNotSupported),
            other => panic!("BIND 必须被拒，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unknown_address_type_is_refused_with_address_type_not_supported() {
        let (mut client, mut server) = duplex(1024);
        let serving = tokio::spawn(async move { negotiate(&mut server).await.unwrap_err() });
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut chosen = [0u8; 2];
        client.read_exact(&mut chosen).await.unwrap();
        client
            .write_all(&[0x05, 0x01, 0x00, 0x09, 1, 2])
            .await
            .unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], 0x08, "不认的 ATYP 必须回 0x08");
        match serving.await.unwrap() {
            SocksError::Rejected { reply, .. } => {
                assert_eq!(reply, Reply::AddressTypeNotSupported);
            }
            other => panic!("不认的 ATYP 必须被拒，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_wrong_version_is_closed_without_a_reply() {
        let (mut client, mut server) = duplex(1024);
        let serving = tokio::spawn(async move { negotiate(&mut server).await.unwrap_err() });
        client.write_all(&[0x04, 0x01, 0x00]).await.unwrap();
        let mut buffer = [0u8; 2];
        // 服务端不回任何东西：读要么挂到超时，要么在对面返回之后得到 EOF。
        let read = timeout(Duration::from_millis(200), client.read(&mut buffer)).await;
        assert!(
            read.is_err() || read.unwrap().unwrap_or(0) == 0,
            "版本不对时不该有回复"
        );
        match serving.await.unwrap() {
            SocksError::BadVersion { version } => assert_eq!(version, 4),
            other => panic!("版本不对必须报 BadVersion，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_truncated_request_is_an_io_error_not_a_hang() {
        let (mut client, mut server) = duplex(1024);
        let serving = tokio::spawn(async move { negotiate(&mut server).await.unwrap_err() });
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut chosen = [0u8; 2];
        client.read_exact(&mut chosen).await.unwrap();
        // 声明了 12 字节的域名，只给 3 个字节就断开。
        client
            .write_all(&[0x05, 0x01, 0x00, 0x03, 0x0c, b'a', b'b', b'c'])
            .await
            .unwrap();
        drop(client);
        // 必须在有限时间内结束（超时是 10 s，EOF 立刻到）。
        let err = timeout(Duration::from_secs(5), serving)
            .await
            .expect("半截报文挂住了")
            .unwrap();
        assert!(matches!(err, SocksError::Io(_)), "{err:?}");
    }

    #[test]
    fn reply_codes_are_the_wire_values() {
        // 这些数字是协议的一部分，不是实现细节 —— 改它们等于换一个协议。
        assert_eq!(Reply::Success.code(), 0x00);
        assert_eq!(Reply::GeneralFailure.code(), 0x01);
        assert_eq!(Reply::NotAllowed.code(), 0x02);
        assert_eq!(Reply::ConnectionRefused.code(), 0x05);
        assert_eq!(Reply::CommandNotSupported.code(), 0x07);
        assert_eq!(Reply::AddressTypeNotSupported.code(), 0x08);
    }

    #[test]
    fn only_loopback_addresses_are_allowed_for_socks5() {
        // 绑回环的四种写法都要放行（`localhost` 与 `[::1]` 是用户在规则里真会写的）。
        for host in ["127.0.0.1", "::1", "[::1]", "localhost", " 127.0.0.1 "] {
            assert!(ensure_loopback(host).is_ok(), "{host} 应当被放行");
        }
        // 非回环一律拒绝 —— 这一侧无认证，开放到同网段等于把跳板交出去。
        for host in ["0.0.0.0", "::", "192.168.1.10", "example.com", ""] {
            match ensure_loopback(host) {
                Err(SshError::NotLoopback { address }) => {
                    assert_eq!(address, host.trim(), "错误里要带上想绑的那条地址");
                }
                other => panic!("{host} 必须被拒，实际 {other:?}"),
            }
        }
    }

    #[test]
    fn a_failure_class_becomes_the_closest_reply() {
        use crate::ForwardFailure as F;
        assert_eq!(reply_for(F::Prohibited), Reply::NotAllowed);
        assert_eq!(reply_for(F::ConnectFailed), Reply::ConnectionRefused);
        assert_eq!(reply_for(F::UnknownChannelType), Reply::CommandNotSupported);
        assert_eq!(reply_for(F::ResourceShortage), Reply::GeneralFailure);
        assert_eq!(reply_for(F::Other), Reply::GeneralFailure);
    }
}
