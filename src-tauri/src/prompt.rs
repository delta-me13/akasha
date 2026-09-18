//! **「后端问 → 前端答」的往返**（plan 0504）—— 凭据与"没见过的主机密钥"共用这一条路。
//!
//! ## 为什么它必须是一条往返，而不是两次独立调用
//!
//! 提问发生在**连接中途**：`russh` 在认证或握手时回头看我们
//! （[`CredentialProvider`] / [`HostKeyPrompt`]，两者都是**同步** trait），
//! 而那些回调跑在 SSH 的 task 上。前端必须能在**同一时刻**把答案送回来 —— 于是：
//!
//! ```text
//! 连接线程                                    前端
//!   ask()  ── 事件 ssh_prompt ──────────────▶ 提示界面
//!   recv_timeout(120s) ◀── 命令 ssh_prompt_* ── 用户作答
//! ```
//!
//! ## 三条硬性质
//!
//! 1. **超时 = 拒绝**（120 s）。不是接受、也不是"把连接永远挂在那里"：
//!    提问期间连接线程停在一次同步回调里，没有人回答就永远不返回。
//! 2. **答案不落盘、不进日志**。口令经 [`PassphraseInput`] 进来（**没有 `Debug`/`Clone`**），
//!    立刻变成 `ssh` 的 `Credential`（受保护页）。
//! 3. **答错类型一律当"没答"**：主机密钥那一问只认"接受/拒绝"，收到别的（或没收到）
//!    → `HostKeyUnknown`（**拒绝连接**），绝不把一次错答读成"用户接受了"。
//!
//! ## 可测性
//!
//! 发布口是可注入的（[`Prompts::publish_with`]）：真的 app 里它 `emit` 事件，
//! 单测里它只是一个记录 + 自动作答的闭包 —— 于是"超时 / 取消 / 答错类型"这些分支
//! **不需要 app** 就能验（`Sessions::on_change` 与 `SessionWatchdog::with_writer` 同一个套路）。
//!
//! ## 边界（照实记）
//!
//! 提问是**同步阻塞**的（trait 就是同步的），所以它会占住一次 SSH 回调所在的 runtime
//! worker。对策是给 app 的 SSH runtime 留余量（`ssh.rs` 的 worker 数）与上面那个超时 ——
//! 两者缺一，一条没人回答的连接就能把 worker 一直攥着。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use crate::ssh::{
    Credential, CredentialKind, CredentialProvider, CredentialRequest, HostKey, HostKeyPrompt,
    SshError, SshTarget,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_specta::Event;

use crate::store::ipc::vault::PassphraseInput;

/// 一次提问等多久。**超时就拒绝**（见模块文档的性质 1）。
///
/// 120 s 的取值：2FA 用户要去手机上看一眼验证码、口令要回想一下，够用；
/// 而它同时是"一个 runtime worker 最多被占多久"的上界 —— 再长，一条没人理的连接
/// 就能把 SSH runtime 攥住很久（那条 runtime 只有几个 worker）。
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// 一次提问的编号。前端原样带回来。
///
/// 从 1 开始：`0` 在调试输出里太容易与"空值"混为一谈（生成成 TS 就是 `number`）。
pub type PromptId = u32;

/// **要哪一种凭据** —— 过 IPC 的形态（`ssh` 的 `CredentialKind` 没有 `specta`）。
///
/// 前端用它决定提示语的语气（验证码 / 口令 / 私钥口令），**不决定行为**：
/// 三种都是"一句秘密"，答案的走法完全相同。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    LoginPassword,
    KeyboardInteractive,
    KeyPassphrase,
}

impl From<&CredentialKind> for SecretKind {
    /// 穷尽 `match`：`ssh` 加一种凭据，**这里编译不过** —— 而不是悄悄落进
    /// 某个兜底分支，让前端在一个它不认识的值上做默认动作。
    fn from(kind: &CredentialKind) -> Self {
        match kind {
            CredentialKind::LoginPassword => Self::LoginPassword,
            CredentialKind::KeyboardInteractive => Self::KeyboardInteractive,
            CredentialKind::KeyPassphrase { .. } => Self::KeyPassphrase,
        }
    }
}

