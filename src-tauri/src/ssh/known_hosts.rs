//! **known_hosts 校验与缓存**（ADR-0003 **D11**）：三态判定，两个注入点。
//!
//! D11 已经把策略定死了，这里只实现它：
//!
//! | 情形 | 行为 |
//! |---|---|
//! | 我们的库里有这台主机的**同类型**密钥，且一致 | 连 |
//! | 有记录、**对不上** | 拒绝并提示（两个指纹都带上）—— 不静默接受、不静默改写 |
//! | 库里没有 → 用户的 `~/.ssh/known_hosts` 里有且一致 | 连（那份文件**只读**，永不写） |
//! | 用户文件里有、对不上 | 拒绝（指出行号） |
//! | 两边都没有（**未知**） | 问用户；没人可问 → 明确报错 |
//! | 用户确认了 | 记进**我们的库**，下次不再问 |
//!
//! ## 为什么是两个注入点，而不是让这一层自己去开库
//!
//! 与 0502 的 [`CredentialProvider`](crate::ssh::CredentialProvider) 同一个套路：
//!
//! - [`HostKeyCache`]：库内记录的读写。这一层**不持有库连接** —— 连接归 app 的解锁窗口
//!   （plan 0407），而 verifier 要活到连接结束（`'static`），两者借不到一起。
//! - [`HostKeyPrompt`]：未知密钥问用户。现在由测试桩实现，plan 0504 接到前端。
//!
//! 两个都是**同步** trait —— 与凭据那条路一样（`russh` 的回调是 async，但提问这件事
//! 天生是阻塞的）。**副作用照实记**：提问期间会占住 runtime 的一个 worker，
//! 所以 plan 0504 得给这条路上的连接留出余量（专用线程或 `spawn_blocking`）。
//!
//! ## 判定材料是**密钥本体**，不是指纹文本
//!
//! 逐字节比 `key_blob`（SSH 线格式）。指纹是 `SHA256:` 的文本表示，给人核对用的；
//! 拿它当判据是把"同一把密钥"押在一段有损的表示上。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use russh::keys::{PublicKey, check_known_hosts_path};

use crate::ssh::error::SshError;
use crate::ssh::handshake::{HostKey, HostKeyVerifier};
use crate::ssh::target::SshTarget;

/// 库里记着的一把主机密钥。
///
/// 形状刻意与 `crate::store::known_hosts::KnownHost` 对齐（它就是那个类型的适配目标）：
/// `blob` 是**判定材料**，`fingerprint` 只用来把"记录的是哪一把"说给用户听。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedHostKey {
    /// SSH 线格式的密钥本体。
    pub blob: Vec<u8>,
    /// `SHA256:…`，给人核对用。
    pub fingerprint: String,
}

/// **记录在哪** —— 拒绝的时候要说清这一点：库里的记录是"你在这个程序里确认过的"，
/// 用户文件里的记录是"你在别处维护的"，两种情况下用户该做的事不一样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedIn {
    /// 我们的库（用户在 akasha 里确认过）。
    Vault,
    /// 用户的 `~/.ssh/known_hosts`。
    UserFile {
        path: PathBuf,
        /// 那一行的行号（1 起）。**找不到就是 `None`** —— 见 [`true_line_of`]：
        /// 宁可只说文件名，也不报一个错的数字。
        line: Option<usize>,
    },
}

impl std::fmt::Display for RecordedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vault => write!(f, "akasha 的库"),
            Self::UserFile {
                path,
                line: Some(line),
            } => write!(f, "{} 第 {line} 行", path.display()),
            Self::UserFile { path, line: None } => write!(f, "{}", path.display()),
        }
    }
}

/// 库内记录的读写口。由调用方实现（测试用内存桩，plan 0504 接到解锁的库上）。
pub trait HostKeyCache: Send + Sync {
    /// 这台主机上**这个类型**的密钥记的是什么。没有 → `Ok(None)` = 未知。
    fn recorded(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
    ) -> Result<Option<RecordedHostKey>, SshError>;

    /// 记下一把**用户刚确认过**的密钥。
    ///
    /// 实现方必须把"已经记着别的密钥"这件事变成**拒绝**（库那一层就是这么写的，
    /// 见 `crate::store::known_hosts::remember`）—— 静默改写等于把中间人攻击变成默认行为。
    fn remember(&self, host: &str, port: u16, key: &HostKey) -> Result<(), SshError>;
}

