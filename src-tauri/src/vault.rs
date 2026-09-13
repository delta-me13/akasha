//! 库的**落点、状态与解锁的生命周期**（落点与状态是 plan 0403，生命周期是 plan 0407）。
//!
//! ## 这一层回答四个问题
//!
//! | 问题 | 结论 | 为什么 |
//! |---|---|---|
//! | 谁持有解好的库 | [`Vault`]（`tauri::State`）：一个 `Mutex<Option<Unlocked>>` | 存储层是同步的、快的；专用线程要多一套 actor 协议却没有可量的收益。`Connection` 是 `Send` 不是 `Sync`，`Mutex` 正好给出"独占一次用"需要的 `&mut`（口令的 `expose` 也要 `&mut`） |
//! | 口令从哪来 | 命令参数 [`PassphraseInput`] → [`Passphrase`] | 那一份缓冲由 `memsafe` **擦零**（它是"搬"过去，不是拷贝）；够不着的副本见下面「边界」 |
//! | 什么时候锁 | **只有**显式 [`vault_lock`] | 关窗口默认只是收托盘（进程、会话、终端缓冲都留着，库也一样）；进程退出时解好的连接与锁住的那页随进程消失，写 exit hook 只是仪式；空闲超时见 plan 0407 的非目标 |
//! | 同时解锁 | 第二个拿到 [`VaultError::AlreadyUnlocked`] | 单实例（plan 0304）保证只有一个进程；进程内用一个 `Mutex` 串起来，**不替换、不静默成功** |
//!
//! ## 口令留在解锁窗口里，是 plan 0404 的硬约束
//!
//! [`Unlocked`] 里同时放着连接与口令。导出那条路要检查"导出口令与库口令不同"
//! （ADR-0002 D6），那就得在解锁窗口内拿得到库口令 —— 否则用户得为一次导出重新输一遍。
//! 代价是解锁期间多一页 `mlock`（实测 4 kB，锁定后归零）。
//!
//! ## 解锁为什么是 `async`
//!
//! KDF 实测约 105 ms（ADR-0002 §7.1）。命令体里真正慢的那段（开库 / 建库）放
//! `spawn_blocking`，于是它跑在 tokio 的阻塞线程池上，**不挡 UI 线程**。
//! `Connection` 与 `Passphrase` 都是 `Send`（`memsafe::Secret` 对静止态实现了 `Send`），
//! 所以整份 [`Unlocked`] 能跨线程回到命令里。
//!
//! ## 边界（照实说，别把它说大）
//!
//! 口令经 IPC 进来时，**tauri 内部还有两份副本够不着**：请求体的字节缓冲与
//! `serde_json::Value`。它们的生命期是这一次调用，之后被 free —— **不擦**。
//! 这一层没有接口能拿到它们的 `&mut`。实测与逐份清单见 plan 0407 的「决定 2」；
//! 能收紧的方向是让口令走 raw body，代价是前端必须手写一行裸 `invoke`
//! （`AGENTS.md` §0 绝对禁止 #1），所以**记为后续**而不是现在做。
//!
//! ## 状态命令为什么不需要口令
//!
//! 连 `user_version` 都在加密的第一页里：不开库就读不到。所以能不开库说出来的只有
//! "文件在不在、是不是空的、**锁开着没有**"，而"有文件但打不开"不是状态
//! （那是 [`vault_unlock`] 的错误）—— 所以这里没有第四个文件状态，也没有"错误口令"那条分支。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use akasha_store::{Connection, Passphrase, StoreError, VaultState, dump, vault_path};
use tauri::{AppHandle, Manager, Wry};

use crate::config;

