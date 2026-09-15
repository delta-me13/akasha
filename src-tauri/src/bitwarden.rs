//! Bitwarden 的接线：两个轴、运行时下载、登录 / 解锁 / 锁定（plan 0902 / 0905）。
//!
//! 形状的出处是 [ADR-0007](../../docs/adr/0007-bitwarden-cli-acquisition.md)，
//! `bw` 的命令面与实测输出在 [`docs/bitwarden.md`](../../docs/bitwarden.md)；
//! 本模块只做三件事：**解析那一份 CLI**、**把命令串起来**、**把失败翻成界面认得的形状**。
//!
//! ## 为什么这里有一把锁
//!
//! `bw` 自己在 `data.json` 上**没有任何互斥**：两条命令同时跑会互相覆盖状态
//! （`akasha-bw` 的 crate 文档记了这一条）。所以本模块的全部动作都在一条 `Mutex` 上排队 ——
//! 它不是为了"并发安全"，而是因为被调用的那个程序不并发安全。
//!
//! ## session key 只在这个进程的内存里
//!
//! 它住在 [`Session`]（受保护页，ADR-0007 D7），**不落盘、不进日志、不进任何事件载荷**。
//! 界面能看到的只有"现在有没有"（[`BwSnapshot::has_session`]）。锁 / 登出 / 进程退出都会
//! 让它消失；此外每次动作之后重新读一次 `bw status --raw`，**状态说不是解锁就把手里的丢掉**
//! （ADR-0007 D10）：外部用 `bw lock` 锁过之后那份 key 已经失效，留着只会让下一条命令
//! 以 `You are not logged in.` 失败。
//!
//! ## 为什么没有事件
//!
//! 这一版的三态只在**用户按下按钮之后**才变（登录 / 解锁 / 锁定 / 登出），而每一次都经命令
//! 返回同一份快照。加一个事件就得回答"谁还会改这个状态"（这一版的答案是"没有"），
//! 而那个答案将来会变 —— 等真的有了后台同步再补。
//!
//! ## 形状：`Arc` + 一次性配置
//!
//! 数据目录在 `.setup()` 里才知道，而 `.manage()` 更早 —— 所以默认值先放进去、
//! 到 `.setup()` 再 [`Bitwarden::configure`] 覆盖一次。`Bitwarden` 内部是 `Arc`，
//! clone 出来的是同一份（probe 与命令看到的是同一个状态）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use akasha_bw::{BwError, Cli, Paths, Session, Settings, State, Status};
use tauri::State as TauriState;

use crate::config;
use crate::vault::PassphraseInput;

/// app 侧的长住状态。**一次只跑一个 `bw`**（理由见模块文档）。
#[derive(Clone, Default)]
pub struct Bitwarden {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// 数据目录在 `.setup()` 里才知道；在那之前一切都是"还没初始化"。
    dir: Option<PathBuf>,
    settings: Settings,
    /// 上一次**探测**出来的东西（起因与缓存键见 [`inspect`] 的文档）。
    probed: Option<(PathBuf, Probed)>,
    /// 手上的 session key（**只在内存**）。
    session: Option<Session>,
    /// CLI 里**当前配置的**服务器（`bw config server` 的回读）。
    ///
    /// ⚠️ 它与 `status.server_url` **不是同一个东西**：未登录时上游的 `status` 给的是
    /// `null`（实测），而配置好的地址一直读得出来 —— 面板在登录之前要显示的是这一个。
    server: Option<String>,
    /// 最近一次读到状态 —— 只给快照用，"真相"永远在下一次 `bw status` 里。
    last: Option<Status>,
}

impl Bitwarden {
    /// `.setup()` 里定下数据目录与两个轴（启动期 `.manage()` 只能先放默认值）。
    pub fn configure(&self, dir: PathBuf, settings: Settings) {
        let mut inner = self.lock();
        inner.dir = Some(dir);
        inner.settings = settings;
    }