/// 后端要问用户一件事。前端据此打开提示界面。
///
/// 两个变体放在同一个事件里，是因为它们**在同一条路上**发生（连接中途、同一个往返机制、
/// 同一个超时）—— 分成两个事件只会让"谁在问"多一份要维护的契约。
/// ⚠️ 但**策略**完全不同：主机密钥那一问只有"接受 / 拒绝"两个答案（见 `ssh.rs` 的接线），
/// 凭据那一问是一句秘密。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PromptRequest {
    /// 一把没见过的服务端主机密钥，请用户核对指纹（ADR-0003 D11）。
    #[serde(rename_all = "camelCase")]
    HostKey {
        id: PromptId,
        host: String,
        port: u16,
        /// 密钥算法（`ssh-ed25519` 一类）—— 与指纹一起显示，用户核对的是指纹。
        algorithm: String,
        /// `SHA256:…`，**要用户拿去核对的那串东西**。
        fingerprint: String,
    },
    /// 要一句秘密。
    #[serde(rename_all = "camelCase")]
    Credential {
        id: PromptId,
        /// `user@host:port`（`SshTarget` 的 `Display`）—— 用户得知道在给谁输口令。
        target: String,
        credential: SecretKind,
        /// 直接可以显示给用户的一句话（`CredentialRequest::prompt`）。
        prompt: String,
    },
}

impl PromptRequest {
    /// 这一次提问的编号。
    pub fn id(&self) -> PromptId {
        match self {
            Self::HostKey { id, .. } | Self::Credential { id, .. } => *id,
        }
    }

    /// 把编号填进去（构造时用 `0` 占位，编号由 [`Prompts`] 分配）。
    fn with_id(self, id: PromptId) -> Self {
        match self {
            Self::HostKey {
                host,
                port,
                algorithm,
                fingerprint,
                ..
            } => Self::HostKey {
                id,
                host,
                port,
                algorithm,
                fingerprint,
            },
            Self::Credential {
                target,
                credential,
                prompt,
                ..
            } => Self::Credential {
                id,
                target,
                credential,
                prompt,
            },
        }
    }
}

// 手写 `Event`（理由与 `SessionEnded` 同：为一个"一个常量 + 其余全默认方法"的 trait
// 多引一个 proc-macro 依赖不划算）。
impl tauri_specta::Event for PromptRequest {
    const NAME: &'static str = "ssh_prompt";
}

/// 这次提问**不用答了**（超时被撤下）。
///
/// 为什么要有它：界面上一份提示不能永远留着 —— 没有人答的那一问在
/// [`PROMPT_TIMEOUT`] 之后会被撤掉，前端必须**看得见**这件事，否则它会一直显示一个
/// 早已不存在的提问（用户再点"确定"只会得到 `Gone`）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptDismissed {
    pub id: PromptId,
}

impl tauri_specta::Event for PromptDismissed {
    const NAME: &'static str = "ssh_prompt_dismissed";
}

/// 用户对一问的回答。
enum Answer {
    /// 一句秘密（口令 / 验证码 / 私钥口令）。
    Secret(Credential),
    /// 主机密钥那一问的"接受 / 拒绝"。
    Confirm(bool),
    /// 用户撤销了这一问（关掉面板）。
    Cancel,
}

/// 提问这一步自己的失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AskError {
    /// 没人回答（超过 [`PROMPT_TIMEOUT`]）。
    Timeout,
    /// 用户撤销了（或者前端走了）。
    Cancelled,
}

impl AskError {
    /// 放进 `SshError` 的说法（`docs/logging.md`：消息是常量，细节进字段）。
    fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "超时没人回答",
            Self::Cancelled => "用户取消",
        }
    }
}

/// 发布出去的一条。`pub(crate)`：它是 [`Prompts::publish_with`] 的参数类型，
/// 而那个口只给 crate 内部用（真的 app 走 [`Prompts::to_frontend`]）。
pub(crate) enum Published {
    Asked(PromptRequest),
    Dismissed(PromptId),
}

/// `ask` 的返回值（`Secret` 那一支是受保护页，**只能**在闭包里用掉）。
type Pending = Mutex<HashMap<PromptId, Sender<Answer>>>;

