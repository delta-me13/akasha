//! 运行时下载（ADR-0007 D1–D3、D6）。
//!
//! ## 流程
//!
//! 1. 解析**最新**的 `cli-v*` 版本（GitHub releases 列表，跳过 draft 与 prerelease）；
//! 2. 按平台 / 架构拼出资产名 `bw-oss-<os>[-<arch>]-<版本>.zip`；
//! 3. 下载 → 算 SHA-256 → 解包（只取那一个可执行文件）→ 落进
//!    `<数据目录>/bitwarden/bw-<版本>/`；
//! 4. 清掉更早的版本目录。
//!
//! ## 只取 OSS 变体
//!
//! 上游每个 bundle 都有 OSS 与非 OSS 两版，**非 OSS 是各分发平台的默认包**。
//! 专有变体的许可证（2.1）把用途限制在内部开发与测试、非生产环境 —— 所以本模块
//! **只拼 `bw-oss-` 开头的资产名**，没有"下载哪一版"这个开关（ADR-0007 D1）。
//!
//! ## 校验到哪一步：只到 HTTPS
//!
//! ⚠️ 上游自 `cli-v2025.6.0` 起**不再发布 SHA-256 文件**（实测，`docs/bitwarden.md` §2），
//! 所以这里做的不是"按上游公布的哈希校验"，而是：
//!
//! - 走 HTTPS、资产 URL 里带版本号（上游自己的 release）；
//! - 算出下载物的 SHA-256 并**交给调用方**（[`Installed::sha256`]），由它记进日志与界面。
//!
//! **不得**把这一步说成"已校验"。将来上游若恢复发布校验文件，它接在这里。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::bw::error::BwError;
use crate::bw::location::{EXECUTABLE, Paths, version_key};

/// 上游 release 列表的地址（可用 [`Sources`] 换掉，测试指向进程内的假上游）。
pub const RELEASES_API: &str = "https://api.github.com/repos/bitwarden/clients/releases";

/// 上游资产的下载根：`<根>/cli-v<版本>/<资产名>`。
pub const DOWNLOAD_BASE: &str = "https://github.com/bitwarden/clients/releases/download";

/// 一次下载最多收多少字节。资产约 45 MB；这个上限只是"别被一个坏服务器吃光内存"。
const DOWNLOAD_LIMIT: u64 = 512 * 1024 * 1024;

/// release 列表响应的上限（几十条 JSON，远小于它）。
const LIST_LIMIT: u64 = 8 * 1024 * 1024;

/// 两个可注入的地址。**默认值就是上游**；测试把它指向进程内的假上游。
#[derive(Debug, Clone)]
pub struct Sources {
    pub releases: String,
    pub download: String,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            releases: RELEASES_API.to_owned(),
            download: DOWNLOAD_BASE.to_owned(),
        }
    }
}

/// 一台目标机器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Macos,
    Windows,
}

/// 一个目标架构。上游只有 x64 与 arm64。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Platform {
    /// 资产名里的那一段。
    fn asset_part(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }
}

/// 这台机器是哪个平台 / 架构。
pub fn host_target() -> Result<(Platform, Arch), BwError> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => Arch::X86_64,
        "aarch64" => Arch::Aarch64,
        other => {
            return Err(BwError::UnsupportedTarget {
                platform: std::env::consts::OS.to_owned(),
                arch: other.to_owned(),
            });
        }
    };
    let platform = if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        return Err(BwError::UnsupportedTarget {
            platform: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        });
    };
    Ok((platform, arch))
}

