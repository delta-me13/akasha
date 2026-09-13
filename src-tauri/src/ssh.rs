//! **SSH 接进 IPC**（plan 0504 / 跳板链 plan 0505）—— 让真 app 能开一个 SSH 会话。
//!
//! 这个模块是"库那一侧"与"界面那一侧"之间的接线，一共四件事：
//!
//! | 事 | 在哪 | 为什么在这里 |
//! |---|---|---|
//! | 专用 runtime（ADR-0003 D2） | [`Ssh`] | 建 runtime 是 app 的事，库只收 `Handle` |
//! | 库内的主机密钥缓存（D11） | [`VaultHostKeys`] | 库连接归解锁窗口（plan 0407），verifier 要 `'static` 口 |
//! | 一条带目标的会话命令（D3） | [`open_ssh_session`] | `Sessions::register` 已经是 `<T: Transport>` |
//! | **跳板链**（D9） | [`plan_chain`] | 链住**池**里（`hosts.jump_id`），而库不知道池长什么样 |
//!
//! ## 三条别改回去的边界
//!
//! 1. **连接跑在一条普通 `std::thread` 上**。`SshTransport::connect_via` 会在
//!    `RuntimeHandle::try_current()` 命中时拒绝（[`SshError::BlockingInsideRuntime`]），
//!    而 `spawn_blocking` 的线程**也算**在 tokio 上下文里（它 `rt.enter()` 过）——
//!    所以既不能用 async command 直接调，也不能丢给 `spawn_blocking`。
//! 2. **不持着库的锁去连接**。`known_hosts` 的 `remember` 会在连接中途回头锁库
//!    （用户确认一把没见过的密钥时）—— 持锁连接就是自己等自己。跳板链因此是
//!    **一次读完整条**（[`plan_chain`]）再开始连，而不是"连一跳读一行"。
//! 3. **库锁着 = 不连**。信任记录与密钥池都在库里，"绕过缓存连上"会把 D11 的三态
//!    退化成两态，而缺的那一态正是警报那一态（[`SshFailureKind::HostKeyChanged`]）。
//!
//! ## 失败面的分法
//!
//! [`SshIpcError`] 的变体按**用户的下一步动作**分（同 `VaultError` / `StoreError` 的原则）：
//! 先解锁（`Locked`）、去池子里挑一台（`NoSuchHost`）、去查（`HostKeyChanged` 是警报）、
//! 去核对凭据（`Auth`）、去查网络 / 服务端（`Connect`）、去看**跳板机能不能看见目标**
//! （`Jump`）。

use std::sync::Arc;

use akasha_pty::TerminalSize;
use akasha_ssh::{
    CredentialCache, HostKey, HostKeyCache, KeyCandidate, KnownHostsVerifier, RecordedHostKey,
    SshAuth, SshConfig, SshConnect, SshError, SshTarget, SshTransport,
};
use akasha_store::StoreError;
use akasha_store::pools::hosts::Auth;
use serde::Serialize;
use tauri::{AppHandle, Manager, Webview};
use tokio::runtime::{Handle as RuntimeHandle, Runtime};

use crate::pools::HostId;
use crate::prompt::Prompts;
use crate::session::{self, IpcError, RawChannel, SessionHandle, Sessions};
use crate::vault::{ConnError, Vault};

/// SSH runtime 的 worker 数。
///
/// 为什么不是 `Runtime::new()`（= 按 CPU 数铺线程）：这条 runtime 上的活儿很少，
/// 唯一的坑是**提问会占住一个 worker**（`check_server_key` / 认证回调是同步的，
/// 提问期间就停在那儿，最长 [`crate::prompt::PROMPT_TIMEOUT`]）。
/// 4 个 worker 够两三条连接同时提问而不至于把 runtime 攥死，也不会在大机器上空铺线程。
const SSH_WORKERS: usize = 4;

/// SSH 需要的长住状态：专用 runtime、内存凭据缓存、提问往返。
///
/// 由 tauri 作为 `State` 持有。**不看 `Clone`**：拿到它的地方都是 `State<'_, Ssh>`，
/// 而需要 `'static` 的两样（`CredentialCache` / `Prompts`）各自已经是 `Arc` / `Clone` 的。
pub struct Ssh {
    /// 专用 runtime。`None` = 起不来（降级：命令返回明确错误，app 照常跑）。
    runtime: Option<Runtime>,
    /// 起不来的原因（留给 `.setup()` 记日志 —— 那时日志出口才就绪，见 `lib.rs`）。
    startup_error: Option<String>,
    /// 凭据缓存（D8）。进程内存，**绝不落盘**。
    cache: Arc<CredentialCache>,
    /// 提问往返（本 plan）。
    prompts: Prompts,
}