/// 未知密钥的提问口。`Ok(false)` = 用户否认，`Err(_)` = 问不出去（前端走了 / 超时）。
pub trait HostKeyPrompt: Send + Sync {
    fn confirm(&self, target: &SshTarget, key: &HostKey) -> Result<bool, SshError>;
}

/// 按 D11 判定的 [`HostKeyVerifier`]。
pub struct KnownHostsVerifier {
    cache: Arc<dyn HostKeyCache>,
    /// 用户的 `~/.ssh/known_hosts`（**只读**）。`None` = 不查它。
    ///
    /// 默认值是 [`user_known_hosts_file`]；测试与无家目录的场景用
    /// [`KnownHostsVerifier::without_user_file`] 关掉它。
    file: Option<PathBuf>,
    /// 未知密钥问谁。`None` = 没人可问 → [`SshError::HostKeyUnknown`]。
    prompt: Option<Arc<dyn HostKeyPrompt>>,
}

impl KnownHostsVerifier {
    /// 建一个：库内缓存必备，用户文件默认读，**没有人可问**。
    ///
    /// 默认不带提问是刻意的：忘了配的后果是**拒绝**（带指纹的明确错误），
    /// 而不是默默接受一把没见过的密钥。
    pub fn new(cache: Arc<dyn HostKeyCache>) -> Self {
        Self {
            cache,
            file: user_known_hosts_file(),
            prompt: None,
        }
    }

    /// 指定用户的 `~/.ssh/known_hosts`（测试与"用户的文件在别处"的场景）。
    pub fn with_user_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.file = Some(path.into());
        self
    }

    /// 不查用户的文件（只看我们自己的库）。
    pub fn without_user_file(mut self) -> Self {
        self.file = None;
        self
    }

    /// 配上一个提问口（用户在未知密钥面前会被问一次）。
    pub fn with_prompt(mut self, prompt: Arc<dyn HostKeyPrompt>) -> Self {
        self.prompt = Some(prompt);
        self
    }
}

impl HostKeyVerifier for KnownHostsVerifier {
    fn verify(&self, target: &SshTarget, key: &HostKey) -> Result<(), SshError> {
        // ① 我们的库优先：它装的是**用户在这个程序里确认过**的记录，
        //    比用户在别处维护的文件更新。
        if let Some(recorded) =
            self.cache
                .recorded(target.host(), target.port(), key.algorithm())?
        {
            if recorded.blob == key.blob() {
                return Ok(());
            }
            return Err(SshError::HostKeyChanged {
                host: target.host().to_owned(),
                port: target.port(),
                recorded: recorded.fingerprint,
                presented: key.fingerprint().to_owned(),
                recorded_in: RecordedIn::Vault,
            });
        }

        // ② 用户的文件（**只读**）。上游的判别与我们自己那条同构：
        //    `Ok(true)` = 同一类型的记录对上了；`KeyChanged` = 同类型但对不上；
        //    `Ok(false)` = 没有这一类型的记录（= 未知，包括"只有别的类型"）。
        //    用上游那一个而不是自己解析文件：它认得哈希主机名（`|1|…`）这类形态。
        if let Some(path) = &self.file {
            match check_known_hosts_path(target.host(), target.port(), key.public_key(), path) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(russh::keys::Error::KeyChanged { .. }) => {
                    // 记录里那一把（拿它的指纹说"记的是哪一把"、拿它的密钥本体定位行号）。
                    // 用户文件里那一份读不出来时，仍然拒绝，只是少说两件事。
                    let recorded = recorded_key_in_file(path, target, key);
                    return Err(SshError::HostKeyChanged {
                        host: target.host().to_owned(),
                        port: target.port(),
                        recorded: recorded
                            .as_ref()
                            .map(|(recorded, _)| {
                                recorded
                                    .fingerprint(russh::keys::HashAlg::Sha256)
                                    .to_string()
                            })
                            .unwrap_or_else(|| "<无法读出的记录>".to_owned()),
                        presented: key.fingerprint().to_owned(),
                        recorded_in: RecordedIn::UserFile {
                            path: path.clone(),
                            line: recorded.and_then(|(_, line)| line),
                        },
                    });
                }
                Err(err) => {
                    // 某个**读得动但认不出**的行（文件里有一行垃圾，或上游不认识的标记）：
                    // 文件没有回答我们的问题，于是当作**未知**继续往下走 —— 未知的后果是
                    // 问用户，不是静默接受。反过来（在这里直接拒绝）会让用户文件里任意一行
                    // 奇怪的记录变成"这台机器你连不上"，而那不是他的本意。
                    tracing::warn!(
                        path = %path.display(),
                        %err,
                        "user known_hosts line unreadable"
                    );
                }
            }
        }