#[derive(Default)]
struct Inner {
    /// 正在等的提问：编号 → 把答案送回去的那条口。
    pending: Pending,
    /// 下一个编号。**只增不减**（重复使用编号会让"答的是哪一问"变成一个没人答得上来的问题）。
    /// **从 1 开始**（见 [`Prompts::new`]）—— `0` 与"空值"太容易混。
    next: AtomicU32,
    /// 发布口。`OnceLock`：只被设置一次，读的人多（同 `Sessions::attach_watchdog` 的理由）。
    publish: OnceLock<Arc<dyn Fn(Published) + Send + Sync>>,
    /// 等的期限。默认 [`PROMPT_TIMEOUT`]；测试用它把"超时"这条分支跑到。
    timeout: OnceLock<Duration>,
}

/// 提问往返的注册表。由 tauri 作为 `State` 持有；`Clone` 出来的是**同一张表**。
#[derive(Clone)]
pub struct Prompts {
    inner: Arc<Inner>,
}

impl Default for Prompts {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                next: AtomicU32::new(1),
                ..Inner::default()
            }),
        }
    }
}

impl Prompts {
    /// 装一个没有发布口的表（**提问会超时**，因为没人看得到）。
    ///
    /// 这是刻意的默认：忘了装发布口的表现是"连不上、日志里说超时"，
    /// 而不是"悄悄拿一个默认答案继续"。
    pub fn new() -> Self {
        Self::default()
    }

    /// 装上发布口。已经装过就忽略这一次并记日志 —— 同
    /// [`Sessions::attach_watchdog`](crate::session::Sessions::attach_watchdog)：
    /// 第二个发布口意味着有第三处在悄悄接同一条路，那必须被看见。
    pub(crate) fn publish_with(&self, publish: impl Fn(Published) + Send + Sync + 'static) {
        if self.inner.publish.set(Arc::new(publish)).is_err() {
            tracing::warn!(reason = "already-installed", "prompt publisher ignored");
        }
    }

    /// 接上真的前端：`emit` 两个事件。
    ///
    /// ⚠️ 必须在 `mount_events` 之后调用（`lib.rs` 的 `.setup()`）—— 事件没注册时
    /// `emit` 会 panic（`EventRegistry not found`）。
    pub fn to_frontend(&self, app: AppHandle) {
        self.publish_with(move |published| {
            let emitted = match published {
                Published::Asked(request) => app.emit(PromptRequest::NAME, request),
                Published::Dismissed(id) => app.emit(PromptDismissed::NAME, PromptDismissed { id }),
            };
            if let Err(err) = emitted {
                // 前端可能已经走了（窗口销毁 / webview 没了）。这一条只影响"用户看不看得到
                // 提示"，而连接那一侧照样会在超时之后拒绝 —— 所以记 warn，不做别的。
                tracing::warn!(%err, "prompt emit failed");
            }
        });
    }

