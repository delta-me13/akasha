//! 一次 `bw` 调用的形状：参数、子进程环境、超时、输出捕获与失败分类。
//!
//! ## 机密怎么交给子进程
//!
//! 两条都**不走 argv**（ADR-0007 D8 / D7）：
//!
//! | 机密 | 怎么给 | 为什么不走 argv |
//! |---|---|---|
//! | 主密码 | 环境变量 `BW_PASSWORD` + `login` / `unlock` 的 `--passwordenv` | argv 对同机进程可见（`ps` 就能读到），而 `--passwordfile` 会把主密码落盘 |
//! | session key | 环境变量 `BW_SESSION`（CLI 自己会读它，官方文档 §"Using a session key"） | 同上；`--session <key>` 会把 key 放进 argv |
//!
//! ⚠️ **残余暴露面**：`bw` 存活期间，同用户进程可读 `/proc/<pid>/environ`。这与
//! `russh` 需要一份普通 `String` 口令是同一类无法消除的副本（ADR-0003 D8 已如实记录一次）。
//!
//! ## 为什么一定要 `--nointeraction` 且把 stdin 接到 `null`
//!
//! 缺凭据时 `bw` 会走 inquirer 的交互提问：它逐字符回显（实测：管里出现
//! `? Email address: a` `ak` `aka` …），并且能一直等下去。一条没有终点的等待会把
//! app 的那把锁一直占着，所以两件事一起做：**禁掉提问**，并让 stdin 立刻 EOF。
//!
//! ## 为什么输出是 `Vec<u8>` 而不是 `String`
//!
//! session key 要从这里**搬进**受保护页（[`crate::session`]）。先变成 `String` 就会在
//! 普通堆上多留一份 —— `Vec<u8>` 可以直接交给 `Protected::new`，它成功时会把源缓冲擦零。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::BwError;
use crate::location::Located;
use crate::session::Session;
use crate::status::{self, Status};
use crate::variant::{self, Variant};

/// 主密码用的环境变量名（`--passwordenv` 的参数就是它）。
const PASSWORD_ENV: &str = "BW_PASSWORD";

/// session key 用的环境变量名。CLI 自己读它，因此不需要 `--session`。
const SESSION_ENV: &str = "BW_SESSION";

/// 两次 `try_wait` 之间的间隔。**不是"等异步完成的猜测"**（`AGENTS.md` §0 禁止 #5）：
/// 这里是在等一个我们已经持有的子进程，`try_wait` 是它唯一的同步问法。
const POLL: Duration = Duration::from_millis(20);

/// 三类命令各自的期限。默认值在 [`crate::timeout`]；**可换**是为了让"超时"这条路径
/// 能被测到 —— 一条 20 秒的默认期限没法在单测里等。
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub local: Duration,
    pub status: Duration,
    pub network: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            local: crate::timeout::LOCAL,
            status: crate::timeout::STATUS,
            network: crate::timeout::NETWORK,
        }
    }
}

/// 一个已经解析好落点的 CLI。
#[derive(Debug, Clone)]
pub struct Cli {
    program: PathBuf,
    appdata: Option<PathBuf>,
    extra_ca: Option<PathBuf>,
    timeouts: Timeouts,
}

/// 一次调用的输出（**非 0 退出也在这里**，分类是 [`Cli::run_checked`] 的事）。
#[derive(Debug, Clone)]
pub struct Output {
    /// 正常退出的退出码；被信号杀掉的进程没有它，取 `-1`。
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    /// stdout 的文本形态（坏字节变成替换字符）。**只给消息用**，不要拿它做机密搬运。
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// 两步登录（`--method` + `--code`）。取值见 `docs/bitwarden.md` §7：`0` 认证器 /
/// `1` 邮件 / `3` YubiKey；FIDO2 与 Duo 上游 CLI 不支持。
#[derive(Debug, Clone, Copy)]
pub struct TwoFactor<'a> {
    pub method: &'a str,
    pub code: &'a str,
}

impl Cli {
    pub fn new(located: &Located) -> Self {
        Self {
            program: located.program.clone(),
            appdata: located.appdata.clone(),
            extra_ca: None,
            timeouts: Timeouts::default(),
        }
    }

