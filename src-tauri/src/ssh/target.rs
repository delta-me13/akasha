//! 一台 SSH 目标主机 —— 也是凭据缓存键的一半（ADR-0003 D8）。

use std::fmt;

/// `user@host:port`。
///
/// 三个字段都是**构造时**就要有的：它们共同决定"问哪一句口令"，也让缓存键在类型上
/// 完整 —— 缺一个就会出现"同一台主机的两个用户共用一句口令"这种错（而这正是 D8
/// 把 `user` 放进键里的理由）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SshTarget {
    host: String,
    port: u16,
    user: String,
}

impl SshTarget {
    /// 构造一个目标。
    pub fn new(host: impl Into<String>, port: u16, user: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
        }
    }

    /// 主机名或 IP（`~/.ssh/config` 的 `HostName` 语义，plan 0504）。
    pub fn host(&self) -> &str {
        &self.host
    }

    /// 端口。默认 22 由**调用方**给（本 crate 不猜：这里是库，不是配置层）。
    pub fn port(&self) -> u16 {
        self.port
    }

    /// 登录用户名。
    pub fn user(&self) -> &str {
        &self.user
    }

    /// 交给 `TcpStream::connect` 的形态。
    pub(crate) fn address(&self) -> String {
        host_and_port(&self.host, self.port)
    }
}

/// `host:port`，**IPv6 字面量带方括号**。
///
/// 两处用它：连接一个 SSH 目标（[`SshTarget::address`]）与绑定一个本地监听
/// （[`crate::ssh::LocalListener::bind`]）。抽成一份的理由与"建链只有一份实现"相同 ——
/// 少一处手工拼接，就少一处 `::1:46010` 那样的坏地址（它会被当成主机名去解析，
/// 报出来的错指向 DNS，而问题在格式）。
pub(crate) fn host_and_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

impl fmt::Display for SshTarget {
    /// `user@host:port`。错误消息与日志字段都用这一种形态 —— 与 `ssh` 命令行一致，
    /// 用户一眼能对上。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}:{}", self.user, self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn ipv6_literals_get_bracketed() {
        // 不加方括号时 `::1:22` 是一个**坏主机名**，症状是解析失败而不是"连错地方"，
        // 但错误信息会指向 DNS，排查方向完全错。这条断言把它钉死。
        let target = SshTarget::new("::1", 22, "cyrene");
        assert_eq!(target.address(), "[::1]:22");
        assert_eq!(target.to_string(), "cyrene@::1:22");
    }

    #[test]
    fn a_plain_host_is_left_alone() {
        let target = SshTarget::new("example.invalid", 2222, "root");
        assert_eq!(target.address(), "example.invalid:2222");
    }
}