    /// `PoisonError` 说明另一个线程在持锁时 panic 了 —— 那时表里的状态不可信，
    /// 但**没有可用的降级**（返回错误会让界面永久卡在"坏了"），所以取回那份数据。
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// 失败的形状。**界面按 `kind` 分辨**，不匹配消息字符串（同 `SshIpcError` 的口径）。
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BwIpcError {
    pub kind: BwErrorKind,
    /// `bw` 自己说的那句话（或我们的可读原因）。**只用来显示**。
    pub message: String,
}

/// 与 [`akasha_bw::BwError`] 一一对应。分成两份是因为 IPC 上要的是一个能穷尽 `switch`
/// 的枚举，而 crate 的错误带着内部细节（路径、长度、被拒绝的地址）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum BwErrorKind {
    /// `PATH` 里没有 `bw`（`host` 那一轴）。
    MissingBinary,
    /// 还没下载过（`managed` 那一轴）。
    NotInstalled,
    NotRunnable,
    UnsupportedTarget,
    NoRelease,
    Network,
    Tls,
    InsecureUrl,
    InvalidServer,
    /// `bw` 说没有登录 —— 界面上该做的是"先登录"。
    NotLoggedIn,
    /// `bw` 说这个 vault 锁着 —— 界面上该做的是"先解锁"（与上一条不同的一句话）。
    Locked,
    /// `bw` 以非 0 退出而我们认不出类别（原话在 `message` 里）。
    CommandFailed,
    Parse,
    Archive,
    Io,
    Timeout,
    ProtectedPage,
    Internal,
}

impl From<BwError> for BwIpcError {
    fn from(err: BwError) -> Self {
        let kind = match err {
            BwError::MissingBinary => BwErrorKind::MissingBinary,
            BwError::NotInstalled { .. } => BwErrorKind::NotInstalled,
            BwError::NotRunnable { .. } => BwErrorKind::NotRunnable,
            BwError::UnsupportedTarget { .. } => BwErrorKind::UnsupportedTarget,
            BwError::NoRelease { .. } => BwErrorKind::NoRelease,
            BwError::Network { .. } => BwErrorKind::Network,
            BwError::Tls { .. } => BwErrorKind::Tls,
            BwError::InsecureUrl { .. } => BwErrorKind::InsecureUrl,
            BwError::InvalidServer { .. } => BwErrorKind::InvalidServer,
            BwError::NotLoggedIn => BwErrorKind::NotLoggedIn,
            BwError::Locked => BwErrorKind::Locked,
            BwError::CommandFailed { .. } => BwErrorKind::CommandFailed,
            BwError::Parse { .. } => BwErrorKind::Parse,
            BwError::Archive { .. } => BwErrorKind::Archive,
            BwError::Io { .. } => BwErrorKind::Io,
            BwError::Timeout { .. } => BwErrorKind::Timeout,
            // 空 key 与超长 key 都是"形状"那一档：用户要做的事一样（重新解锁）。
            BwError::TokenTooLong { .. } | BwError::EmptyToken => BwErrorKind::Parse,
            BwError::ProtectedPage { .. } => BwErrorKind::ProtectedPage,
            // 手上没有 session key 与"没登录"在界面上是同一句话：先登录 / 先解锁。
            BwError::NoSession => BwErrorKind::NotLoggedIn,
        };
        Self {
            kind,
            message: err.to_string(),
        }
    }
}

/// CLI 那一块：这两个轴解析出来是什么。
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BwCliInfo {
    /// `host` / `managed`。
    pub binary: String,
    /// `host` / `managed`。
    pub appdata: String,
    /// 解析到的可执行文件（解析不出来时为空）。
    pub program: Option<String>,
    pub version: Option<String>,
    /// `oss` / `proprietary` / `unknown`。
    pub variant: Option<String>,
    /// 需要提示许可证时的那句话（只有专有变体有）。
    pub license_notice: Option<String>,
    /// 解析不出来时的原因。**"没有 `bw`"与"还没下载"是不同的两句**（ADR-0007 D5）。
    pub problem: Option<String>,
}

/// 三态的投影。取值与 `bw status --raw` 一致（`unauthenticated` / `locked` / `unlocked`）。
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BwVaultStatus {
    pub server_url: Option<String>,
    pub last_sync: Option<String>,
    pub user_email: Option<String>,
    pub user_id: Option<String>,
    pub state: String,
}

impl From<Status> for BwVaultStatus {
    fn from(status: Status) -> Self {
        Self {
            server_url: status.server_url,
            last_sync: status.last_sync,
            user_email: status.user_email,
            user_id: status.user_id,
            state: match status.state {
                State::Unauthenticated => "unauthenticated",
                State::Locked => "locked",
                State::Unlocked => "unlocked",
            }
            .to_owned(),
        }
    }
}

/// 一次命令之后的完整读数（也是 `bitwarden` probe 报的东西）。
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BwSnapshot {
    pub cli: BwCliInfo,
    /// CLI 里当前配置的服务器（`bw config server` 的回读）—— 登录之前也读得到。
    pub server: Option<String>,
    /// CLI 说得出来的状态；`bw` 自己跑不起来时为 `None`。
    pub status: Option<BwVaultStatus>,
    /// **我们手里有没有 session key**（不是 key 本身）。
    pub has_session: bool,
    /// 读状态失败的原因。
    pub problem: Option<String>,
}