impl Ssh {
    /// 建那一个专用 runtime（D2）并挂上缓存与提问口。
    ///
    /// **失败不挡启动**（`AGENTS.md` §3.3）：SSH 是可选能力，起不来只该让"开 SSH 会话"
    /// 报一句明确的话，而不是让整个终端打不开。
    pub fn start() -> Self {
        match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(SSH_WORKERS)
            .thread_name("akasha-ssh")
            .enable_all()
            .build()
        {
            Ok(runtime) => Self {
                runtime: Some(runtime),
                startup_error: None,
                cache: Arc::new(CredentialCache::new()),
                prompts: Prompts::new(),
            },
            Err(err) => Self {
                runtime: None,
                startup_error: Some(err.to_string()),
                cache: Arc::new(CredentialCache::new()),
                prompts: Prompts::new(),
            },
        }
    }

    /// 起不来的原因（`None` = 起来了）。
    pub fn startup_error(&self) -> Option<&str> {
        self.startup_error.as_deref()
    }

    /// 把提示接进真正的前端（`.setup()` 里调用，`mount_events` 之后）。
    pub fn connect_frontend(&self, app: AppHandle) {
        self.prompts.to_frontend(app);
    }

    /// 提问往返的把手（回答命令与适配器都要它）。
    pub fn prompts(&self) -> &Prompts {
        &self.prompts
    }

    fn handle(&self) -> Result<RuntimeHandle, SshIpcError> {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.handle().clone())
            .ok_or_else(|| SshIpcError::Internal {
                message: format!(
                    "SSH runtime 起不来：{}",
                    self.startup_error.as_deref().unwrap_or("原因未知")
                ),
            })
    }
}

/// IPC 边界的 SSH 错误。变体按**用户的下一步动作**分（见模块文档）。
#[derive(Debug, thiserror::Error, Serialize, specta::Type)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum SshIpcError {
    /// 库没解锁。**不连**：信任记录与密钥池都在库里，绕过去就等于把三态判定降成两态。
    #[error("库是锁着的：SSH 要先解锁（主机密钥记录与密钥池都在库里）")]
    Locked,

    /// 池里没有这一行（`id` 不存在，或者它引用的密钥不见了）。
    #[error("主机池里没有这台主机（id {id}）")]
    NoSuchHost { id: HostId },

    /// 连接这条路失败。`kind` 是给界面分辨**警报**用的，`message` 是给人看的整句话。
    #[error("{message}")]
    Failed {
        kind: SshFailureKind,
        message: String,
    },

    /// 内部状态不可用（runtime 起不来、会话表中毒、连接线程起不来）。
    #[error("内部状态不可用：{message}")]
    Internal { message: String },
}

/// 连接失败的类别。**前端据此决定"要不要让用户核对什么"** —— 不是把消息字符串
/// 拿去匹配（那是最容易在改文案时静默失效的一种判据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SshFailureKind {
    /// **服务端的密钥与记下来的不一样**（ADR-0003 D11 的警报，阶段 6 也不重连）。
    HostKeyChanged,
    /// 用户否认了一把没见过的密钥 —— 与上一档分开：那是常态，这是明确拒绝。
    HostKeyRejected,
    /// 没见过这把密钥，而没问成（超时 / 没人可问）。**不是"接受"的近义词**。
    HostKeyUnknown,
    /// 库内的主机密钥缓存用不了（写了 / 读不出）—— 提示里那条"先解锁"多半是它。
    HostKeyCache,
    /// 认证失败：我们有的方式都试过了。
    Auth,
    /// 连不上 / 超时 / 通道开不出来。
    Connect,
    /// **跳板这条线断了**（plan 0505）：链成环 / 过长，或者跳板机拒绝转发到目标。
    ///
    /// 与 [`SshFailureKind::Connect`] 分开是刻意的：那一档是"我连不上**你挑的那台**"，
    /// 这一档是"**跳板机**够不着它"。混在一起用户会去查自己的网络，而问题在对端。
    Jump,
    /// 其余（错误消息里说得清）。
    Other,
}