    /// 改期限（测试用；真的 app 用默认的 [`PROMPT_TIMEOUT`]）。
    ///
    /// 返回 `self` 的**克隆**（同一个 `Arc`）：这样它能在链式写法里当构造器用
    /// （`Prompts::new().with_timeout(…)`），而 `&self` 那版会被临时值的生命期挡住。
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        let _ = self.inner.timeout.set(timeout);
        self.clone()
    }

    fn timeout(&self) -> Duration {
        self.inner.timeout.get().copied().unwrap_or(PROMPT_TIMEOUT)
    }

    /// 取表。中毒（有线程在持锁时 panic）时**继续用**：这是一张待答清单，
    /// 让一次 panic 把后续全部连接拖死没有道理（同 `CredentialCache::entries`）。
    fn pending(&self) -> MutexGuard<'_, HashMap<PromptId, Sender<Answer>>> {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// **问一次并等答案**。这条路会**阻塞调用线程**，直到有人回答 / 用户撤销 / 超时。
    fn ask(&self, request: PromptRequest) -> Result<Answer, AskError> {
        let id = self.inner.next.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = std::sync::mpsc::channel();
        self.pending().insert(id, sender);

        // 发布口取一份出来：下面要用它两次（发布 + 判断"有没有人在听"），
        // 而持着 `OnceLock` 的引用去调用它是没必要的耦合。
        let publish = self.inner.publish.get().cloned();
        let Some(publish) = publish else {
            // 没有发布口 = 没有人看得到这一问。**立刻失败**，不留一条永远等着的线程。
            self.pending().remove(&id);
            tracing::warn!(prompt = id, "prompt has no publisher");
            return Err(AskError::Timeout);
        };
        publish(Published::Asked(request.with_id(id)));

        let outcome = receiver.recv_timeout(self.timeout());

        // 无论成败都要把这一条摘掉：留着它，前端晚到的答案会送进一条没人读的通道
        //（表现是"点确定没反应"），而那条通道还占着一个编号。
        self.pending().remove(&id);

        match outcome {
            Ok(Answer::Cancel) => Err(AskError::Cancelled),
            Ok(answer) => Ok(answer),
            Err(RecvTimeoutError::Timeout) => {
                // 撤下这一问：界面上一份提示不能永远留着（没人答的那一问已经不存在了）。
                publish(Published::Dismissed(id));
                Err(AskError::Timeout)
            }
            // 发送端没了 = 只有 `cancel` 那条路（本函数自己持着接收端）。
            Err(RecvTimeoutError::Disconnected) => Err(AskError::Cancelled),
        }
    }

    /// 送一个答案。没有这一问（超时被撤 / 已经答过）→ `false`。
    fn deliver(&self, id: PromptId, answer: Answer) -> bool {
        let sender = self.pending().remove(&id);
        match sender {
            Some(sender) => {
                // 送不出去只可能是**问的那一方已经不在了**（连接在超时之后失败了）——
                // 那不是用户错误，也不必记日志：这一问本来就已经结束。
                sender.send(answer).is_ok()
            }
            None => false,
        }
    }

    /// 回答一句秘密。空值 / 太长由 `Credential::new` 拒绝（**不截断、不顶替**）。
    pub fn answer_credential(
        &self,
        id: PromptId,
        secret: PassphraseInput,
    ) -> Result<(), PromptError> {
        let credential =
            Credential::new(secret.into_bytes()).map_err(|err| PromptError::Invalid {
                message: err.to_string(),
            })?;
        if self.deliver(id, Answer::Secret(credential)) {
            Ok(())
        } else {
            Err(PromptError::Gone { id })
        }
    }

    /// 回答主机密钥那一问。
    pub fn answer_host_key(&self, id: PromptId, accept: bool) -> Result<(), PromptError> {
        if self.deliver(id, Answer::Confirm(accept)) {
            Ok(())
        } else {
            Err(PromptError::Gone { id })
        }
    }

    /// 撤销这一问（用户关掉了面板）。
    pub fn cancel(&self, id: PromptId) -> Result<(), PromptError> {
        if self.deliver(id, Answer::Cancel) {
            Ok(())
        } else {
            Err(PromptError::Gone { id })
        }
    }

    /// 现在有几问在等答案（诊断与测试用）。
    pub fn pending_count(&self) -> usize {
        self.pending().len()
    }
}

impl CredentialProvider for Prompts {
    /// `russh` 在认证中途回头问我们要一句口令 —— 我们把它交给前端。
    fn request(&self, request: &CredentialRequest) -> Result<Credential, SshError> {
        let asked = PromptRequest::Credential {
            id: 0,
            target: request.target.to_string(),
            credential: SecretKind::from(&request.kind),
            prompt: request.prompt.clone(),
        };
        match self.ask(asked) {
            Ok(Answer::Secret(credential)) => Ok(credential),
            // 答错了类型 = 没答（**不是**"空口令"）：那是编程错误，照实说。
            Ok(_) => Err(SshError::CredentialUnavailable(
                "答案与问题不匹配".to_owned(),
            )),
            Err(err) => Err(SshError::CredentialUnavailable(err.as_str().to_owned())),
        }
    }
}