/// 库文件的状态。**与 `akasha_store::VaultState` 分开**：这是 IPC 类型（要生成 TS），
/// 而存储 crate 不该为了生成 TS 去依赖被 app 钉住版本的 `specta`。
///
/// 两边的映射写成穷尽 `match`（下面的 `From`）：存储层加了状态，**这里编译不过** ——
/// 而不是悄悄少一个分支，让前端在一个它不认识的值上做默认动作。
///
/// ⚠️ 这个名字与 `tauri::State` 撞车，所以那几个签名里写全 `tauri::State<'_, Vault>` ——
/// 不为了省几个字把 IPC 类型改名（它已经出现在生成物里）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// 文件不存在：还没有建过。解锁会**建一个新的**。
    Missing,
    /// 文件在但是 **0 字节**：还没有密钥落在那里（这种文件用什么口令都能"打开"，
    /// 见 ADR-0002 §7）。解锁同样会建一个新的。
    Empty,
    /// 有内容。解锁会**打开它**（口令不对就打不开）。
    Present,
}

impl From<VaultState> for State {
    fn from(state: VaultState) -> Self {
        match state {
            VaultState::Missing => Self::Missing,
            VaultState::Empty => Self::Empty,
            VaultState::Present => Self::Present,
        }
    }
}

/// 口令 / 凭据**经 IPC 进来的形态**（`PassphraseInput`）。
///
/// 它是"一句秘密从前端进来"在 IPC 边界上的唯一类型 —— 库解锁（plan 0407）与 SSH 的凭据
/// 提问（plan 0504）共用它。共用而不是各写一个，是因为它的性质正是两者都要的：
/// 它唯一能做的事是 [`PassphraseInput::into_bytes`]。
///
/// 刻意**没有** `Debug`：`tracing::info!(?input)` 是**编译错误**，而不是"打码" ——
/// 与 `Passphrase` 同一手法（`AGENTS.md` §3.4）。
///
/// `#[specta(transparent)]` 让它生成成 `string`（线上确实是 JSON 字符串），
/// 前端因此不需要为它手写第二份签名（对比 `RawChannel` 那段：那个需要手写，
/// 因为频道句柄的语义生成器表达不出来）。
#[derive(serde::Deserialize, specta::Type)]
#[specta(transparent)]
pub struct PassphraseInput(String);

impl PassphraseInput {
    /// 把字节**搬**出去（不是拷贝）：随后 [`Passphrase::new`] / `Credential::new` 会把它擦零。
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.0.into_bytes()
    }

    /// 单测里造一个（真的路径只有一个来路：IPC 反序列化）。
    #[cfg(test)]
    pub(crate) fn for_test(secret: &str) -> Self {
        Self(secret.to_owned())
    }
}

/// 解锁后的库：连接 + 口令，同生共死。
///
/// 两者的生命期**必须**绑在一起：单独放一个连接，导出那条路就拿不到库口令；
/// 单独放一个口令，就没有连接可用。合成一个结构体之后，"锁"只有一种写法 ——
/// 把整个 `Some` 换成 `None`，两者一起没。
struct Unlocked {
    conn: Connection,
    passphrase: Passphrase,
}

/// 库的解锁状态。app 启动时是锁着的（不自动解锁：口令只从 [`vault_unlock`] 进来）。
///
/// `inner` 是 `Arc` 的（照 `Sessions` 的形状，plan 0504）：**解锁窗口之外也有读者** ——
/// `KnownHostsVerifier` 要一个活到连接结束（`'static`）的主机密钥缓存口，而
/// `tauri::State<'_, Vault>` 借不出这样的句柄。`Clone` 出来的是同一个 `Arc`，不是两份状态。
#[derive(Default, Clone)]
pub struct Vault {
    inner: Arc<Mutex<Option<Unlocked>>>,
}

impl Vault {
    /// 取那个槽位。Mutex 中毒 = 上一次有人在持锁时 panic 了 —— 这不是用户错误，
    /// 但也不能 `unwrap`（`AGENTS.md` §0 绝对禁止 #4）。
    fn slot(&self) -> Result<MutexGuard<'_, Option<Unlocked>>, VaultError> {
        self.inner.lock().map_err(|_| VaultError::Internal {
            message: "vault lock poisoned".to_owned(),
        })
    }

    /// **短借**解好的连接：闭包用完即还。
    ///
    /// 为什么不把 `&Connection` 返回出去：`Connection` 不是 `Sync`，而"借出去的引用
    /// 活多久"这件事我们说了不算 —— 短借是唯一能保证的形态。
    ///
    /// ⚠️ **闭包里不要做会再锁库的事**（`known_hosts::remember` 就会）：那是自己等自己。
    /// 连接中途要写库的地方，都是先把锁放掉再进去的。
    pub(crate) fn with_conn<R>(
        &self,
        f: impl FnOnce(&Connection) -> Result<R, StoreError>,
    ) -> Result<R, ConnError> {
        let slot = self
            .slot()
            .map_err(|err| ConnError::Internal(err.to_string()))?;
        let Some(unlocked) = slot.as_ref() else {
            return Err(ConnError::Locked);
        };
        f(&unlocked.conn).map_err(ConnError::Store)
    }
}

