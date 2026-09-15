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
    /// 手上的 session key（**只在内存**）。
    session: Option<Session>,
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
    /// CLI 说得出来的状态；`bw` 自己跑不起来时为 `None`。
    pub status: Option<BwVaultStatus>,
    /// **我们手里有没有 session key**（不是 key 本身）。
    pub has_session: bool,
    /// 读状态失败的原因。
    pub problem: Option<String>,
}

/// 解析出可执行文件与它的自报信息（会起进程：`--version` 与 `--help`）。
fn inspect(paths: &Paths, settings: Settings) -> (BwCliInfo, Option<Cli>) {
    let mut info = BwCliInfo {
        binary: settings.binary.as_str().to_owned(),
        appdata: settings.appdata.as_str().to_owned(),
        program: None,
        version: None,
        variant: None,
        license_notice: None,
        problem: None,
    };
    let located = match paths.resolve(settings) {
        Ok(located) => located,
        Err(err) => {
            info.problem = Some(err.to_string());
            return (info, None);
        }
    };
    info.program = Some(located.program.display().to_string());
    let cli = Cli::new(&located);

    match cli.version() {
        Ok(version) => info.version = Some(version),
        Err(err) => {
            info.problem = Some(err.to_string());
            return (info, Some(cli));
        }
    }
    match cli.variant() {
        Ok(variant) => {
            info.variant = Some(
                match variant {
                    akasha_bw::Variant::Oss => "oss",
                    akasha_bw::Variant::Proprietary => "proprietary",
                    akasha_bw::Variant::Unknown => "unknown",
                }
                .to_owned(),
            );
            if variant.needs_license_notice() {
                info.license_notice = Some(format!(
                    "这一份是{}：它的许可证把用途限制在内部开发与测试、非生产环境。\
                     要避开这条限制，可以改用运行时下载的 OSS 那一份。",
                    variant.describe()
                ));
            }
        }
        Err(err) => info.problem = Some(err.to_string()),
    }
    (info, Some(cli))
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
            status: None,
            has_session: false,
            problem: None,
        };
    };
    let paths = Paths::new(&dir);
    let (info, cli) = inspect(&paths, settings);
    let mut problem = info.problem.clone();

    if refresh && let Some(cli) = &cli {
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
        // 动作之后重新读一次状态（ADR-0007 D10）。
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