impl HostKeyPrompt for Prompts {
    /// 一把没见过的密钥：问用户。**`Ok(false)` 只表示"用户拒绝了"** ——
    /// 超时 / 撤销 / 答错类型一律是 `Err`（[`SshError::HostKeyUnknown`]，= 拒绝连接），
    /// 绝不能读成"用户接受了"。
    fn confirm(&self, target: &SshTarget, key: &HostKey) -> Result<bool, SshError> {
        let unknown = || SshError::HostKeyUnknown {
            host: target.host().to_owned(),
            port: target.port(),
            fingerprint: key.fingerprint().to_owned(),
        };
        let asked = PromptRequest::HostKey {
            id: 0,
            host: target.host().to_owned(),
            port: target.port(),
            algorithm: key.algorithm().to_owned(),
            fingerprint: key.fingerprint().to_owned(),
        };
        match self.ask(asked) {
            Ok(Answer::Confirm(accept)) => Ok(accept),
            Ok(_) => Err(unknown()),
            Err(_) => Err(unknown()),
        }
    }
}

/// 回答提问时可能出的错。
///
/// 变体按**前端能做什么**分：`Gone` 什么都不用做（那一问已经超时了，界面自己也收到了
/// `ssh_prompt_dismissed`），`Invalid` 要用户改一下再提交，`Internal` 只该记日志。
#[derive(Debug, thiserror::Error, Serialize, specta::Type)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum PromptError {
    /// 这个编号没有在等的提问：已经超时被撤下，或者已经答过了。
    #[error("提问 {id} 已经不存在了（超时或已作答）")]
    Gone { id: PromptId },

    /// 答案本身不合法（空 / 比一页受保护内存还长）。**不是截断**。
    #[error("答案不合法：{message}")]
    Invalid { message: String },

    /// 内部状态不可用（待答表中毒）。
    #[error("内部状态不可用：{message}")]
    Internal { message: String },
}

/// 回答一句秘密（口令 / 验证码 / 私钥口令）。
#[tauri::command]
#[specta::specta]
pub fn ssh_prompt_credential(
    id: PromptId,
    secret: PassphraseInput,
    prompts: State<'_, Prompts>,
) -> Result<(), PromptError> {
    prompts.answer_credential(id, secret)
}

/// 回答"这把没见过的主机密钥认不认"。
#[tauri::command]
#[specta::specta]
pub fn ssh_prompt_host_key(
    id: PromptId,
    accept: bool,
    prompts: State<'_, Prompts>,
) -> Result<(), PromptError> {
    prompts.answer_host_key(id, accept)
}