    /// 换掉三类期限（默认值见 [`crate::timeout`]）。
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// 让子进程信任一张额外的 CA 证书（`NODE_EXTRA_CA_CERTS`）。
    ///
    /// 自托管服务器常常用自签证书，而 `bw` 是个 Node SEA —— 官方文档给的正是这条路，
    /// 实测有效（`docs/bitwarden.md` §7.3）。
    pub fn with_extra_ca(mut self, path: Option<PathBuf>) -> Self {
        self.extra_ca = path;
        self
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// `bw --version`（两版变体同值，分辨变体见 [`Cli::variant`]）。
    pub fn version(&self) -> Result<String, BwError> {
        let out = self.run_checked(&["--version"], &[], self.timeouts.local)?;
        let text = out.stdout_text().trim().to_owned();
        if text.is_empty() {
            return Err(BwError::Parse {
                what: "版本号".to_owned(),
                message: "输出是空的".to_owned(),
            });
        }
        Ok(text)
    }

    /// 判定这一份是 OSS 还是专有（读 `--help` 的命令表，`docs/bitwarden.md` §2.2）。
    pub fn variant(&self) -> Result<Variant, BwError> {
        let out = self.run_checked(&["--help"], &[], self.timeouts.local)?;
        Ok(variant::from_help(&out.stdout_text()))
    }

    /// `bw status --raw` → [`Status`]。**这是三态的唯一真相**（ADR-0007 D10）。
    pub fn status(&self) -> Result<Status, BwError> {
        let out = self.run_checked(&["status", "--raw"], &[], self.timeouts.status)?;
        status::parse(&out.stdout_text())
    }

    /// 读回当前配置的服务器地址（`bw config server` 无参数即是读）。
    ///
    /// 上游在**没设过**时也回得出一个值（官方云的地址），因此 `None` 表示的是
    /// "输出是空的"这一异常，而不是"用的是默认值"。
    pub fn server(&self) -> Result<Option<String>, BwError> {
        let out = self.run_checked(&["config", "server"], &[], self.timeouts.local)?;
        let text = out.stdout_text().trim().to_owned();
        Ok((!text.is_empty()).then_some(text))
    }

    /// 设自托管服务器地址，返回**读回来的生效值**。
    ///
    /// 明文 HTTP 在**这一步**就被拒（ADR-0007 D9）：`bw` 自己也会拒，但那次拒绝发生在
    /// `login` 时、且错误里的地址是它自己拼出来的 `/api` 地址，用户看不出是哪一步错了。
    pub fn set_server(&self, url: &str) -> Result<String, BwError> {
        let url = url.trim();
        if url.is_empty() {
            return Err(BwError::InvalidServer {
                message: "是空的".to_owned(),
            });
        }
        if url
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        {
            return Err(BwError::InsecureUrl {
                message: url.to_owned(),
            });
        }
        self.run_checked(&["config", "server", url], &[], self.timeouts.local)?;
        // 读回生效值：上游会自己补前缀（帮助里 `bw config server bitwarden.com` 就是例子），
        // 界面上要显示的是它认下来的那一个，而不是用户敲进去的那一串。
        Ok(self.server()?.unwrap_or_else(|| url.to_owned()))
    }

    /// 登录，成功时交出 session key。
    ///
    /// ⚠️ 未实测的部分：需要一个真实 vault 才能看到"口令对不对"这一档的原文
    /// （`docs/bitwarden.md` §8）。
    pub fn login(
        &self,
        email: &str,
        password: &str,
        two_factor: Option<TwoFactor<'_>>,
    ) -> Result<Session, BwError> {
        let email = email.trim();
        if email.is_empty() {
            return Err(BwError::Parse {
                what: "邮箱".to_owned(),
                message: "是空的".to_owned(),
            });
        }
        let mut args = vec![
            "login",
            email,
            "--passwordenv",
            PASSWORD_ENV,
            "--raw",
            "--nointeraction",
        ];
        if let Some(two_factor) = two_factor {
            args.extend(["--method", two_factor.method, "--code", two_factor.code]);
        }
        let out = self.run_checked(&args, &[(PASSWORD_ENV, password)], self.timeouts.network)?;
        Session::from_output(out.stdout)
    }

    /// 解锁，成功时交出新的 session key（旧的随之失效）。
    pub fn unlock(&self, password: &str) -> Result<Session, BwError> {
        let out = self.run_checked(
            &[
                "unlock",
                "--passwordenv",
                PASSWORD_ENV,
                "--raw",
                "--nointeraction",
            ],
            &[(PASSWORD_ENV, password)],
            self.timeouts.network,
        )?;
        Session::from_output(out.stdout)
    }

    /// 锁定：让 session key 失效。**幂等** —— 本来就没登录也当成功。
    pub fn lock(&self) -> Result<(), BwError> {
        self.idempotent(&["lock"])
    }

    /// 登出：连同登录态一起清掉。**幂等**。
    pub fn logout(&self) -> Result<(), BwError> {
        self.idempotent(&["logout"])
    }

    /// 把本地状态与上游对齐（纯 pull）。
    pub fn sync(&self, session: &mut Session) -> Result<(), BwError> {
        let exposed = session.expose()?;
        let key = std::str::from_utf8(&exposed).map_err(|_| BwError::Parse {
            what: "session key".to_owned(),
            message: "不是 UTF-8".to_owned(),
        })?;
        self.run_checked(&["sync"], &[(SESSION_ENV, key)], self.timeouts.network)?;
        Ok(())
    }

    /// "本来就没登录"也是成功（把 `NotLoggedIn` 咽掉）。
    fn idempotent(&self, args: &[&str]) -> Result<(), BwError> {
        match self.run_checked(args, &[], self.timeouts.network) {
            Ok(_) | Err(BwError::NotLoggedIn) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// 跑一条必须成功的命令；非 0 退出按输出分类成 [`BwError`]。
    fn run_checked(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<Output, BwError> {
        let output = self.run(args, env, timeout)?;
        if output.code == 0 {
            return Ok(output);
        }
        Err(classify(&output))
    }

    /// 跑一次。**非 0 退出也是 `Ok`** —— 只有需要按输出分类的调用方走这条。
    fn run(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<Output, BwError> {
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &self.appdata {
            command.env("BITWARDENCLI_APPDATA_DIR", dir);
        }
        if let Some(ca) = &self.extra_ca {
            command.env("NODE_EXTRA_CA_CERTS", ca);
        }
        for (key, value) in env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|err| BwError::NotRunnable {
            path: self.program.display().to_string(),
            reason: err.to_string(),
        })?;

        // 两条读线程：不读的话，输出超过管道缓冲（`bw --help` 就有几 KB）子进程会阻塞。
        let stdout = child.stdout.take().ok_or_else(|| BwError::NotRunnable {
            path: self.program.display().to_string(),
            reason: "拿不到 stdout 管道".to_owned(),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| BwError::NotRunnable {
            path: self.program.display().to_string(),
            reason: "拿不到 stderr 管道".to_owned(),
        })?;
        let out_reader = std::thread::spawn(move || read_all(stdout));
        let err_reader = std::thread::spawn(move || read_all(stderr));

        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if started.elapsed() > timeout {
                        // 先杀，再收：`kill` 之后不 `wait` 会留下僵尸（`AGENTS.md` §3.3）。
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = out_reader.join();
                        let _ = err_reader.join();
                        return Err(BwError::Timeout {
                            seconds: timeout.as_secs(),
                        });
                    }
                    std::thread::sleep(POLL);
                }
                Err(err) => {
                    return Err(BwError::NotRunnable {
                        path: self.program.display().to_string(),
                        reason: err.to_string(),
                    });
                }
            }
        };

        let stdout = out_reader.join().unwrap_or_default();
        let stderr = err_reader.join().unwrap_or_default();
        Ok(Output {
            // 被信号杀掉时没有退出码。**不编一个数字**：`-1` 是"没有退出码"的记号，
            // 而 `code >= 0` 的判据（`run_checked`）对它是"非 0"。
            code: status.code().unwrap_or(-1),
            stdout,
            stderr,
        })
    }
}

fn read_all(mut reader: impl std::io::Read) -> Vec<u8> {
    let mut buffer = Vec::new();
    let _ = reader.read_to_end(&mut buffer);
    buffer
}

/// 按**已知信号**把一次失败的输出分类（`docs/bitwarden.md` §7.2 是实测原文）。
///
/// 认不出来的走 [`BwError::CommandFailed`]，并把原文带上 —— 猜类别会把用户指向错的地方。
fn classify(output: &Output) -> BwError {
    // ⚠️ 实测：`bw` 把错误写在 **stdout**（不是 stderr）。两边都看，stdout 优先。
    let stdout = output.stdout_text();
    let stderr = output.stderr_text();
    let text = if stdout.trim().is_empty() {
        stderr
    } else {
        stdout
    };
    let first = || first_line(&text);

    if text.contains("You are not logged in.") {
        return BwError::NotLoggedIn;
    }
    if text.contains("Insecure URL not allowed") {
        return BwError::InsecureUrl { message: first() };
    }
    if text.contains("self-signed certificate")
        || text.contains("UNABLE_TO_VERIFY")
        || text.contains("CERT_")
    {
        return BwError::Tls { message: first() };
    }
    if text.contains("FetchError") || text.contains("ETIMEDOUT") || text.contains("Unable to fetch")
    {
        return BwError::Network { message: first() };
    }
    BwError::CommandFailed {
        code: output.code,
        message: first(),
    }
}

/// 第一行非空文本，截到 200 字节 —— 错误消息要能进日志与界面，不能是整段堆栈。
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("（没有输出）");
    let mut end = line.len().min(200);
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    if end == line.len() {
        line.to_owned()
    } else {
        format!("{}…", &line[..end])
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    fn output(code: i32, stdout: &str) -> Output {
        Output {
            code,
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    /// 实测原文（`cli-v2026.8.0`）：未登录时查询类命令退出码 1、话写在 stdout。
    #[test]
    fn not_logged_in_is_its_own_class() {
        let err = classify(&output(1, "You are not logged in."));
        assert!(matches!(err, BwError::NotLoggedIn), "{err:?}");
    }

    #[test]
    fn the_insecure_url_refusal_is_recognised() {
        let err = classify(&output(
            1,
            "InsecureUrlNotAllowedError: Insecure URL not allowed. All URLs must use HTTPS.",
        ));
        assert!(matches!(err, BwError::InsecureUrl { .. }), "{err:?}");
    }

    #[test]
    fn a_self_signed_certificate_is_a_trust_problem_not_a_dead_network() {
        let err = classify(&output(
            1,
            "FetchError: request failed, reason: self-signed certificate",
        ));
        assert!(matches!(err, BwError::Tls { .. }), "{err:?}");
    }

    #[test]
    fn a_network_failure_keeps_the_first_line() {
        let err = classify(&output(
            1,
            "Unable to fetch ServerConfig from https://api.bitwarden.com FetchError: request to https://api.bitwarden.com/config failed, reason: \n    at ClientRequest.<anonymous> (/snapshot/clients/node_modules/node-fetch/lib/index.js:1501:11)\n  errno: 'ETIMEDOUT'",
        ));
        match err {
            BwError::Network { message } => {
                assert!(
                    message.contains("Unable to fetch ServerConfig"),
                    "{message}"
                );
                assert!(
                    !message.contains("at ClientRequest"),
                    "堆栈不该进消息：{message}"
                );
            }
            other => panic!("应认成网络失败：{other:?}"),
        }
    }

    /// 诱饵：认不出来的失败**原样**交给用户，不许被归进上面任何一档。
    #[test]
    fn an_unrecognised_failure_keeps_its_own_words() {
        let err = classify(&output(2, "Vault is corrupted, please re-import"));
        match err {
            BwError::CommandFailed { code, message } => {
                assert_eq!(code, 2);
                assert_eq!(message, "Vault is corrupted, please re-import");
            }
            other => panic!("不该猜类别：{other:?}"),
        }
    }

    #[test]
    fn a_killed_process_has_no_exit_code_and_is_not_zero() {
        let err = classify(&output(-1, ""));
        match err {
            BwError::CommandFailed { code, message } => {
                assert_eq!(code, -1);
                assert_eq!(message, "（没有输出）");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_long_message_is_cut_but_still_says_something() {
        let long = "x".repeat(500);
        let message = first_line(&long);
        assert!(message.ends_with('…'));
        assert!(message.len() < 210, "{}", message.len());
    }

    #[test]
    fn the_http_prefix_check_ignores_case_and_survives_multibyte() {
        // 这条只测我们对**字符串**的判断，不起进程。
        let insecure = |url: &str| {
            url.get(..7)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        };
        assert!(insecure("http://vault.example.com"));
        assert!(insecure("HTTP://vault.example.com"));
        assert!(!insecure("https://vault.example.com"));
        assert!(!insecure("vault.example.com"));
        assert!(!insecure("中文地址"), "多字节前缀不能 panic");
    }
}