/// 这次解锁是"打开已有的"还是"建一个新的"（plan 0402 把两者拆成了两条路，这里只做**选择**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Open,
    Create,
}

/// 文件状态 → 动作。**纯函数**：单测因此能钉住"三种状态各做什么"，
/// 而不必真去开一个库（也就能在没有口令的地方验证这条规则）。
fn plan_for(state: VaultState) -> Plan {
    match state {
        VaultState::Present => Plan::Open,
        // 0 字节的文件里没有密钥（实测：用什么口令都能"打开"它），所以它不是"已有的库"。
        VaultState::Missing | VaultState::Empty => Plan::Create,
    }
}

/// `vault_status` 的返回。
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct VaultStatus {
    /// 库文件的路径（ADR-0002 D1：与 `config.json` 同一个数据目录）。
    ///
    /// 它**是**绝对路径，而这与 P2 第 2 条（"库里不存绝对路径"）不冲突：那条禁止的是
    /// **把路径存进库**（搬家后会静默失效），而这是运行时算出来给用户看的 ——
    /// 顺带回答"我的数据到底在哪"（`portable.md` §4）。
    pub path: String,
    pub state: State,
    /// 库**开着**没有（解密连接在不在）。与 `state` 是两件事：文件一直在，
    /// 而锁开了、又锁上了。
    pub unlocked: bool,
}

/// 解锁之后读一次四套池的结果。
///
/// 它存在的理由不是"给用户看几个数字"，而是让 `AGENTS.md` §7 那条"真实路径走通"
/// 有对象：真 app 上 `invoke` 出来的这份数字，正是**库真的被解开并读通了**的证据
/// （只有拿到了正确的密钥才读得出行数）。
#[derive(Debug, Clone, Copy, serde::Serialize, specta::Type)]
pub struct VaultContents {
    /// 计数是 `u32` 而不是 `usize`：生成器**拒绝**把 64 位整数导出成 TS
    /// （BigInt 的精度问题，与问题 #32 同类）。这里不做 `as` 截断，而是 checked 转换。
    pub keys: u32,
    pub hosts: u32,
    pub serials: u32,
    pub forwards: u32,
}

impl VaultContents {
    /// 读一次库。用 `dump` 而不是四套池各自的 `list`：那正是 dump 存在的理由，
    /// 而且它是**结构上不含私钥**的（`Key` 里没有那个字段）—— 这条路径上没有机密。
    fn read(conn: &Connection) -> Result<Self, VaultError> {
        // 数组模式解构是穷尽的：`row_counts` 换长度时这里**编译不过**（顺序也钉住了）。
        let [keys, hosts, serials, forwards] = dump::dump(conn)
            .map_err(VaultError::from_store)?
            .row_counts();
        Ok(Self {
            keys: count(keys)?,
            hosts: count(hosts)?,
            serials: count(serials)?,
            forwards: count(forwards)?,
        })
    }
}

/// `usize` → `u32`，**checked**（不写 `as`：静默截断会把"数"变成谎话）。
///
/// 这条分支实际不可达 —— 任何一张表都不可能有一行不到一百字节的四十亿行；
/// 留着它不是为那一天，是为了**不引入一处静默截断**。
fn count(rows: usize) -> Result<u32, VaultError> {
    u32::try_from(rows).map_err(|_| VaultError::Unusable {
        message: format!("池里的行数超出可表示范围（{rows}）"),
    })
}

