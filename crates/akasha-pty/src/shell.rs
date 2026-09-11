//! shell 启动参数的**唯一来源**（`AGENTS.md` §3.3：不散落）。
//!
//! shell 的启动方式是一段**平台相关且容易做错**的逻辑：登录 shell 的 argv0 约定、
//! 环境变量白名单、cwd 兜底。散落成多处，就必然出现"某一条路径少设了 `SHELL`"，
//! 而那种差别只在特定命令下才暴露。

use portable_pty::CommandBuilder;

/// 启动哪个 shell、带什么参数。
///
/// 默认值**委托**给 `portable-pty` 的 `new_default_prog()`，因为它已经做对了三件事：
///
/// * **登录 shell**：把 argv0 前缀成 `-bash`（这决定了 `/etc/profile` 会不会被读）
/// * **环境**：先 `env_clear()`，再放回 `SHELL` 与一份基础白名单 —— 不是无差别继承
/// * **cwd**：未指定时落在用户 home，而不是继承 app 的 cwd
///
/// 自己再实现一遍等于把这三条重写一遍，而且日后必然与上游漂移。
/// 需要确定性时（测试、将来的"用户自定义 shell"）用 [`ShellLaunch::new`] 显式指定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellLaunch {
    program: Option<String>,
    args: Vec<String>,
}

impl ShellLaunch {
    /// 平台默认的登录 shell（`$SHELL` → 口令库 → 平台默认）。
    pub const fn default_shell() -> Self {
        Self {
            program: None,
            args: Vec::new(),
        }
    }

    /// 显式指定程序（例如测试里的 `/bin/sh`）。
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: Some(program.into()),
            args: Vec::new(),
        }
    }

    /// 追加参数。
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// 程序名；`None` 表示"用平台默认 shell"。
    pub fn program(&self) -> Option<&str> {
        self.program.as_deref()
    }

    /// 显式参数（默认 shell 下恒为空 —— 登录 shell 的参数由上游决定）。
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// 转成 portable-pty 的 builder。
    ///
    /// **crate 内唯一**把 [`ShellLaunch`] 变成真实命令的地方：要改启动方式，改这里一处。
    pub(crate) fn to_command_builder(&self) -> CommandBuilder {
        match &self.program {
            // `new_default_prog()` 的 builder 上调用 `arg()` 会 panic，所以两条分支
            // 不能混用 —— 这也是把它收在一个 `match` 里的原因。
            None => CommandBuilder::new_default_prog(),
            Some(program) => {
                let mut builder = CommandBuilder::new(program);
                for arg in &self.args {
                    builder.arg(arg);
                }
                builder
            }
        }
    }
}

impl Default for ShellLaunch {
    fn default() -> Self {
        Self::default_shell()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shell_defers_the_program_to_the_platform() {
        let launch = ShellLaunch::default_shell();

        assert_eq!(
            launch.program(),
            None,
            "默认 shell 由平台决定，不由我们硬编码"
        );
        assert!(launch.args().is_empty());
        assert!(
            launch.to_command_builder().is_default_prog(),
            "默认分支必须走 new_default_prog（登录 shell + env 白名单 + home 兜底都在它里面）"
        );
    }

    #[test]
    fn explicit_program_and_args_reach_the_command_line() {
        let launch = ShellLaunch::new("/bin/sh").with_args(["-c", "true"]);
        let builder = launch.to_command_builder();
        let argv: Vec<String> = builder
            .get_argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        assert!(!builder.is_default_prog());
        assert_eq!(argv.first().map(String::as_str), Some("/bin/sh"));
        assert_eq!(argv, vec!["/bin/sh", "-c", "true"]);
    }
}