/// 探测出来的东西里**与程序有关**的那一半（缓存的就是它）。
#[derive(Clone)]
struct Probed {
    version: Option<String>,
    variant: Option<String>,
    license_notice: Option<String>,
    problem: Option<String>,
}

/// 解析出可执行文件与它的自报信息（会起进程：`--version` 与 `--help`）。
///
/// ## 为什么带缓存
///
/// `bw` 的启动不便宜（上游那份是约 140 MB 的 Node SEA；本机这个 npm 版在只读家目录下 13 秒
/// 才报错），而每个快照都要读版本与变体 —— 那两条对一个**给定的程序文件**是不变的。
/// 缓存键因此是**解析出来的程序路径**：换轴、换版本目录、或用户装/卸 `bw` 之后它自己失效。
///
/// ⚠️ 缓存的是**与程序有关**的那一半；`binary` / `appdata` 两个字段每次都按当前设置重写
/// （否则换轴之后快照会报上一次的轴）。
fn inspect(inner: &mut Inner, paths: &Paths, settings: Settings) -> (BwCliInfo, Option<Cli>) {
    let located = match paths.resolve(settings) {
        Ok(located) => located,
        Err(err) => {
            let info = BwCliInfo {
                binary: settings.binary.as_str().to_owned(),
                appdata: settings.appdata.as_str().to_owned(),
                program: None,
                version: None,
                variant: None,
                license_notice: None,
                problem: Some(err.to_string()),
            };
            // 解析不到就不是"那一份程序变了"，把缓存丢掉（下次解析成功时要重新探测）。
            inner.probed = None;
            return (info, None);
        }
    };
    let program = located.program.clone();
    let cli = Cli::new(&located);

    let probed = match &inner.probed {
        Some((cached, probed)) if *cached == program => probed.clone(),
        _ => {
            let probed = ask(&cli);
            inner.probed = Some((program.clone(), probed.clone()));
            probed
        }
    };

    (
        BwCliInfo {
            binary: settings.binary.as_str().to_owned(),
            appdata: settings.appdata.as_str().to_owned(),
            program: Some(program.display().to_string()),
            version: probed.version,
            variant: probed.variant,
            license_notice: probed.license_notice,
            problem: probed.problem,
        },
        Some(cli),
    )
}

/// 真的去问那一份 CLI：版本、变体、许可证提示。
///
/// ⚠️ 名字不叫 `probe`：本模块另有一个给 Victauri 的 `pub fn probe(&Bitwarden)`。
fn ask(cli: &Cli) -> Probed {
    let mut probed = Probed {
        version: None,
        variant: None,
        license_notice: None,
        problem: None,
    };
    match cli.version() {
        Ok(version) => probed.version = Some(version),
        Err(err) => {
            probed.problem = Some(err.to_string());
            return probed;
        }
    }
    match cli.variant() {
        Ok(variant) => {
            probed.variant = Some(
                match variant {
                    akasha_bw::Variant::Oss => "oss",
                    akasha_bw::Variant::Proprietary => "proprietary",
                    akasha_bw::Variant::Unknown => "unknown",
                }
                .to_owned(),
            );
            if variant.needs_license_notice() {
                probed.license_notice = Some(format!(
                    "这一份是{}：它的许可证把用途限制在内部开发与测试、非生产环境。\
                     要避开这条限制，可以改用运行时下载的 OSS 那一份。",
                    variant.describe()
                ));
            }
        }
        Err(err) => probed.problem = Some(err.to_string()),
    }
    probed
}