/// 取库的状态时可能出的错。
///
/// 与 `IpcError` 分开：域不同，前端能据此做的动作也不同（这里是"环境或口令的问题"，
/// 而不是"某个会话坏了"）。
///
/// 变体按**用户的下一步动作**合并（与 `StoreError::Conflict` 同一个理由）：
/// 版本不对 / 缺表 / 文件在半路被删了，对用户都是"这个库用不了，看消息"。
#[derive(Debug, thiserror::Error, serde::Serialize, specta::Type)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum VaultError {
    /// 连数据目录都取不到（既没有便携目录，OS 数据目录也解析失败）。
    ///
    /// ⚠️ **不退回 OS 目录**（`portable.md` §2 第 3 条）：用户以为数据在 U 盘上、
    /// 实际落在本地磁盘，比"明确报错"坏得多。
    #[error("数据目录不可用（便携目录与 OS 数据目录都取不到）")]
    NoDataDir,

    /// 看文件状态这一步失败了（权限 / IO）。
    #[error("无法查看库文件：{message}")]
    Io { message: String },

    /// 已经解开了。**不替换**：静默换掉一个正在用的连接，会让"我现在用的是哪把口令"
    /// 变成一个没人答得上来的问题。
    #[error("库已经解锁了（要换口令先锁定）")]
    AlreadyUnlocked,

    /// 打不开：口令错**或**文件不是本程序的库。SQLCipher 对这两者给同一个
    /// `SQLITE_NOTADB`，我们也不假装能区分（文案因此把两种可能都说出来）。
    #[error("打不开库：口令不对，或者这个文件不是 akasha 的库")]
    WrongPassphrase,

    /// 空口令。前端本该拦住，但**后端也不接受** —— 空 key 会让 SQLCipher 退化成明文库
    /// （ADR-0002 D5）。
    #[error("口令不能为空")]
    EmptyPassphrase,

    /// 口令比一页受保护内存还长（`akasha_store::MAX_LEN`）。**不是截断**：
    /// 截断会让"口令错了"变成一件没人能解释的事。
    #[error("口令太长了（上限 {max} 字节）")]
    PassphraseTooLong { max: u32 },

    /// **内存锁不住**（`mlock` 被拒）：`memsafe` 没有降级路径，所以解锁**不能进行**。
    ///
    /// 这条必须是用户能懂的一句话，而不是一个错误码：它是"这台机器上你解不开自己的库"，
    /// 下一步动作是查 `ulimit -l` / 容器里的 `RLIMIT_MEMLOCK`，而不是重试。
    #[error("内存锁不住（mlock 失败），出于安全拒绝解锁：{message}")]
    NotLockable { message: String },

    /// 库**锁着**，而这条路径需要一个解开的库。
    ///
    /// 与 [`VaultError::AlreadyUnlocked`] 是一对：那个说"已经有连接了，别再开一个"，
    /// 这个说"还没有连接，先解开"。目前只有 SSH 那条路会走到这里（它的信任记录与
    /// 密钥池都在库里），但文案不写"SSH" —— 错误说的是**库的状态**，不是谁在问。
    #[error("库是锁着的：这条路径需要先解锁（口令只从解锁命令进来）")]
    Locked,

    /// 这个库用不了：版本不认识、缺表、或者文件在半路没了。
    #[error("库不可用：{message}")]
    Unusable { message: String },

    /// 内部状态不可用（Mutex 中毒、阻塞任务 join 失败）。
    /// **不是用户错误**，但也不能 `unwrap`。
    #[error("内部状态不可用：{message}")]
    Internal { message: String },
}

/// [`Vault::with_conn`] 借库时的两种失败。
///
/// 为什么不直接给 `VaultError`：**怎么说，由调用方决定**。
/// `vault_hosts` 把它翻成 `VaultError`（用户的动作是"看消息"），而 SSH 那条路要
/// 单独认出 [`StoreError::NoSuchRow`]（用户的动作是"回池子里重挑一台"）——
/// 在这一层就折成一句话，那个区别就没了。锁着这一档两条路都要单独认，所以它在类型里。
#[derive(Debug)]
pub(crate) enum ConnError {
    /// 库是锁着的（还没有解开的连接）。
    Locked,
    /// 库那一侧报的错（表 / 行 / 文件 / 机密页……）。
    Store(StoreError),
    /// 连槽位都拿不到（Mutex 中毒）。**不是用户错误**，但也不能 `unwrap`。
    Internal(String),
}