impl SshIpcError {
    /// 把 `akasha-ssh` 的说法收进 IPC 这一侧。
    ///
    /// ⚠️ 这里**带兜底分支**（`Other`）是有意的：`SshError` 是分域定义的一大族，
    /// 而 IPC 只需要把"用户下一步做什么"分出来；多出来的变体不该让**这里**编译不过
    /// （那会把库的一次内部改名变成 app 的编译错误）。细节一个字不少地进 `message`。
    fn from_ssh(err: SshError) -> Self {
        let kind = match &err {
            SshError::HostKeyChanged { .. } => SshFailureKind::HostKeyChanged,
            SshError::HostKeyRejected { .. } => SshFailureKind::HostKeyRejected,
            SshError::HostKeyUnknown { .. } => SshFailureKind::HostKeyUnknown,
            SshError::HostKeyCache(_) => SshFailureKind::HostKeyCache,
            SshError::AuthenticationFailed { .. } => SshFailureKind::Auth,
            SshError::Connect { .. } | SshError::ConnectTimeout { .. } => SshFailureKind::Connect,
            SshError::Forward { .. } => SshFailureKind::Jump,
            SshError::Channel(_) | SshError::HostKeyUnusable { .. } => SshFailureKind::Other,
            // 凭据要不到 / 空值 / 不是 UTF-8 / 受保护页 / 在 tokio 上下文里调同步门面。
            _ => SshFailureKind::Other,
        };
        Self::Failed {
            kind,
            message: err.to_string(),
        }
    }

    /// 库那一侧的说法 → 这一侧。
    ///
    /// [`StoreError::NoSuchRow`] 单独认出来：那是"用户挑错了主机"，不是"库坏了" ——
    /// 两者的下一步动作不同（回池子里重挑 vs 看消息）。其余一律归到
    /// [`SshFailureKind::HostKeyCache`]：这条路上读库只为主机密钥与钥匙，
    /// 读不动就等于信任记录不可用。
    fn from_conn(err: ConnError) -> Self {
        match err {
            ConnError::Locked => Self::Locked,
            // 行号在这一层认不出来（错误里只有池名）—— 需要具体行号的那个调用点自己认。
            ConnError::Store(StoreError::NoSuchRow { .. }) => Self::NoSuchHost { id: 0 },
            ConnError::Store(other) => Self::Failed {
                kind: SshFailureKind::HostKeyCache,
                message: other.to_string(),
            },
            ConnError::Internal(message) => Self::Internal { message },
        }
    }
}

/// 库内的主机密钥缓存（ADR-0003 D11 的 [`HostKeyCache`] 实现）。
///
/// `Vault` 是 `Clone` 的（同一个 `Arc`），所以这个适配器是 `'static` 的 ——
/// 而那正是 `KnownHostsVerifier` 要活到连接结束所需要的。
struct VaultHostKeys {
    vault: Vault,
}

impl HostKeyCache for VaultHostKeys {
    fn recorded(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
    ) -> Result<Option<RecordedHostKey>, SshError> {
        let found = self
            .vault
            .with_conn(|conn| akasha_store::pools::known_hosts::lookup(conn, host, port, key_type))
            .map_err(unavailable)?;
        Ok(found.map(|row| RecordedHostKey {
            // 判定材料是**本体**，不是指纹文本（D11）。
            blob: row.key_blob,
            fingerprint: row.fingerprint,
        }))
    }

    fn remember(&self, host: &str, port: u16, key: &HostKey) -> Result<(), SshError> {
        let new = akasha_store::pools::known_hosts::NewKnownHost {
            host: host.to_owned(),
            port,
            key_type: key.algorithm().to_owned(),
            key_blob: key.blob().to_vec(),
            fingerprint: key.fingerprint().to_owned(),
        };
        self.vault
            .with_conn(|conn| akasha_store::pools::known_hosts::remember(conn, &new))
            .map(|_id| ())
            .map_err(unavailable)
    }
}

/// 库这一侧的失败 → [`SshError::HostKeyCache`]。**一律拒绝连接**（见那个变体的文档）。
fn unavailable(err: ConnError) -> SshError {
    SshError::HostKeyCache(err.to_string())
}

/// 从池里读出这次连接要用的东西。**读完就把库的锁放掉**（模块文档的边界 2）。
struct Planned {
    target: SshTarget,
    auth: SshAuth,
}

