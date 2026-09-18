//! 凭据，以及"同一台主机不要问两次"的那张表（ADR-0003 D8）。
//!
//! 这里只有**一句话的秘密**：登录口令，或者打开某把私钥要的那句口令。
//! **不存私钥** —— 私钥本体继续只住在密钥池那一页里（`store`），
//! 缓存里多一份长期私钥是最贵的那种副本，D8 专门把它排除了。
//!
//! 三条性质是这个模块存在的全部意义，每一条都由类型或结构保证，而不是靠约定：
//!
//! | 性质 | 由什么保证 |
//! |---|---|
//! | 空口令不可表示 | [`Credential::new`] 是唯一构造口，空值被拒（同 `Passphrase`） |
//! | 打进日志不可能 | 没有 `Debug` / `Display`（`{:?}` 是编译错误，不是打码） |
//! | 长住内存里受保护 | 本体是 `store` 那一页受保护内存（`mlock` + 静止态 `PROT_NONE`） |
//!
//! ⚠️ **照实记下的边界**（ADR-0003 D8 的另一半）：
//!
//! * `russh` 的认证接口要 `impl Into<String>`，所以送口令那一刻**必然**存在一份
//!   普通堆上的 `String`（[`Credential::expose_string`]）。它只能"用后尽快 drop"，
//!   我们擦不掉它 —— 上游没有接受字节缓冲的入口。
//! * 命中缓存时**不做并发去重**：两条连接同时未命中会各问一次。判据
//!   （"同主机三个 Session 只问一次"）说的是**顺序**开三个，那条是成立的；
//!   并发那条路要的是 single-flight，等真有并发开多个会话的场景再加（记在 plan 0502）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::store::protected::{Exposed, Protected};

use crate::ssh::error::SshError;
use crate::ssh::target::SshTarget;

/// 一句凭据的字节上限 —— 也就是受保护页的大小。
///
/// 与 `crate::store::Passphrase` 的 `MAX_LEN` 取同一个值（256）是**有意的**：
/// 同一个用户、同一类东西（一句话的口令），没有理由在两个地方给出不同的上限。
/// 超了是明确报错，不是截断。
pub const MAX_CREDENTIAL_LEN: usize = 256;

/// 一句话的凭据。**空值不可表示**，**打不出来**。
pub struct Credential {
    page: Protected<MAX_CREDENTIAL_LEN>,
}

impl Credential {
    /// 从字节造一个凭据。成功时源缓冲已被擦零（受保护页那一侧做的）。
    pub fn new(bytes: Vec<u8>) -> Result<Self, SshError> {
        if bytes.is_empty() {
            return Err(SshError::EmptyCredential);
        }
        Ok(Self {
            page: Protected::new(bytes)?,
        })
    }

    /// 实际字节数（不是页大小）。
    pub fn byte_len(&self) -> usize {
        self.page.byte_len()
    }

    /// 临时取得字节。读它是**一次需要 `&mut` 的提权动作**（同 `Passphrase`）。
    ///
    /// `pub(crate)`：凭据能流向哪里，在这个 crate 里数得出来 —— 只有认证路径。
    /// 这一条比 `Passphrase` 松不了：那边也是 `pub(crate)`。
    pub(crate) fn expose(&mut self) -> Result<Exposed<'_, MAX_CREDENTIAL_LEN>, SshError> {
        Ok(self.page.expose()?)
    }

    /// 借出 `&str`（私钥口令那条路：`decode_secret_key` 收 `&str`）。**不留副本。**
    pub(crate) fn with_str<R>(&mut self, f: impl FnOnce(&str) -> R) -> Result<R, SshError> {
        let guard = self.expose()?;
        let text = std::str::from_utf8(&guard).map_err(|_| SshError::CredentialNotUtf8)?;
        Ok(f(text))
    }

    /// 交出**一份 `String` 副本**（登录口令那条路：`russh` 只收 `Into<String>`）。
    ///
    /// ⚠️ 这一份在普通堆上，用后即 drop，但**不擦**：上游没有接受字节缓冲的入口。
    /// 这不是懒，是无处可施 —— 记为 D8 的边界而不是假装它不存在。
    pub(crate) fn expose_string(&mut self) -> Result<String, SshError> {
        let guard = self.expose()?;
        let text = std::str::from_utf8(&guard)
            .map_err(|_| SshError::CredentialNotUtf8)?
            .to_owned();
        Ok(text)
    }
}

/// **要问的是哪一种凭据。** 它是缓存键的一部分，因为同一个目标上它们是不同的秘密：
/// 登录口令与私钥口令可以完全不同。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CredentialKind {
    /// 登录口令（`password` 认证）。
    LoginPassword,
    /// keyboard-interactive（2FA / OTP）。同一个目标上它通常就是验证码，
    /// **与登录口令分开缓存**：混在一起会拿验证码当口令，或者反过来。
    KeyboardInteractive,
    /// 打开私钥要的口令。
    ///
    /// ⚠️ 键里必须带**哪把钥匙**：同一台主机上两把钥匙各有各的口令，
    /// 只按主机缓存会拿错口令去解另一把钥匙（症状是"解密失败"，而真实原因在缓存里）。
    /// `key` 是调用方给的稳定标识（密钥池的行 id 一类）。**换钥匙就要换标识** ——
    /// 密钥材料变了而标识没变，缓存里那句口令就是过期的。
    KeyPassphrase {
        /// 私钥的稳定标识（密钥池行 id 一类）。
        key: String,
    },
}