impl std::fmt::Display for ConnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // 与 `VaultError::Locked` 的文案同源：同一个状态不该有两种说法。
            Self::Locked => write!(f, "库是锁着的：这条路径需要先解锁"),
            Self::Store(err) => write!(f, "{err}"),
            Self::Internal(message) => write!(f, "{message}"),
        }
    }
}

impl VaultError {
    /// [`ConnError`] → IPC 这一侧的说法（`vault_hosts` 那条路的唯一入口）。
    pub(crate) fn from_conn(err: ConnError) -> Self {
        match err {
            ConnError::Locked => Self::Locked,
            ConnError::Store(err) => Self::from_store(err),
            ConnError::Internal(message) => Self::Internal { message },
        }
    }

    /// 把存储层的错误翻成 IPC 这一侧的说法。
    ///
    /// 穷尽 `match`：`akasha-store` 加一个变体，**这里编译不过** ——
    /// 而不是悄悄落进某个兜底分支，让用户在一句没有信息量的话上做决定。
    fn from_store(err: StoreError) -> Self {
        match err {
            StoreError::NotADatabase => Self::WrongPassphrase,
            StoreError::EmptyPassphrase => Self::EmptyPassphrase,
            StoreError::PassphraseTooLong { max } => Self::PassphraseTooLong {
                // `akasha-store` 里那一页是 256 字节，且有 `const _ = assert!(MAX_LEN <= i32::MAX)`
                // —— 这里一定放得下。写 `unwrap_or` 而不是 `as`，是为了不引入静默截断
                // （真放不下时给一个"大到不可能"的上限，比给一个错的数字好）。
                max: u32::try_from(max).unwrap_or(u32::MAX),
            },
            // 用户要看的是**操作系统的原话**（"Cannot allocate memory" / "Operation not
            // permitted"），不是上游那层包装（`Memory error: …`）—— 下一步动作在 OS 那句话里：
            // `ulimit -l`、容器里的 `RLIMIT_MEMLOCK`。
            StoreError::MemoryProtection(err) => Self::NotLockable {
                message: err.inner().to_string(),
            },
            // 版本 / 缺表 / 文件在半路没了 / 其余 sqlite 与 IO：用户的动作是同一个 ——
            // 看消息、别再重试。`NoVault` 归这里：状态是 Present 却开不出来，
            // 说明文件在"看一眼"和"打开"之间变了（另一条路正在动它，或者被删了）。
            StoreError::UnsupportedVersion { found } => Self::Unusable {
                message: format!("不认识的格式版本 {found}"),
            },
            // 需要升级但升不动（最常见的是文件不可写）。**与"版本不认识"分开**：
            // 这里程序**知道**怎么升，只是环境不允许 —— 用户的动作是让文件可写，
            // 而不是换一个程序版本。plan 0503 起 `open` 会为了迁移而写文件。
            StoreError::UpgradeFailed { from, detail } => Self::Unusable {
                message: format!("库需要从 v{from} 升级到当前格式，但升级没成功：{detail}"),
            },
            StoreError::MissingTable { table } => Self::Unusable {
                message: format!("缺表 {table}"),
            },
            StoreError::NoVault(path) => Self::Unusable {
                message: format!("文件不见了：{}", path.display()),
            },
            other @ (StoreError::VaultExists(_)
            | StoreError::Io(_)
            | StoreError::Sqlite(_)
            | StoreError::KeyRejected(_)
            | StoreError::SecretTooLong { .. }
            | StoreError::EmptyPrivateKey
            | StoreError::NoSuchRow { .. }
            | StoreError::Conflict { .. }
            | StoreError::JumpChain
            | StoreError::PlaintextRefused { .. }
            | StoreError::SharedPassphrase) => Self::Unusable {
                message: other.to_string(),
            },
        }
    }
}