/// **整条跳板链**：`[目标, 它的跳板, 跳板的跳板, …]`（与 `hosts::jump_chain` 同序）。
///
/// 目标在前是 `akasha-store` 定的顺序（"我要连**这台**，它得先经**那台**"）；
/// 连接那一步要的是反过来，由 [`open_ssh_session`] 翻转 —— 靠近使用点翻转，
/// 比让每一层都记一遍"哪个是最外层"要可靠。
fn plan_chain(vault: &Vault, id: HostId) -> Result<Vec<Planned>, SshIpcError> {
    // 池那一层用 `i64` 作行 id，IPC 这一侧是 `u32` 代理 —— 到这里再换回去（不是截断，
    // 而是回到它本来的宽度：`HostId` 就是从 `i64` checked 出来的）。
    let rows = vault
        .with_conn(|conn| akasha_store::pools::hosts::jump_chain(conn, i64::from(id)))
        .map_err(|err| match err {
            ConnError::Locked => SshIpcError::Locked,
            // 池里没有这一行是"用户挑错了"，不是库坏了 —— 这一步单独认
            // （`from_conn` 认不出行号，而链里断了一行也要说清是**哪个 id** 起头的）。
            ConnError::Store(StoreError::NoSuchRow { .. }) => SshIpcError::NoSuchHost { id },
            // 成环 / 过深：**不是**"库坏了"，是这份配置连不通。`from_conn` 会把它归到
            // 主机密钥缓存不可用 —— 那个提示会把人引到解锁上去，方向完全错。
            ConnError::Store(StoreError::JumpChain) => SshIpcError::Failed {
                kind: SshFailureKind::Jump,
                message: format!("主机池里的跳板链成环或过长，连不到 id {id} 这台"),
            },
            err => SshIpcError::from_conn(err),
        })?;

    rows.into_iter().map(|row| planned(vault, row)).collect()
}

/// 一行池记录 → "连它要什么"。
///
/// 认证方式决定带不带钥匙、试不试 agent（ADR-0003 D7 的顺序由 `akasha-ssh` 定，
/// 这里只决定**给它什么材料**）—— **每一跳各算各的**：跳板与目标是两台机器，
/// 凭据与钥匙都各是各的。
fn planned(vault: &Vault, row: akasha_store::pools::hosts::Host) -> Result<Planned, SshIpcError> {
    let auth = match row.auth {
        Auth::Password => SshAuth::keys(Vec::new()),
        Auth::Agent => SshAuth::agent_only(),
        // `publickey` 可以没有 `key_id`：那时钥匙在 ssh-agent 里（池不存别人的私钥）。
        Auth::PublicKey => match row.key_id {
            Some(key_id) => SshAuth::keys(vec![candidate(vault, key_id)?]),
            None => SshAuth::agent_only(),
        },
    };

    Ok(Planned {
        target: SshTarget::new(row.host.clone(), row.port, row.user.clone()),
        auth,
    })
}

/// 从密钥池取一把钥匙，装成候选。
///
/// ⚠️ 这里会**短暂地**在普通堆上存在一份私钥明文（`to_vec()`）：`russh` 要
/// `ssh_key::PrivateKey` 才能签名，而那是普通堆上的东西 —— D8 照实记的边界。
/// 缓解与 0502 相同：`KeyCandidate::new` 会把源缓冲擦零，之后它只活在受保护页里。
fn candidate(vault: &Vault, key_id: i64) -> Result<KeyCandidate, SshIpcError> {
    let pem = vault
        .with_conn(|conn| {
            let mut private = akasha_store::pools::keys::private_key(conn, key_id)?;
            let exposed = private.expose()?;
            Ok(exposed.to_vec())
        })
        .map_err(SshIpcError::from_conn)?;
    KeyCandidate::new(stable_id(key_id), pem).map_err(SshIpcError::from_ssh)
}

/// 候选私钥的**稳定标识**（进凭据缓存的键：同一台主机上两把钥匙各有各的口令）。
///
/// 用行 id：池里的 `update` 会保留 id，所以"换了材料而标识没变"在**同一个解锁窗口内**
/// 可能留下一条过期口令。那条路的自愈是现成的（`akasha-ssh` 解不开就 `forget` 再问一次），
/// 而缓存本身**只在内存里**（锁定 / 退出即清空）—— 所以这不值得为它加一列哈希。
fn stable_id(key_id: i64) -> String {
    format!("key#{key_id}")
}