/// 资产名。**只有 OSS 这一种拼法**（见模块文档）。
///
/// 实测（`cli-v2026.8.0` 的资产表）：`bw-oss-linux-<v>.zip` / `bw-oss-linux-arm64-<v>.zip` /
/// `bw-oss-macos-<v>.zip` / `bw-oss-macos-arm64-<v>.zip` / `bw-oss-windows-<v>.zip` ——
/// 那一版**没有** Windows 的 arm64 资产，因此它是 [`BwError::UnsupportedTarget`]。
pub fn asset_name(version: &str, platform: Platform, arch: Arch) -> Result<String, BwError> {
    match (platform, arch) {
        // 实测：那一版没有 Windows 的 arm64 资产。
        (Platform::Windows, Arch::Aarch64) => Err(BwError::UnsupportedTarget {
            platform: "windows".to_owned(),
            arch: "aarch64".to_owned(),
        }),
        (platform, Arch::X86_64) => Ok(format!("bw-oss-{}-{version}.zip", platform.asset_part())),
        (platform, Arch::Aarch64) => Ok(format!(
            "bw-oss-{}-arm64-{version}.zip",
            platform.asset_part()
        )),
    }
}

/// 下过什么：版本、可执行文件在哪、下载物的 SHA-256。
#[derive(Debug, Clone)]
pub struct Installed {
    pub version: String,
    pub path: PathBuf,
    /// 下载物的 SHA-256（十六进制）。**是记录，不是校验结论**（见模块文档）。
    pub sha256: String,
}

/// 一个 HTTP 客户端。阻塞式：这条路径本来就是"下载一个 45 MB 的文件再解包"，
/// 而它跑在调用方给的阻塞线程上（与 SSH 建链同一条口径）。
pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub fn new(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            // GitHub 的 API 对没有 User-Agent 的请求回 403。
            .user_agent(concat!("akasha/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    pub(crate) fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, BwError> {
        let mut response = self.agent.get(url).call().map_err(|err| net(url, &err))?;
        response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|err| net(url, &err))
    }
}