/// 解锁：打开已有的库；**没有库就建一个**（文件不存在或 0 字节）。
///
/// 建 / 开的选择来自 [`plan_for`]，而"开"与"建"仍然是存储层的两条路（plan 0402）——
/// 这里没有把它们合并成一个"打不开就建"的兜底：那正是实测里"任何口令都能打开一个
/// 0 字节文件"的那条歧路。
///
/// 已经解开时返回 [`VaultError::AlreadyUnlocked`]：**不替换、不重复开**。
/// 检查是 check-then-act：并发调用时两边都会算一次 KDF（约 105 ms）而只有一个成功 ——
/// 代价照实记，不为它加一套排队逻辑。
#[tauri::command]
#[specta::specta]
pub async fn vault_unlock(
    app: AppHandle<Wry>,
    passphrase: PassphraseInput,
) -> Result<VaultContents, VaultError> {
    let path = vault_file(&app)?;
    let state = akasha_store::vault_state(&path).map_err(VaultError::from_store)?;
    let plan = plan_for(state);

    // 慢的那一段（KDF + 建库 / 开库）挪出 UI 线程。整份 `Unlocked` 是 `Send` 的，
    // 所以连接与口令能跨线程搬回来。
    let unlocked = tauri::async_runtime::spawn_blocking(move || unlock(&path, plan, passphrase))
        .await
        .map_err(|err| VaultError::Internal {
            message: err.to_string(),
        })?
        .map_err(VaultError::from_store)?;

    // 先读一次四套池**再**装进槽位：读失败时什么都不留在进程里（连接与口令一起 drop）。
    let contents = VaultContents::read(&unlocked.conn)?;

    let vault = app.state::<Vault>();
    let mut slot = vault.slot()?;
    if slot.is_some() {
        return Err(VaultError::AlreadyUnlocked);
    }
    if plan == Plan::Create {
        tracing::info!("vault created");
    }
    *slot = Some(unlocked);
    tracing::info!(
        keys = contents.keys,
        hosts = contents.hosts,
        serials = contents.serials,
        forwards = contents.forwards,
        "vault unlocked"
    );
    Ok(contents)
}

/// 锁定：把解好的连接与口令**一起**丢掉（`Some` → `None`）。
///
/// 返回"刚才是不是真锁上了一个"：调用方（前端 / 测试）要靠它区分
/// "锁上了"与"本来就是锁着的"—— 后者不是错误，但也不该被报成一次状态变化。
///
/// 抹掉了什么、抹不掉什么：见模块文档的「边界」与 plan 0407 的副本清单。
/// 口令那一页由 `memsafe` `munmap`（实测 `VmLck` 归零）；SQLCipher 内部的密钥材料
/// 由 `cipher_memory_security` 在释放时擦零（实测：派生密钥的副本一处不剩）。
#[tauri::command]
#[specta::specta]
pub fn vault_lock(state: tauri::State<'_, Vault>) -> Result<bool, VaultError> {
    let Some(unlocked) = state.slot()?.take() else {
        return Ok(false);
    };
    // 两半都显式丢掉，而不是靠 `Option` 顺手带走 —— 锁定的语义**就是**这两件事，
    // 而"顺手带走"在有人往结构体里加第三个字段（比如一块缓存）时会静默漏掉它：
    //   * `conn` 的 drop 释放 SQLCipher 的密钥材料（它自己擦零后再 `munlock`）；
    //   * `passphrase` 的 drop 让 `memsafe` `munmap` 那一页受保护内存。
    let Unlocked { conn, passphrase } = unlocked;
    drop(conn);
    drop(passphrase);
    tracing::info!("vault locked");
    Ok(true)
}

/// 在阻塞线程上做的全部工作。**密钥的副本只在这一条路上**：
/// `into_bytes()` 把 IPC 反序列化出来的那块缓冲搬进 [`Passphrase::new`]，
/// 后者成功时由 `memsafe` 把它擦零。
fn unlock(path: &Path, plan: Plan, passphrase: PassphraseInput) -> Result<Unlocked, StoreError> {
    let mut passphrase = Passphrase::new(passphrase.into_bytes())?;
    let conn = match plan {
        Plan::Open => akasha_store::open(path, &mut passphrase)?,
        Plan::Create => akasha_store::create(path, &mut passphrase)?,
    };
    Ok(Unlocked { conn, passphrase })
}