impl CredentialKind {
    /// 日志用的稳定短名（`docs/logging.md`：消息是常量，细节进字段）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LoginPassword => "password",
            Self::KeyboardInteractive => "keyboard-interactive",
            Self::KeyPassphrase { .. } => "key-passphrase",
        }
    }
}

/// 缓存键：`(host, port, user, 认证方式)`（D8）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    target: SshTarget,
    kind: CredentialKind,
}

impl CacheKey {
    /// 构造键。
    pub fn new(target: SshTarget, kind: CredentialKind) -> Self {
        Self { target, kind }
    }

    /// 目标。
    pub fn target(&self) -> &SshTarget {
        &self.target
    }

    /// 认证方式。
    pub fn kind(&self) -> &CredentialKind {
        &self.kind
    }
}

/// 问用户要一次凭据。
///
/// app 侧的实现会把它接到 IPC / 提示界面；测试里接一个计数的桩
/// （判据"只问一次"就是数它被调了几次）。
///
/// ⚠️ 它是**同步**的，因为对外的整条路都是同步门面（D3）。app 那边实现时要用
/// 一条有期限的等待，**不能**无限期挂住调用线程。
pub trait CredentialProvider: Send + Sync {
    /// 请求一次凭据。返回 `Err` 表示用户取消或那条路自己坏了。
    fn request(&self, request: &CredentialRequest) -> Result<Credential, SshError>;
}

/// 一次凭据请求的描述。**给提示界面看的东西都在这里**，没有别的渠道。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRequest {
    /// 哪个目标。
    pub target: SshTarget,
    /// 要哪一种。
    pub kind: CredentialKind,
    /// 直接可以显示给用户的一句话。
    pub prompt: String,
}

impl CredentialRequest {
    /// 按目标与方式拼出提示语。
    pub fn new(target: &SshTarget, kind: CredentialKind) -> Self {
        let prompt = match &kind {
            CredentialKind::LoginPassword => format!("{target} 的登录口令"),
            CredentialKind::KeyboardInteractive => format!("{target} 的验证码"),
            CredentialKind::KeyPassphrase { .. } => format!("{target} 的私钥口令"),
        };
        Self {
            target: target.clone(),
            kind,
            prompt,
        }
    }
}

/// **内存凭据缓存**（D8）。进程内的表，**绝不落盘**。
///
/// 值放在 `Arc<Mutex<..>>` 里而不是"取出来一份拷贝"：受保护页不可 `Clone`，
/// 而"每个用途各持一份口令"正是我们要避免的 —— 于是命中时交出的是**同一个**凭据，
/// 同一个时刻只有一份明文页是热的。
pub struct CredentialCache {
    entries: Mutex<HashMap<CacheKey, Arc<Mutex<Credential>>>>,
}

impl CredentialCache {
    /// 建一张空表。
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 取表。
    ///
    /// 中毒（有线程在持锁时 panic）时**继续用**而不是放弃：这是一张缓存，
    /// 里面的东西随时可以重问，让一次 panic 把后续全部连接拖死没有道理。
    fn entries(&self) -> MutexGuard<'_, HashMap<CacheKey, Arc<Mutex<Credential>>>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 查一条。命中就返回**同一个**凭据（不是拷贝）。
    pub fn get(&self, key: &CacheKey) -> Option<Arc<Mutex<Credential>>> {
        self.entries().get(key).map(Arc::clone)
    }

    /// 存一条，返回它（`resolve` 要把它交给调用方）。
    pub fn insert(&self, key: CacheKey, credential: Credential) -> Arc<Mutex<Credential>> {
        let shared = Arc::new(Mutex::new(credential));
        self.entries().insert(key, Arc::clone(&shared));
        shared
    }

    /// 忘掉一条。**失效条件 ②**（认证被服务端拒绝）与 **③**（用户显式忘记）都走它。
    ///
    /// 返回"本来有没有"：没有命中也是正常结果（比如两个连接同时被拒），
    /// 所以调用方不该把它当失败。
    pub fn forget(&self, key: &CacheKey) -> bool {
        self.entries().remove(key).is_some()
    }

    /// 清空。**失效条件 ①**：库锁定、进程退出前的显式清理。
    ///
    /// ⚠️ 只清"表里的那份引用"：调用方手上若还握着某个 `Arc`，那一页到它 drop
    /// 才真正释放（`VmLck` 也就那时才回落）。这条是 `Arc` 的语义，不是 bug，
    /// 但验"清空之后 `VmLck` 回落"时必须把它算进去。
    pub fn clear(&self) {
        self.entries().clear();
    }

    /// 现在缓存着几条。
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// 一条都没有。
    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// **命中就复用，没命中就问一次并记下来** —— 这是判据的落点。
    ///
    /// 调用方在"服务端拒绝了这个凭据"时必须显式 [`Self::forget`]：缓存不知道自己
    /// 被拒绝了，而拿错口令反复重试正是账号锁定的经典成因（ADR-0003 D13）。
    pub fn resolve(
        &self,
        key: CacheKey,
        provider: &dyn CredentialProvider,
    ) -> Result<Arc<Mutex<Credential>>, SshError> {
        if let Some(existing) = self.get(&key) {
            return Ok(existing);
        }
        let request = CredentialRequest::new(key.target(), key.kind().clone());
        let credential = provider.request(&request)?;
        Ok(self.insert(key, credential))
    }
}

impl Default for CredentialCache {
    fn default() -> Self {
        Self::new()
    }
}