/// 组装快照：解析 → （可选）读状态 → 处理 session。
///
/// `refresh` 为真时才起 `bw status`（"只是看看设置"不必每次都起一个约 140 MB 的进程）。
fn snapshot(bitwarden: &Bitwarden, refresh: bool) -> BwSnapshot {
    let mut inner = bitwarden.lock();
    let settings = inner.settings;
    let Some(dir) = inner.dir.clone() else {
        return BwSnapshot {
            cli: BwCliInfo {
                binary: settings.binary.as_str().to_owned(),
                appdata: settings.appdata.as_str().to_owned(),
                program: None,
                version: None,
                variant: None,
                license_notice: None,
                problem: Some("还没初始化（数据目录未定）".to_owned()),
            },
            server: None,
            status: None,
            has_session: false,
            problem: None,
        };
    };
    let paths = Paths::new(&dir);
    let (info, cli) = inspect(&mut inner, &paths, settings);
    let mut problem = info.problem.clone();

    // ⚠️ 这一份 CLI 连 `--version` 都答不出来时**不再起后面的进程**：那不是"状态读不出来"，
    // 而是"这一份 `bw` 根本用不了"—— 本机的 `/usr/bin/bw`（发行版的 npm 包）在只读家目录下
    // 要 **13 秒**才报错，而每个快照原本要起三次进程（版本 / 帮助 / 状态），
    // 那会把处理 IPC 的那条线程占住半分钟。
    let usable = info.version.is_some();
    if refresh
        && usable
        && let Some(cli) = &cli
    {
        inner.server = cli.server().unwrap_or(None);
        match cli.status() {
            Ok(status) => {
                // D10：状态说不是解锁，就丢掉手上的 key。
                if !status.state.allows_session() {
                    inner.session = None;
                }
                inner.last = Some(status);
            }
            Err(err) => problem = Some(err.to_string()),
        }
    }

    let status = inner.last.clone().map(BwVaultStatus::from);
    BwSnapshot {
        cli: info,
        server: inner.server.clone(),
        status,
        has_session: inner.session.is_some(),
        problem,
    }
}

/// 一次动作 + 之后的刷新，全在**同一把锁**上（一次只跑一个 `bw`）。
fn act(
    bitwarden: &Bitwarden,
    action: impl FnOnce(&Cli, &mut Inner) -> Result<(), BwError>,
) -> Result<BwSnapshot, BwIpcError> {
    {
        let mut inner = bitwarden.lock();
        let settings = inner.settings;
        let Some(dir) = inner.dir.clone() else {
            return Err(BwIpcError {
                kind: BwErrorKind::Internal,
                message: "还没初始化（数据目录未定）".to_owned(),
            });
        };
        let paths = Paths::new(&dir);
        let located = paths.resolve(settings).map_err(BwIpcError::from)?;
        let cli = Cli::new(&located);
        action(&cli, &mut inner).map_err(BwIpcError::from)?;
        // 动作之后重新读一次配置与状态（ADR-0007 D10）。设服务器那一条改的就是前者，
        // 而未登录时 `status` 里的 `serverUrl` 是 `null`（实测）—— 只看它就等于
        // "刚设完地址，界面还显示没设"。
        inner.server = cli.server().unwrap_or(None);
        match cli.status() {
            Ok(status) => {
                if !status.state.allows_session() {
                    inner.session = None;
                }
                inner.last = Some(status);
            }
            Err(err) => tracing::warn!(%err, "bitwarden status unreadable after an action"),
        }
    }
    Ok(snapshot(bitwarden, false))
}

/// 读一次 CLI 与状态（面板打开时用）。
#[tauri::command]
#[specta::specta]
pub fn bw_cli_status(bitwarden: TauriState<'_, Bitwarden>) -> BwSnapshot {
    snapshot(&bitwarden, true)
}

/// 换两个轴。**只改选择并记下来**，不下载（下载是 [`bw_cli_install`]）。
#[tauri::command]
#[specta::specta]
pub fn bw_cli_settings(
    bitwarden: TauriState<'_, Bitwarden>,
    binary: String,
    appdata: String,
) -> Result<BwSnapshot, BwIpcError> {
    let binary = akasha_bw::BinarySource::parse(&binary).ok_or_else(|| BwIpcError {
        kind: BwErrorKind::Internal,
        message: format!("不认识的二进制来源 {binary:?}"),
    })?;
    let appdata = akasha_bw::AppData::parse(&appdata).ok_or_else(|| BwIpcError {
        kind: BwErrorKind::Internal,
        message: format!("不认识的状态目录 {appdata:?}"),
    })?;
    let settings = Settings { binary, appdata };

    let dir = {
        let mut inner = bitwarden.lock();
        inner.settings = settings;
        // 换 CLI 之后手里那份 session key 多半不再适用（换的是**状态目录**时尤其明确）。
        inner.session = None;
        inner.last = None;
        inner.dir.clone()
    };
    if let Some(dir) = dir {
        config::save_bitwarden(&dir, settings).map_err(|err| BwIpcError {
            kind: BwErrorKind::Io,
            message: err.to_string(),
        })?;
    }
    Ok(snapshot(&bitwarden, true))
}