/// 建立连接（**同步门面，异步在门後面**）。
///
/// 一条普通 `std::thread` 上跑 `SshTransport::connect_via`：那条路径内部会
/// `Handle::block_on`（在 tokio 上下文里会 panic / 被拒），而 `spawn_blocking` 的线程
/// 也算 tokio 上下文 —— 所以只能自己起线程。结果经一条 oneshot 回到 async 命令，
/// 于是**没有任何一个 runtime worker 被这一次连接占住**。
async fn establish(
    handle: RuntimeHandle,
    hops: Vec<SshConnect>,
    options: SshConnect,
) -> Result<SshTransport, SshIpcError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("akasha-ssh-connect".to_owned())
        .spawn(move || {
            // 送不出去只可能是命令那一侧已经走了（超时被前端放弃 / app 在退出）——
            // 那一边本来也不等，所以不记日志。
            let _ = sender.send(SshTransport::connect_via(&handle, hops, options));
        })
        .map_err(|err| SshIpcError::Internal {
            message: format!("连接线程起不来：{err}"),
        })?;

    receiver
        .await
        .map_err(|_| SshIpcError::Internal {
            message: "连接线程没有回话".to_owned(),
        })?
        .map_err(SshIpcError::from_ssh)
}

/// 连接取值。默认值只有**一处**（`SshConfig::default()`），这里只把它组装起来。
fn config() -> SshConfig {
    SshConfig::default()
}

/// 打开一个 SSH 终端会话，输出经 `channel` 以 **raw 字节**送出。
///
/// 与 [`crate::session::open_session`] 的关系：**同一条尾巴**（注册 → 频道 → 收尾线程），
/// 差的是载体怎么来 —— 本地那条是 `PtyTransport::spawn_default()`，这条是
/// "照池里那一行连过去"。
///
/// ⚠️ **async**：命令体里有一次会阻塞几秒的握手（最长 `connect_timeout`），而
/// 同步 command 跑在处理 IPC 请求的那条线程上（挡住它就等于挡住全部 IPC ——
/// 包括用户回答问题要用的那三条命令）。
#[tauri::command]
#[specta::specta]
pub async fn open_ssh_session(
    app: AppHandle,
    webview: Webview,
    channel: RawChannel,
    host_id: HostId,
) -> Result<SessionHandle, SshIpcError> {
    let channel = channel.into_channel(webview).map_err(|err| match err {
        IpcError::Channel { message } => SshIpcError::Internal {
            message: format!("频道句柄无效：{message}"),
        },
        other => SshIpcError::Internal {
            message: other.to_string(),
        },
    })?;

    let ssh = app.state::<Ssh>();
    let handle = ssh.handle()?;
    let vault = app.state::<Vault>();
    // 短借：把整条链读出来（每跳的地址与认证材料）就把锁放掉
    // —— 连接中途 `remember` 要回头锁库（模块文档的边界 2）。
    let mut links = plan_chain(&vault, host_id)?;
    // 链是"目标在前"读出来的；连接要的是"最外层先连"，所以把目标摘出来、其余翻转。
    let target_link = links.remove(0);
    links.reverse();

    let prompts = ssh.prompts().clone();
    // 每一跳各装一份：**每一跳各问各的凭据、各校各的主机密钥**（跳板与目标是两台机器，
    // 信任记录与密钥池当然各是各的）。缓存是共享的 —— 它按 `(host, port, user, 方式)` 分键。
    let link_options = |link: Planned| SshConnect {
        target: link.target,
        auth: link.auth,
        cache: Arc::clone(&ssh.cache),
        provider: Arc::new(prompts.clone()),
        host_keys: Arc::new(
            KnownHostsVerifier::new(Arc::new(VaultHostKeys {
                vault: Vault::clone(&vault),
            }))
            .with_prompt(Arc::new(prompts.clone())),
        ),
        config: config(),
        size: TerminalSize::DEFAULT,
    };
    let hops: Vec<SshConnect> = links.into_iter().map(link_options).collect();
    let target = target_link.target.clone();
    let options = link_options(target_link);

    tracing::info!(
        host = target.host(),
        port = target.port(),
        user = target.user(),
        hops = hops.len(),
        "ssh session opening"
    );

    let transport = establish(handle, hops, options).await?;
    let sessions = app.state::<Sessions>();
    session::open_terminal(&app, &sessions, transport, channel).map_err(Into::into)
}

impl From<IpcError> for SshIpcError {
    fn from(err: IpcError) -> Self {
        Self::Internal {
            message: err.to_string(),
        }
    }
}