        // ③ 未知：**绝不默认接受**（D11）。有人可问就问，问了才算数。
        if let Some(prompt) = &self.prompt {
            if prompt.confirm(target, key)? {
                self.cache.remember(target.host(), target.port(), key)?;
                return Ok(());
            }
            return Err(SshError::HostKeyRejected {
                fingerprint: key.fingerprint().to_owned(),
            });
        }

        Err(SshError::HostKeyUnknown {
            host: target.host().to_owned(),
            port: target.port(),
            fingerprint: key.fingerprint().to_owned(),
        })
    }
}

/// 用户的 `~/.ssh/known_hosts`（ADR-0003 D11 要读的那份）。
///
/// 取不到家目录 → `None`：那时只按我们自己的库判，而不是拿一个猜出来的路径去读。
/// 这里不用 `std::env::home_dir()`：它在旧版本上被弃用过，行为也随时间变过；
/// 我们只需要两个环境变量（Windows 上是 `USERPROFILE`）。
pub fn user_known_hosts_file() -> Option<PathBuf> {
    ssh_file("known_hosts")
}

/// 用户的 `~/.ssh/config`（plan 0506 导入的**默认**输入）。
///
/// 与 [`user_known_hosts_file`] 共用"`~/.ssh` 在哪"这一条推导（`HOME` / `USERPROFILE`）：
/// 两处各写一份，迟早会出现"一个认 `HOME`、另一个只认 `USERPROFILE`"。
/// 导入的调用方**可以**给别的路径 —— 这个函数只回答"不给路径时读哪个"。
pub fn user_ssh_config_file() -> Option<PathBuf> {
    ssh_file("config")
}

/// `~/.ssh/<name>`。取不到家目录 → `None`（**不猜**一个路径去读别人的东西）。
fn ssh_file(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".ssh").join(name))
}

/// 用户文件里那把**同类型**的密钥，以及它在文件里的**真实**行号。
///
/// 只在"对不上"那条路上用一次：用户需要同时看到两个指纹（旧的是哪个、现在来的是哪个）
/// 才做得了判断。读不出来不影响拒绝本身。
fn recorded_key_in_file(
    path: &Path,
    target: &SshTarget,
    presented: &HostKey,
) -> Option<(PublicKey, Option<usize>)> {
    let recorded =
        russh::keys::known_hosts::known_host_keys_path(target.host(), target.port(), path).ok()?;
    let (_, recorded) = recorded
        .into_iter()
        .find(|(_, key)| key.algorithm() == presented.public_key().algorithm())?;
    let line = true_line_of(path, &recorded);
    Some((recorded, line))
}

/// **这一把密钥**在文件里的真实行号（1 起）。
///
/// 为什么不照抄上游 `Error::KeyChanged` 给的那个行号：它跳过注释行时**不递增计数**
/// （`russh-0.63.3/src/keys/known_hosts.rs` 的 `continue` 在 `line += 1` 之前），
/// 于是给出的是"非注释行的序号" —— 而那是一个**错的**行号，写进错误消息就是把用户
/// 指到别处。宁可自己数一遍。
///
/// 匹配用的是第三列（密钥本体的 base64 文本）：主机名那一列可能是 `|1|…` 哈希形态，
/// 而密钥本体足够唯一。找不到就 `None`（那时只说文件名，不猜行号）。
fn true_line_of(path: &Path, recorded: &PublicKey) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    let openssh = recorded.to_openssh().ok()?;
    let needle = openssh.split_whitespace().nth(1)?;
    text.lines().enumerate().find_map(|(index, line)| {
        let mut fields = line.split_whitespace();
        // 三列：主机、算法、base64。注释行与空行只有一到两列，自然会跳过。
        let _hosts = fields.next()?;
        let _algorithm = fields.next()?;
        (fields.next()? == needle).then_some(index + 1)
    })
}