/// 下载一份 OSS 变体并**改用它**（下载是用户动作，ADR-0007 D2）。
///
/// ⚠️ **async 且把阻塞那一半放进阻塞池**：下载约 45 MB，同步命令会把处理 IPC 的那条线程
/// 占住（同 `open_ssh_session` 的理由）。下载**不持锁** —— 它只写数据目录下的版本目录，
/// 不碰 CLI 的状态文件，所以这期间面板还能读状态。
#[tauri::command]
#[specta::specta]
pub async fn bw_cli_install(
    bitwarden: TauriState<'_, Bitwarden>,
) -> Result<BwSnapshot, BwIpcError> {
    // 先克隆一份 `Arc`：`State<'_, _>` 跨 `await` 会把借用一起带进去，而这里只需要那份状态。
    let bitwarden = bitwarden.inner().clone();
    let dir = bitwarden.lock().dir.clone().ok_or_else(|| BwIpcError {
        kind: BwErrorKind::Internal,
        message: "还没初始化（数据目录未定）".to_owned(),
    })?;

    let installed_dir = dir.clone();
    let installed = tauri::async_runtime::spawn_blocking(move || {
        let paths = Paths::new(&installed_dir);
        let http = akasha_bw::Http::new(akasha_bw::timeout::DOWNLOAD);
        akasha_bw::acquire::install(&http, &akasha_bw::Sources::default(), &paths)
    })
    .await
    .map_err(|err| BwIpcError {
        kind: BwErrorKind::Internal,
        message: format!("下载任务没有跑起来：{err}"),
    })?
    .map_err(BwIpcError::from)?;

    tracing::info!(
        version = %installed.version,
        sha256 = %installed.sha256,
        "bitwarden cli installed"
    );

    let settings = {
        let mut inner = bitwarden.lock();
        // "下载并使用这一份"是一件事：只装不切，`host` 那一轴上仍然什么都用不了。
        inner.settings.binary = akasha_bw::BinarySource::Managed;
        inner.session = None;
        inner.last = None;
        inner.settings
    };
    config::save_bitwarden(&dir, settings).map_err(|err| BwIpcError {
        kind: BwErrorKind::Io,
        message: err.to_string(),
    })?;
    Ok(snapshot(&bitwarden, true))
}

/// 读一次三态（界面刷新用）。
///
/// **不返回 `Err`**：读不出来是快照里的一个字段（`problem`），而不是一次调用失败 ——
/// 界面要显示的是"为什么读不出来"那句话，而不是一个被抛出的错误。
#[tauri::command]
#[specta::specta]
pub fn bw_status(bitwarden: TauriState<'_, Bitwarden>) -> BwSnapshot {
    snapshot(&bitwarden, true)
}

/// 设服务器地址（自托管）。官方云的默认值**不写** `config`：`bw` 自己的默认就是它。
#[tauri::command]
#[specta::specta]
pub fn bw_server_set(
    bitwarden: TauriState<'_, Bitwarden>,
    url: String,
) -> Result<BwSnapshot, BwIpcError> {
    act(&bitwarden, |cli, _| {
        cli.set_server(&url)?;
        Ok(())
    })
}

/// 登录（邮箱 + 主密码；两步验证可选）。成功时 session key 进内存。
#[tauri::command]
#[specta::specta]
pub fn bw_login(
    bitwarden: TauriState<'_, Bitwarden>,
    email: String,
    password: PassphraseInput,
    method: Option<String>,
    code: Option<String>,
) -> Result<BwSnapshot, BwIpcError> {
    // ⚠️ 借的是这两个 `String`，**不许 `Box::leak`**：那会把每一条口令的副本永久留在堆上。
    let two_factor = match (method.as_deref(), code.as_deref()) {
        (Some(method), Some(code)) => Some(akasha_bw::cli::TwoFactor { method, code }),
        _ => None,
    };
    act(&bitwarden, |cli, inner| {
        let session = cli.login(&email, password.as_str(), two_factor)?;
        inner.session = Some(session);
        Ok(())
    })
}

/// 解锁。成功时**换掉**手上的 session key（旧的在 CLI 那一侧已经失效）。
#[tauri::command]
#[specta::specta]
pub fn bw_unlock(
    bitwarden: TauriState<'_, Bitwarden>,
    password: PassphraseInput,
) -> Result<BwSnapshot, BwIpcError> {
    act(&bitwarden, |cli, inner| {
        let session = cli.unlock(password.as_str())?;
        inner.session = Some(session);
        Ok(())
    })
}

/// 锁定：让 CLI 那一侧的 key 失效，并抹掉手上的那一份。
#[tauri::command]
#[specta::specta]
pub fn bw_lock(bitwarden: TauriState<'_, Bitwarden>) -> Result<BwSnapshot, BwIpcError> {
    act(&bitwarden, |cli, inner| {
        cli.lock()?;
        inner.session = None;
        Ok(())
    })
}