/// 撤销这一问（用户关掉了提示面板）。**不是"接受"的另一种写法**：
/// 它让连接以"用户取消"结束。
#[tauri::command]
#[specta::specta]
pub fn ssh_prompt_cancel(id: PromptId, prompts: State<'_, Prompts>) -> Result<(), PromptError> {
    prompts.cancel(id)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;
    use crate::ssh::{HostKeyCache, HostKeyVerifier, KnownHostsVerifier, RecordedHostKey};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    /// 一个记流水账的发布口：把发出去了什么记下来，并**在测试给的闭包里作答**。
    ///
    /// 为什么必须换个线程 / 换段代码来答：`ask` 会阻塞调用线程等答案 ——
    /// 同在一条路上谁都答不了它。真实形态也是分开的（前端在另一个进程里）。
    struct Recorder {
        published: Arc<Mutex<Vec<String>>>,
    }

    impl Recorder {
        fn new() -> Self {
            Self {
                published: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn lines(&self) -> Vec<String> {
            self.published.lock().unwrap().clone()
        }

        /// 装上发布口：每一问都交给 `answer` 去处置。
        ///
        /// `answer` 是在**发布口里**跑的，也就是跑在 `ask` 的调用线程上 —— 于是"作答"
        /// 走的是与真前端同一条路（`answer_*` → 送到通道），只是没有 IPC 那一段。
        fn install(
            &self,
            prompts: &Prompts,
            answer: impl Fn(&Prompts, PromptRequest) + Send + Sync + 'static,
        ) {
            let published = Arc::clone(&self.published);
            // 两份克隆：给发布口的那一份要 move 进闭包，而"作答"还要用同一张表。
            let asking = prompts.clone();
            let answering = prompts.clone();
            asking.publish_with(move |event| match event {
                Published::Asked(request) => {
                    published.lock().unwrap().push(describe(&request));
                    answer(&answering, request);
                }
                Published::Dismissed(id) => {
                    published.lock().unwrap().push(format!("dismissed:{id}"));
                }
            });
        }
    }

    /// 把一问答成一句可断言的文本（含编号 —— "答的是哪一问"本身也是判据）。
    fn describe(request: &PromptRequest) -> String {
        match request {
            PromptRequest::HostKey {
                id, fingerprint, ..
            } => format!("ask:host-key:{fingerprint}:{id}"),
            PromptRequest::Credential { id, credential, .. } => {
                format!("ask:credential:{credential:?}:{id}")
            }
        }
    }

    fn target() -> SshTarget {
        SshTarget::new("example.invalid", 22, "cyrene")
    }

    /// 一把真的公钥（`testing` 模块是 `HostKey` 的构造口所在地）。
    fn host_key() -> HostKey {
        crate::ssh::testing::host_key_from_openssh(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEXOnqIIB9VdqO+GuhPnge4qQZ4neGrbYXvtur2skY47 probe@akasha",
        )
    }

    /// 空缓存（这一组用例验的是**提问这条路**，不是库 —— 库那一侧在 `ssh.rs` 的接线里）。
    struct EmptyCache;

    impl HostKeyCache for EmptyCache {
        fn recorded(
            &self,
            _host: &str,
            _port: u16,
            _key_type: &str,
        ) -> Result<Option<RecordedHostKey>, SshError> {
            Ok(None)
        }

        fn remember(&self, _host: &str, _port: u16, _key: &HostKey) -> Result<(), SshError> {
            Ok(())
        }
    }

    /// 数"记住"发生了几次的缓存（`EmptyCache` 的观察版）。
    struct CountingCache {
        remembered: Arc<AtomicUsize>,
    }

    impl HostKeyCache for CountingCache {
        fn recorded(
            &self,
            _host: &str,
            _port: u16,
            _key_type: &str,
        ) -> Result<Option<RecordedHostKey>, SshError> {
            Ok(None)
        }

        fn remember(&self, _host: &str, _port: u16, _key: &HostKey) -> Result<(), SshError> {
            self.remembered.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn request() -> CredentialRequest {
        CredentialRequest::new(&target(), CredentialKind::LoginPassword)
    }

    #[test]
    fn a_credential_answer_reaches_the_asker() {
        let prompts = Prompts::new();
        let recorder = Recorder::new();
        recorder.install(&prompts, |prompts, request| {
            let PromptRequest::Credential { id, .. } = request else {
                panic!("这一问应该是凭据");
            };
            prompts
                .answer_credential(id, PassphraseInput::for_test("hunter2"))
                .expect("作答应当成功");
        });

        let credential = prompts.request(&request()).expect("应当拿到凭据");
        // ⚠️ 这里**读不出明文**（`expose` 是 `ssh` 内部的）：能断言的是字节数 ——
        // 而"答案原样到了问的人手里"这句话，字节数已经足够（多一个字符就对不上）。
        assert_eq!(credential.byte_len(), "hunter2".len());
        assert_eq!(prompts.pending_count(), 0, "答完必须把这一条摘掉");
        assert_eq!(recorder.lines(), vec!["ask:credential:LoginPassword:1"]);
    }

    #[test]
    fn an_unanswered_prompt_times_out_and_is_withdrawn() {
        let prompts = Prompts::new().with_timeout(Duration::from_millis(50));
        let recorder = Recorder::new();
        // 装上发布口但**不作答** —— 这正是"用户不理它"。
        recorder.install(&prompts, |_prompts, _request| {});

        let started = Instant::now();
        let err = prompts.request(&request()).err().expect("没人答就该失败");
        assert!(
            matches!(&err, SshError::CredentialUnavailable(reason) if reason.contains("超时")),
            "超时要成为可读的原因：{err}"
        );
        assert!(
            started.elapsed() >= Duration::from_millis(50),
            "不能提前放弃（等满期限才是超时）"
        );
        assert_eq!(
            recorder.lines(),
            vec!["ask:credential:LoginPassword:1", "dismissed:1"],
            "超时之后必须**撤回**这一问（界面上一份提示不能永远留着）"
        );
        assert_eq!(prompts.pending_count(), 0);
    }

    #[test]
    fn answering_after_the_timeout_is_gone_not_a_silent_success() {
        let prompts = Prompts::new().with_timeout(Duration::from_millis(20));
        let recorder = Recorder::new();
        recorder.install(&prompts, |_prompts, _request| {});
        let _ = prompts.request(&request()).err().expect("超时");

        // 前端晚到的答案：**报 Gone**，不假装成功（假装成功会让界面以为用户答上了）。
        assert!(matches!(
            prompts.answer_credential(1, PassphraseInput::for_test("too-late")),
            Err(PromptError::Gone { id: 1 })
        ));
        assert!(matches!(
            prompts.answer_host_key(999, true),
            Err(PromptError::Gone { id: 999 })
        ));
    }

    #[test]
    fn a_cancelled_credential_is_a_cancellation_not_an_empty_password() {
        let prompts = Prompts::new();
        let recorder = Recorder::new();
        recorder.install(&prompts, |prompts, request| {
            prompts.cancel(request.id()).expect("撤销应当成功");
        });

        let err = prompts.request(&request()).err().expect("取消 = 没有凭据");
        assert!(
            matches!(&err, SshError::CredentialUnavailable(reason) if reason.contains("取消")),
            "取消要有一句自己的说法（与超时分开）：{err}"
        );
    }

    #[test]
    fn a_denied_host_key_is_a_denial_and_a_missing_answer_is_not() {
        // 判据的核心：`Ok(false)` **只**表示用户拒绝了；没有答案时必须是 `Err`
        // （= 拒绝连接），绝不能读成"用户接受了"。
        let prompts = Prompts::new();
        Recorder::new().install(&prompts, |prompts, request| {
            prompts.answer_host_key(request.id(), false).expect("作答");
        });
        let verifier = KnownHostsVerifier::new(Arc::new(EmptyCache))
            .without_user_file()
            .with_prompt(Arc::new(prompts.clone()));
        let err = verifier
            .verify(&target(), &host_key())
            .expect_err("用户拒绝 = 拒绝连接");
        assert!(
            matches!(err, SshError::HostKeyRejected { .. }),
            "用户明确拒绝走的是 HostKeyRejected：{err}"
        );
        assert_eq!(prompts.pending_count(), 0);

        // 没人答（超时）：必须是 HostKeyUnknown（= 拒绝），**不是** Ok。
        let silent = Prompts::new().with_timeout(Duration::from_millis(20));
        Recorder::new().install(&silent, |_prompts, _request| {});
        let verifier = KnownHostsVerifier::new(Arc::new(EmptyCache))
            .without_user_file()
            .with_prompt(Arc::new(silent));
        let err = verifier
            .verify(&target(), &host_key())
            .expect_err("没人答 = 拒绝连接");
        assert!(
            matches!(&err, SshError::HostKeyUnknown { fingerprint, .. } if fingerprint.starts_with("SHA256:")),
            "要带上用户能核对的指纹：{err}"
        );
    }

    #[test]
    fn an_accepted_host_key_goes_through_the_cache() {
        let prompts = Prompts::new();
        let recorder = Recorder::new();
        let remembered = Arc::new(AtomicUsize::new(0));
        recorder.install(&prompts, |prompts, request| {
            prompts.answer_host_key(request.id(), true).expect("作答");
        });

        let verifier = KnownHostsVerifier::new(Arc::new(CountingCache {
            remembered: Arc::clone(&remembered),
        }))
        .without_user_file()
        .with_prompt(Arc::new(prompts.clone()));
        verifier
            .verify(&target(), &host_key())
            .expect("用户接受 = 放行");

        assert_eq!(
            remembered.load(Ordering::SeqCst),
            1,
            "确认过的密钥必须真的写进缓存（否则下一次又要问）"
        );
        assert_eq!(recorder.lines().len(), 1, "一次连接只该问一次");
    }
}