fn net(url: &str, err: &ureq::Error) -> BwError {
    BwError::Network {
        message: format!("{url}：{err}"),
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// 上游最新的 CLI 版本（不带 `cli-v` 前缀）。
///
/// 上游的 release 列表里混着 `web-v*` / `desktop-v*` / `browser-v*`，所以不能取
/// "第一条"，只能按 tag 前缀挑。列表默认按创建时间倒序，但**排序仍在这里做一次**：
/// 指望服务端的顺序会在某次翻页或排序变化时静默取错版本。
pub fn latest_version(http: &Http, sources: &Sources) -> Result<String, BwError> {
    let url = format!("{}?per_page=30", sources.releases.trim_end_matches('/'));
    let body = http.get(&url, LIST_LIMIT)?;
    let text = String::from_utf8_lossy(&body);
    let releases: Vec<Release> = serde_json::from_str(&text).map_err(|err| BwError::NoRelease {
        reason: format!("release 列表读不出来：{err}"),
    })?;

    let mut versions: Vec<String> = releases
        .iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| release.tag_name.strip_prefix("cli-v").map(str::to_owned))
        .collect();
    // 从新到旧：`Reverse` 是因为 `version_key` 给的是"越大越新"。
    versions.sort_by_key(|version| std::cmp::Reverse(version_key(version)));
    versions
        .into_iter()
        .next()
        .ok_or_else(|| BwError::NoRelease {
            reason: "最近 30 条 release 里没有 cli-v* 标签".to_owned(),
        })
}

/// 下载并安装**指定的**版本（测试与"重装同一个版本"用它）。
pub fn install_version(
    http: &Http,
    sources: &Sources,
    paths: &Paths,
    version: &str,
) -> Result<Installed, BwError> {
    let (platform, arch) = host_target()?;
    let asset = asset_name(version, platform, arch)?;
    let url = format!(
        "{}/cli-v{version}/{asset}",
        sources.download.trim_end_matches('/')
    );
    let bytes = http.get(&url, DOWNLOAD_LIMIT)?;
    let sha256 = hex(&Sha256::digest(&bytes));

    let dir = paths.version_dir(version);
    std::fs::create_dir_all(&dir).map_err(|err| BwError::io(dir.display(), err))?;

    // 先落压缩包、再解包：`ZipArchive` 要 `Read + Seek`，而 45 MB 直接摊在内存里
    // 没有必要（也别把 `Cursor` 与文件两条读路径各写一遍）。
    let archive_path = dir.join("asset.zip");
    std::fs::write(&archive_path, &bytes)
        .map_err(|err| BwError::io(archive_path.display(), err))?;
    let unpacked = unpack(&archive_path, &dir)?;
    let _ = std::fs::remove_file(&archive_path);

    // 装成功了才清旧的：失败时上一个版本还在，"升级失败"不会让 CLI 彻底消失。
    for old in paths.installed_versions() {
        if old != version {
            let stale = paths.version_dir(&old);
            if let Err(err) = std::fs::remove_dir_all(&stale) {
                tracing::warn!(path = %stale.display(), %err, "stale bitwarden cli not removed");
            }
        }
    }

    Ok(Installed {
        version: version.to_owned(),
        path: unpacked,
        sha256,
    })
}

/// 解析最新版本并安装它。
pub fn install(http: &Http, sources: &Sources, paths: &Paths) -> Result<Installed, BwError> {
    let version = latest_version(http, sources)?;
    install_version(http, sources, paths, &version)
}

/// 从压缩包里取出那一个可执行文件，落进 `dir`，返回它的路径。
///
/// 落点是**临时名 + 改名**：`bw` 可能正在被这个进程自己跑着（`--version` 刚跑过），
/// 而"直接往最终路径上写"会让另一次调用读到半个文件。
fn unpack(archive_path: &Path, dir: &Path) -> Result<PathBuf, BwError> {
    let file = std::fs::File::open(archive_path)
        .map_err(|err| BwError::io(archive_path.display(), err))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| BwError::Archive {
        message: format!("{}：{err}", archive_path.display()),
    })?;

    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    let index = names
        .iter()
        .position(|name| {
            Path::new(name)
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .is_some_and(|file_name| file_name == EXECUTABLE)
        })
        .ok_or_else(|| BwError::Archive {
            // 不猜：把包里的名字列出来，形状变了要能一眼看出来。
            message: format!("里面没有 {EXECUTABLE}；包里有：{}", names.join(", ")),
        })?;

    let mut entry = archive.by_index(index).map_err(|err| BwError::Archive {
        message: format!("第 {index} 个条目读不出来：{err}"),
    })?;

    let pending = dir.join(format!(".{EXECUTABLE}.pending"));
    {
        let mut out =
            std::fs::File::create(&pending).map_err(|err| BwError::io(pending.display(), err))?;
        std::io::copy(&mut entry, &mut out).map_err(|err| BwError::io(pending.display(), err))?;
    }

    // 可执行位：zip 不带 POSIX 权限位（上游那份也没有），所以在这里显式给。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&pending, std::fs::Permissions::from_mode(0o755))
            .map_err(|err| BwError::io(pending.display(), err))?;
    }

    let final_path = dir.join(EXECUTABLE);
    std::fs::rename(&pending, &final_path).map_err(|err| BwError::io(final_path.display(), err))?;
    Ok(final_path)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // `write!` 到 `String` 不会失败；失败了也没有可用的降级动作。
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;
    use crate::bw::testing::Stub;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bw-acq-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_the_oss_assets_are_ever_named() {
        assert_eq!(
            asset_name("2026.8.0", Platform::Linux, Arch::X86_64).unwrap(),
            "bw-oss-linux-2026.8.0.zip"
        );
        assert_eq!(
            asset_name("2026.8.0", Platform::Linux, Arch::Aarch64).unwrap(),
            "bw-oss-linux-arm64-2026.8.0.zip"
        );
        assert_eq!(
            asset_name("2026.8.0", Platform::Macos, Arch::Aarch64).unwrap(),
            "bw-oss-macos-arm64-2026.8.0.zip"
        );
        assert_eq!(
            asset_name("2026.8.0", Platform::Windows, Arch::X86_64).unwrap(),
            "bw-oss-windows-2026.8.0.zip"
        );
        // 实测：那一版没有 Windows 的 arm64 资产。
        assert!(matches!(
            asset_name("2026.8.0", Platform::Windows, Arch::Aarch64),
            Err(BwError::UnsupportedTarget { .. })
        ));
    }

    #[test]
    fn the_host_target_is_one_we_support() {
        let (platform, arch) = host_target().expect("开发机是上游支持的平台");
        assert!(asset_name("2026.8.0", platform, arch).is_ok());
    }

    /// 端到端（进程内假上游）：解析版本 → 下载 → 哈希 → 解包 → 落盘 → 清旧的。
    #[test]
    fn install_takes_the_newest_release_and_leaves_one_working_binary() {
        let stub = Stub::start_all(&["2026.1.0", "2026.10.0"]);
        let dir = scratch("install");
        let paths = Paths::new(&dir);
        let http = Http::new(Duration::from_secs(10));

        let installed = install(&http, &stub.sources(), &paths).unwrap();

        assert_eq!(installed.version, "2026.10.0", "取的是最新的那一条");
        assert_eq!(installed.path, paths.executable("2026.10.0"));
        assert!(installed.path.is_file());
        assert_eq!(
            installed.sha256,
            hex(&Sha256::digest(stub.asset_bytes())),
            "SHA-256 报的是这次下载下来的那些字节"
        );

        let content = std::fs::read(&installed.path).unwrap();
        assert_eq!(
            content,
            stub.binary_bytes(),
            "解出来的是包里的那个可执行文件"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&installed.path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "要有可执行位（zip 里没有它）");
        }

        assert_eq!(stub.request_count(), 2, "版本解析 + 下载 = 两次请求");

        // 再装一次旧的版本：新的那个目录应当被清掉。
        let older = install_version(&http, &stub.sources(), &paths, "2026.1.0").unwrap();
        assert_eq!(older.version, "2026.1.0");
        assert_eq!(
            paths.installed_versions(),
            vec!["2026.1.0".to_owned()],
            "只留一个版本，免得解析时在几个目录之间挑"
        );
        assert_eq!(stub.request_count(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 反例：压缩包里没有那个可执行文件时**不许**猜一个出来。
    #[test]
    fn an_archive_without_the_executable_is_refused_with_its_contents() {
        let stub = Stub::start_with("2026.10.0", &[("README.md", b"hello".to_vec())]);
        let dir = scratch("no-executable");
        let paths = Paths::new(&dir);
        let http = Http::new(Duration::from_secs(10));

        let err = install_version(&http, &stub.sources(), &paths, "2026.10.0").unwrap_err();
        match err {
            BwError::Archive { message } => {
                assert!(message.contains("README.md"), "要说清包里有什么：{message}");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            !paths.executable("2026.10.0").exists(),
            "失败不许留下一个看似装好的版本目录"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 反例：列表里没有 `cli-v*` 时不说"最新版是空字符串"。
    #[test]
    fn a_list_without_cli_releases_is_an_error() {
        let stub = Stub::start(&["web-v2026.8.0", "desktop-v2026.8.0"], "2026.8.0");
        let http = Http::new(Duration::from_secs(10));
        let err = latest_version(&http, &stub.sources()).unwrap_err();
        assert!(matches!(err, BwError::NoRelease { .. }), "{err:?}");
    }

    /// 反例：draft / prerelease 不参与。
    #[test]
    fn drafts_and_prereleases_are_skipped() {
        let stub = Stub::start_with_meta(
            &[
                ("cli-v2027.1.0", true, false),
                ("cli-v2026.12.0", false, true),
                ("cli-v2026.8.0", false, false),
            ],
            "2026.8.0",
        );
        let http = Http::new(Duration::from_secs(10));
        assert_eq!(latest_version(&http, &stub.sources()).unwrap(), "2026.8.0");
    }

    /// 反例：服务器 404（版本名写错 / 上游删了资产）不能变成"装了个空的"。
    #[test]
    fn a_missing_asset_is_a_network_error_not_an_empty_install() {
        let stub = Stub::start(&["cli-v2026.8.0"], "2026.8.0");
        let dir = scratch("404");
        let paths = Paths::new(&dir);
        let http = Http::new(Duration::from_secs(10));

        let err = install_version(&http, &stub.sources(), &paths, "1999.1.0").unwrap_err();
        assert!(matches!(err, BwError::Network { .. }), "{err:?}");
        assert!(paths.installed_versions().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