/// 登出：连登录态一起清掉。
#[tauri::command]
#[specta::specta]
pub fn bw_logout(bitwarden: TauriState<'_, Bitwarden>) -> Result<BwSnapshot, BwIpcError> {
    act(&bitwarden, |cli, inner| {
        cli.logout()?;
        inner.session = None;
        Ok(())
    })
}

/// 与上游对齐（纯 pull）。需要 session key。
#[tauri::command]
#[specta::specta]
pub fn bw_sync(bitwarden: TauriState<'_, Bitwarden>) -> Result<BwSnapshot, BwIpcError> {
    act(&bitwarden, |cli, inner| {
        let session = inner.session.as_mut().ok_or(BwError::NoSession)?;
        cli.sync(session)
    })
}

// ── 只读导入 SSH key 条目（plan 0903） ──────────────────────────────────────

/// 导入报告（过 IPC 的形状）。
///
/// 它要回答三个问题，缺一个用户就没法相信这次导入 —— 与 `import_ssh_config` 的
/// [`crate::pools::ImportReport`] 同一形状：
///
/// | 字段 | 回答 |
/// |---|---|
/// | [`Self::seen`] / [`Self::ssh_keys`] | **上游那边看到了什么**（全部条目数 / 其中 SSH key 数） |
/// | [`Self::created`] / [`Self::replaced`] | **池里变了吗** |
/// | [`Self::skipped`] | **哪几条没进来、为什么**（逐条给下一步动作） |
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BwImportReport {
    /// 上游一共给了多少条（**全部类型**）。
    pub seen: u32,
    /// 其中 `type = 5`（SSH key）的。
    pub ssh_keys: u32,
    pub created: Vec<ImportedKey>,
    pub replaced: Vec<ImportedKey>,
    pub skipped: Vec<SkippedKey>,
    /// 我们替用户补上的说明（目前只有一条：钥匙怎么才会接到主机上）。
    pub notes: Vec<String>,
}

/// 池里新增 / 被替换的一把钥匙。
///
/// **不带行 id**：报告要回答的是"哪一把钥匙进来了"，而名字是池里的 `UNIQUE` 列；
/// 行 id 是 `i64`，过 IPC 要另一套 checked 转换（同 `HostId` 的理由）—— 这里不需要它。
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportedKey {
    pub name: String,
    /// 上游报的那串 `SHA256:…`（plan 0904 的离线自检拿它当参照）。
    pub fingerprint: String,
}

/// 看见了、但**没动**的一条。
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SkippedKey {
    pub name: String,
    pub reason: BwImportSkip,
}

/// 没动这一条的原因。**每个取值对应一个不同的下一步动作**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum BwImportSkip {
    /// 池里已经有同名的行了。
    Exists,
    /// **这一批里**已经有同名的了（上游允许两个条目同名）。
    DuplicateName,
    /// 池里那一行已经归**另外一条**上游条目了。
    Claimed,
    /// 这一条只有公钥（上游允许），没有私钥可导。
    NoPrivateKey,
    /// 缺 id / 名字 / `revisionDate`：导进来会成为一条以后认不出来的记录。
    NoProvenance,
    /// 私钥超过受保护页（16 KiB），进不了库 —— 与 `keys::PrivateKey::new` 同一条上限。
    TooLong,
}

impl From<akasha_store::pools::bw_items::Skip> for BwImportSkip {
    fn from(skip: akasha_store::pools::bw_items::Skip) -> Self {
        use akasha_store::pools::bw_items::Skip;
        match skip {
            Skip::Exists => Self::Exists,
            Skip::DuplicateName => Self::DuplicateName,
            Skip::Claimed => Self::Claimed,
        }
    }
}

/// 导入 SSH key 失败。变体按**用户的下一步动作**分（与 `ImportError` 同一原则）。
#[derive(Debug, thiserror::Error, serde::Serialize, specta::Type)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum BwImportError {
    /// 库没解锁。导入要**写**进密钥池，所以解锁是硬前提（不像读快照那样只是读不到）。
    #[error("库是锁着的：导入要把钥匙写进密钥池，先解锁")]
    Locked,

    /// `bw` 那一侧的失败。`kind` 与面板其余部分同一个域（前端按它分辨，不匹配消息字符串）。
    #[error("{message}")]
    Bw { kind: BwErrorKind, message: String },

    /// 其余（数据目录未定、写库失败……）。
    #[error("{message}")]
    Failed { message: String },
}

impl From<BwError> for BwImportError {
    fn from(err: BwError) -> Self {
        let ipc = BwIpcError::from(err);
        Self::Bw {
            kind: ipc.kind,
            message: ipc.message,
        }
    }
}