/// 库文件在哪。
fn vault_file(app: &AppHandle<Wry>) -> Result<PathBuf, VaultError> {
    let dir = config::data_dir_of(app).ok_or(VaultError::NoDataDir)?;
    Ok(vault_path(&dir))
}

/// 库在哪、建过没有、开着没有。**只读元数据 + 一个布尔**：不打开库、不创建目录、不要口令。
///
/// 它**不记日志**：那是一个查询（前端可能反复调），而日志留给状态**变化** ——
/// 这里没有变化，只有"此刻是什么"。
#[tauri::command]
#[specta::specta]
pub fn vault_status(
    app: AppHandle<Wry>,
    state: tauri::State<'_, Vault>,
) -> Result<VaultStatus, VaultError> {
    let path = vault_file(&app)?;
    let file = akasha_store::vault_state(&path).map_err(VaultError::from_store)?;

    Ok(VaultStatus {
        path: path.display().to_string(),
        state: file.into(),
        unlocked: state.slot()?.is_some(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    /// 三种文件状态各做什么。**这是"打开"与"新建"分岔的那条规则**，
    /// 而它不需要口令、不需要真库就能验 —— 比"去建一个库看看能不能开"精确得多。
    #[test]
    fn a_missing_or_empty_file_is_created_and_a_present_one_is_opened() {
        assert_eq!(plan_for(VaultState::Missing), Plan::Create);
        assert_eq!(plan_for(VaultState::Empty), Plan::Create);
        assert_eq!(plan_for(VaultState::Present), Plan::Open);
    }

    /// 口令错了与"文件不是个库"**给同一句话**：SQLCipher 本来就分不出来
    /// （都是 `SQLITE_NOTADB`），文案因此不能声称是其中之一。
    #[test]
    fn a_wrong_passphrase_and_a_foreign_file_say_the_same_thing() {
        let err = VaultError::from_store(StoreError::NotADatabase);
        assert!(matches!(err, VaultError::WrongPassphrase));
        let text = err.to_string();
        assert!(text.contains("口令不对"), "{text}");
        assert!(text.contains("不是 akasha 的库"), "{text}");
    }

    /// `mlock` 失败要变成**用户能懂的一句话**（plan 0407 的目标之一），
    /// 而不是一个裸的错误码。
    ///
    /// 这里造的是**真的** `StoreError::MemoryProtection`：`memsafe` 的错误只有
    /// `From<io::Error>` 一个公开构造口，而 `.into()` 的目标类型由变体推出来 ——
    /// 所以这条用例不需要 app 依赖 `memsafe`，也不会因为上游改了错误类型就悄悄失真。
    #[test]
    fn a_memory_lock_failure_explains_itself() {
        let err = VaultError::from_store(StoreError::MemoryProtection(
            std::io::Error::other("Cannot allocate memory (os error 12)").into(),
        ));
        let text = err.to_string();
        assert!(text.contains("内存锁不住"), "{text}");
        assert!(text.contains("拒绝解锁"), "{text}");
        assert!(
            !text.contains("Memory error"),
            "不该把上游那层包装的话丢给用户：{text}"
        );
    }

    /// 版本不认识 / 缺表 / 文件半路没了：**各自说清是什么**，但不各占一个变体
    /// （用户的下一步动作是同一个：看消息）。这条钉住"信息没在这些分支里丢掉"。
    #[test]
    fn an_unusable_vault_still_says_what_is_wrong() {
        let cases = [
            (
                StoreError::UnsupportedVersion { found: 0 },
                "不认识的格式版本 0",
            ),
            (StoreError::MissingTable { table: "hosts" }, "缺表 hosts"),
        ];
        for (err, expected) in cases {
            let text = VaultError::from_store(err).to_string();
            assert!(text.contains(expected), "期望 {expected:?}，实际 {text:?}");
        }
    }
}