impl From<crate::vault::ConnError> for BwImportError {
    fn from(err: crate::vault::ConnError) -> Self {
        match err {
            crate::vault::ConnError::Locked => Self::Locked,
            other => Self::Failed {
                message: other.to_string(),
            },
        }
    }
}

/// 把 Bitwarden 里的 SSH key 条目**只读导入**本地密钥池（plan 0903）。
///
/// 两把锁**不同时持有**：先在 `bw` 那把锁里跑完 `list items` 并解析成我们自己的类型，
/// 放锁之后才去动库。顺序反过来（持库锁去起进程）会让一次 `bw` 调用把整个库锁住几百毫秒。
///
/// 这一条**不返回快照**：导入不改三态，也不改两个轴 —— 面板上要刷新的东西由调用方自己再读一次。
#[tauri::command]
#[specta::specta]
pub fn bw_import_keys(
    bitwarden: TauriState<'_, Bitwarden>,
    vault: TauriState<'_, crate::vault::Vault>,
    overwrite: bool,
) -> Result<BwImportReport, BwImportError> {
    let akasha_bw::Inventory { total, ssh_keys } = list_ssh_keys(&bitwarden)?;

    // 上游的条目 → 落库要的形状。三档"导不进来"在这里逐条记下来，**不是整批失败**：
    // 一条只有公钥的条目不该让另外九条好的也进不来。
    let mut incoming: Vec<akasha_store::pools::bw_items::Incoming> = Vec::new();
    let mut skipped: Vec<SkippedKey> = Vec::new();
    for key in ssh_keys {
        if !key.has_private_key() {
            skipped.push(SkippedKey {
                name: key.name,
                reason: BwImportSkip::NoPrivateKey,
            });
            continue;
        }
        if !key.is_recordable() {
            skipped.push(SkippedKey {
                name: key.name,
                reason: BwImportSkip::NoProvenance,
            });
            continue;
        }
        // `into_bytes` 把那个 `String` 的缓冲**移**过来（没有第二份副本），
        // 而 `PrivateKey::new` 成功时会把这块缓冲擦零。
        let name = key.name;
        let private = match akasha_store::pools::keys::PrivateKey::new(key.private_key.into_bytes())
        {
            Ok(private) => private,
            // 另一档（空私钥）在上面就已经挡掉了 —— 这里只剩"超过一页"。
            Err(_) => {
                skipped.push(SkippedKey {
                    name,
                    reason: BwImportSkip::TooLong,
                });
                continue;
            }
        };
        incoming.push(akasha_store::pools::bw_items::Incoming {
            cipher_id: key.id,
            name,
            revision_date: key.revision_date,
            fingerprint: key.fingerprint,
            public_key: key.public_key,
            private,
        });
    }

    // 报告要按名字说清"进来的是哪一把"，而落库那边只回行 id + 名字 —— 指纹先留一份。
    let fingerprints: std::collections::BTreeMap<String, String> = incoming
        .iter()
        .map(|item| (item.name.clone(), item.fingerprint.clone()))
        .collect();

    let outcome = vault
        .with_conn(|conn| {
            akasha_store::pools::bw_items::import_snapshot(conn, &mut incoming, overwrite)
        })
        .map_err(BwImportError::from)?;

    tracing::info!(
        seen = total,
        ssh_keys = fingerprints.len(),
        created = outcome.created.len(),
        replaced = outcome.replaced.len(),
        skipped = skipped.len() + outcome.skipped.len(),
        "bitwarden ssh keys imported"
    );

    let named = |rows: Vec<akasha_store::pools::bw_items::Row>| -> Vec<ImportedKey> {
        rows.into_iter()
            .map(|row| ImportedKey {
                fingerprint: fingerprints.get(&row.name).cloned().unwrap_or_default(),
                name: row.name,
            })
            .collect()
    };
    let created = named(outcome.created);
    let replaced = named(outcome.replaced);
    let mut all_skipped = skipped;
    all_skipped.extend(outcome.skipped.into_iter().map(|(name, skip)| SkippedKey {
        name,
        reason: BwImportSkip::from(skip),
    }));

    let mut notes = Vec::new();
    if !created.is_empty() || !replaced.is_empty() {
        // 这条 note 不是装饰：钥匙进了池不等于主机用得上它。主机引用钥匙的唯一方式是
        // `~/.ssh/config` 导入时 `IdentityFile` 的 basename 与钥匙名**逐字符相同**。
        notes.push(
            "钥匙进池了，但主机还不会用它：再导入一次 `~/.ssh/config`，\
             里面 `IdentityFile` 的**文件名**与这里的钥匙名相同时就会接上。"
                .to_owned(),
        );
    }

    Ok(BwImportReport {
        seen: u32::try_from(total).unwrap_or(u32::MAX),
        ssh_keys: u32::try_from(fingerprints.len()).unwrap_or(u32::MAX),
        created,
        replaced,
        skipped: all_skipped,
        notes,
    })
}

/// 跑一次 `bw list items --raw` 并解析成 [`akasha_bw::Inventory`]（**不碰库**）。
///
/// 那段输出是整个 vault 的明文，它在这一趟里的生命期就是这一行：`Cli::items` 返回
/// `Zeroizing`，[`akasha_bw::items::parse`] 读完，函数返回时缓冲区被擦零。
fn list_ssh_keys(bitwarden: &Bitwarden) -> Result<akasha_bw::Inventory, BwImportError> {
    let mut inner = bitwarden.lock();
    let settings = inner.settings;
    let Some(dir) = inner.dir.clone() else {
        return Err(BwImportError::Failed {
            message: "还没初始化（数据目录未定）".to_owned(),
        });
    };
    let located = Paths::new(&dir)
        .resolve(settings)
        .map_err(BwImportError::from)?;
    let cli = Cli::new(&located);
    let session = inner.session.as_mut().ok_or_else(|| BwImportError::Bw {
        kind: BwErrorKind::NotLoggedIn,
        message: BwError::NoSession.to_string(),
    })?;

    let raw = cli.items(session).map_err(BwImportError::from)?;
    akasha_bw::items::parse(&raw).map_err(BwImportError::from)
}

/// `bitwarden` probe：与 [`bw_cli_status`] 同一份读数（`AGENTS.md` §7：观察后端状态用 probe）。
///
/// ⚠️ 它**不起进程**（`refresh = false`）：probe 会被反复读，而起一次 `bw` 是几百毫秒。
/// 报的是上一次动作留下的读数。
pub fn probe(bitwarden: &Bitwarden) -> serde_json::Value {
    let snapshot = snapshot(bitwarden, false);
    serde_json::to_value(snapshot)
        .unwrap_or_else(|err| serde_json::json!({ "problem": format!("快照序列化失败：{err}") }))
}

/// 启动时读一次配置里的两个轴（`.setup()` 里用）。
pub fn settings_from_config(dir: &Path) -> Settings {
    let settings = config::bitwarden(dir);
    tracing::info!(
        binary = settings.binary.as_str(),
        appdata = settings.appdata.as_str(),
        "bitwarden settings loaded"
    );
    settings
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn an_unconfigured_state_says_so_instead_of_pretending() {
        let bitwarden = Bitwarden::default();
        let snapshot = snapshot(&bitwarden, true);
        assert!(snapshot.cli.problem.is_some(), "还没配置就得说清");
        assert_eq!(snapshot.cli.binary, "host", "默认那一轴要如实报出来");
        assert!(!snapshot.has_session);
    }

    /// 没有 session 时 `sync` 报的是"先登录 / 先解锁"那一档，而不是内部错误。
    #[test]
    fn syncing_without_a_session_is_that_own_kind() {
        let err = BwIpcError::from(BwError::NoSession);
        assert_eq!(err.kind, BwErrorKind::NotLoggedIn);
    }

    /// 界面按 `kind` 分辨：同一档的两种原因给出同一个 kind，而原话各自保留。
    #[test]
    fn the_kind_is_independent_of_the_message() {
        let a = BwIpcError::from(BwError::Network {
            message: "超时".to_owned(),
        });
        let b = BwIpcError::from(BwError::Network {
            message: "DNS 失败".to_owned(),
        });
        assert_eq!(a.kind, b.kind);
        assert_ne!(a.message, b.message);
    }

    /// "没找到 `bw`"与"还没下载"必须是两句话 —— 这正是 ADR-0007 D5 的落点。
    #[test]
    fn missing_and_not_installed_are_different_kinds() {
        let missing = BwIpcError::from(BwError::MissingBinary);
        let not_installed = BwIpcError::from(BwError::NotInstalled {
            root: "/data/bitwarden".to_owned(),
        });
        assert_eq!(missing.kind, BwErrorKind::MissingBinary);
        assert_eq!(not_installed.kind, BwErrorKind::NotInstalled);
        assert_ne!(missing.message, not_installed.message);
    }

    /// 快照序列化成 probe 的形状：字段名是 camelCase（与生成物一致）。
    #[test]
    fn the_probe_snapshot_uses_the_published_field_names() {
        let value = probe(&Bitwarden::default());
        assert!(value.get("cli").is_some());
        assert!(value.get("hasSession").is_some(), "字段名要与 TS 那边一致");
    }
}
