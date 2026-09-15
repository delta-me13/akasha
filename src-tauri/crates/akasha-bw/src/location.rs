//! 两个轴：**用哪一份 `bw`** 与 **CLI 自己的状态放哪**（ADR-0007 D4–D6）。
//!
//! | 轴 | `host`（默认） | `managed` |
//! |---|---|---|
//! | [`BinarySource`] | `PATH` 里找到的 `bw` | 数据目录 `bitwarden/bw-<版本>/` 里那一份 |
//! | [`AppData`] | 不设 `BITWARDENCLI_APPDATA_DIR`，用 CLI 自己的默认目录 | 数据目录 `bitwarden/appdata/` |
//!
//! 两个轴**不得合并成一个三档开关**：它们回答的是两个问题，合并之后就没有
//! "宿主机的 CLI 配隔离状态"这一档（ADR-0007 D4）。
//!
//! ## 为什么 `host` 那一轴上找不到 `bw` 不是"自动改用下载"
//!
//! `host` 解析不到与 `managed` 还没下载是两件不同的事，报错要说清是哪一件（ADR-0007 D5）：
//! 把前者静默变成后者，等于替用户决定"从网上取一个可执行文件回来跑"。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::BwError;

/// 可执行文件的名字。Windows 上带扩展名。
pub const EXECUTABLE: &str = if cfg!(windows) { "bw.exe" } else { "bw" };

/// 已下载版本在数据目录里的目录名前缀（`bw-<版本>`）。
const MANAGED_PREFIX: &str = "bw-";

/// 二进制从哪来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BinarySource {
    /// `PATH` 里的 `bw`（默认）。
    #[default]
    Host,
    /// 运行时下载到数据目录里的那一份。
    Managed,
}

/// CLI 自己的状态目录放哪。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppData {
    /// CLI 自己的默认目录（不设 `BITWARDENCLI_APPDATA_DIR`，默认）。
    #[default]
    Host,
    /// 数据目录下的 `bitwarden/appdata/`。
    Managed,
}

impl BinarySource {
    pub const ALL: [Self; 2] = [Self::Host, Self::Managed];

    /// 配置文件里的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Managed => "managed",
        }
    }

    /// 读配置文件里的写法。**不认识就是 `None`** —— 由调用方决定报错还是取默认值。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "host" => Some(Self::Host),
            "managed" => Some(Self::Managed),
            _ => None,
        }
    }
}

impl AppData {
    pub const ALL: [Self; 2] = [Self::Host, Self::Managed];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Managed => "managed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "host" => Some(Self::Host),
            "managed" => Some(Self::Managed),
            _ => None,
        }
    }
}

/// 两个轴的当前取值。默认两条都是 `host`（ADR-0007 D4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    pub binary: BinarySource,
    pub appdata: AppData,
}

/// 数据目录下 `bitwarden/` 这一棵子树的布局。
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// 数据目录 → `bitwarden/`。**只算路径，不建目录**（建目录是 [`Paths::resolve`] 的事）。
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join("bitwarden"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 某一个版本的落点目录：`bitwarden/bw-<版本>/`。
    pub fn version_dir(&self, version: &str) -> PathBuf {
        self.root.join(format!("{MANAGED_PREFIX}{version}"))
    }

    /// 某一个版本的可执行文件：`bitwarden/bw-<版本>/bw`。
    pub fn executable(&self, version: &str) -> PathBuf {
        self.version_dir(version).join(EXECUTABLE)
    }

    /// 隔离状态下 CLI 自己的状态目录：`bitwarden/appdata/`。
    pub fn appdata_dir(&self) -> PathBuf {
        self.root.join("appdata")
    }

    /// 已经下载好的版本，**从新到旧**。
    ///
    /// 只认"目录里真的有那个可执行文件"的那些：一个半途失败的解包会留下空目录，
    /// 而把它算进"已安装"会让下一次解析失败在一个不存在的东西上。
    pub fn installed_versions(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut found: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
            .filter_map(|name| name.strip_prefix(MANAGED_PREFIX).map(str::to_owned))
            .filter(|version| self.executable(version).is_file())
            .collect();
        // `Reverse`：从新到旧。
        found.sort_by_key(|version| std::cmp::Reverse(version_key(version)));
        found
    }

    /// 按设置解析出**这一台机器上现在该用的那一份 CLI**。
    ///
    /// `managed` 那一条会在需要时建出隔离状态目录 —— 每一次 `bw` 调用都可能写
    /// `data.json`（实测：连 `bw --version` 都会创建它），所以那个目录必须在**调用之前**存在。
    pub fn resolve(&self, settings: Settings) -> Result<Located, BwError> {
        let program = match settings.binary {
            BinarySource::Host => find_in_path(std::env::var_os("PATH").as_deref(), EXECUTABLE)
                .ok_or(BwError::MissingBinary)?,
            BinarySource::Managed => {
                let version = self
                    .installed_versions()
                    .into_iter()
                    .next()
                    .ok_or_else(|| BwError::NotInstalled {
                        root: self.root.display().to_string(),
                    })?;
                self.executable(&version)
            }
        };

        let appdata = match settings.appdata {
            AppData::Host => None,
            AppData::Managed => {
                let dir = self.appdata_dir();
                std::fs::create_dir_all(&dir).map_err(|err| BwError::io(dir.display(), err))?;
                Some(dir)
            }
        };

        Ok(Located {
            program,
            appdata,
            settings,
        })
    }
}

/// 解析结果：[`Cli`](crate::Cli) 的两个输入。
#[derive(Debug, Clone)]
pub struct Located {
    /// 要跑的那个可执行文件。
    pub program: PathBuf,
    /// `Some` = 给子进程设 `BITWARDENCLI_APPDATA_DIR`；`None` = 用 CLI 自己的默认目录。
    pub appdata: Option<PathBuf>,
    pub settings: Settings,
}

/// 在一条 `PATH` 里找一个可执行文件。
///
/// **`PATH` 是输入而不是从环境里读的**：`std::env::set_var` 在 edition 2024 里是 `unsafe`
/// （问题 #109），因此"找不到"这条判据只能靠传值来单测 —— 与 `SshAuth::agent_socket`
/// 同一条口径。
pub fn find_in_path(path_var: Option<&OsStr>, name: &str) -> Option<PathBuf> {
    let path_var = path_var?;
    std::env::split_paths(path_var)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// 版本号的大小关系：按 `.` 分段、能解析成数字的按数值比。
///
/// 上游的版本形如 `2026.8.0`（`cli-v` 前缀在 tag 上，不在资产名里）。**不引 semver 依赖**：
/// 这里只需要"从几个已下载的版本里挑最新的那一个"。
pub(crate) fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    /// 测试用的临时目录（本仓库没有 `tempfile` 依赖，与 `config.rs` 同一条口径）。
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("akasha-bw-loc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn both_axes_default_to_host() {
        let settings = Settings::default();
        assert_eq!(settings.binary, BinarySource::Host);
        assert_eq!(settings.appdata, AppData::Host);
    }

    #[test]
    fn the_two_axes_round_trip_through_their_file_form() {
        for value in BinarySource::ALL {
            assert_eq!(BinarySource::parse(value.as_str()), Some(value));
        }
        for value in AppData::ALL {
            assert_eq!(AppData::parse(value.as_str()), Some(value));
        }
        assert_eq!(BinarySource::parse("Host"), None, "取值区分大小写");
        assert_eq!(AppData::parse("isolated"), None, "不认识就是 None");
    }

    #[test]
    fn path_lookup_takes_the_first_usable_entry() {
        let dir = scratch("path-first");
        let first = dir.join("a");
        let second = dir.join("b");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let name = if cfg!(windows) { "bw.exe" } else { "bw" };
        std::fs::write(second.join(name), b"later").unwrap();

        let path = std::env::join_paths([&first, &second]).unwrap();
        assert_eq!(
            find_in_path(Some(&path), name),
            Some(second.join(name)),
            "前面那些目录里没有就往下找"
        );

        std::fs::write(first.join(name), b"earlier").unwrap();
        assert_eq!(
            find_in_path(Some(&path), name),
            Some(first.join(name)),
            "先找到的优先（PATH 的顺序就是优先级）"
        );

        assert_eq!(find_in_path(None, name), None, "连 PATH 都没有就是找不到");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_host_binary_is_its_own_error() {
        let dir = scratch("missing-host");
        let paths = Paths::new(&dir);
        let err = paths
            .resolve(Settings::default())
            .expect_err("PATH 里没有 bw（测试进程的 PATH 里确实没有）");
        // ⚠️ 这条断言依赖"测试机的 PATH 里没有 bw"。装了 bw 的机器上它会红 ——
        // 那正说明这条判据在工作（它读的确实是环境）。
        assert!(matches!(err, BwError::MissingBinary), "{err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn managed_without_any_download_says_so_instead_of_falling_back() {
        let dir = scratch("managed-empty");
        let paths = Paths::new(&dir);
        let err = paths
            .resolve(Settings {
                binary: BinarySource::Managed,
                appdata: AppData::Host,
            })
            .expect_err("还没下载过");
        assert!(matches!(err, BwError::NotInstalled { .. }), "{err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installed_versions_ignore_directories_without_the_executable() {
        let dir = scratch("installed");
        let paths = Paths::new(&dir);

        // 一个半途失败的解包：目录在，可执行文件不在。
        std::fs::create_dir_all(paths.version_dir("2026.1.0")).unwrap();
        // 两个真的装好的版本。
        for version in ["2026.8.0", "2026.10.0"] {
            std::fs::create_dir_all(paths.version_dir(version)).unwrap();
            std::fs::write(paths.executable(version), b"fake").unwrap();
        }
        // 一个名字对不上的目录。
        std::fs::create_dir_all(paths.root().join("appdata")).unwrap();

        assert_eq!(
            paths.installed_versions(),
            vec!["2026.10.0".to_owned(), "2026.8.0".to_owned()],
            "从新到旧，且空目录不算已安装"
        );
        assert_eq!(
            paths
                .resolve(Settings {
                    binary: BinarySource::Managed,
                    appdata: AppData::Host,
                })
                .unwrap()
                .program,
            paths.executable("2026.10.0"),
            "解析出来的是最新的那个版本"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_managed_appdata_directory_is_created_before_the_first_call() {
        let dir = scratch("appdata");
        let paths = Paths::new(&dir);
        std::fs::create_dir_all(paths.version_dir("2026.8.0")).unwrap();
        std::fs::write(paths.executable("2026.8.0"), b"fake").unwrap();

        let located = paths
            .resolve(Settings {
                binary: BinarySource::Managed,
                appdata: AppData::Managed,
            })
            .unwrap();

        assert_eq!(located.appdata, Some(paths.appdata_dir()));
        assert!(
            paths.appdata_dir().is_dir(),
            "每一次 bw 调用都可能写 data.json，目录必须先存在"
        );

        // 另一条轴取 host 时**不设**那个变量，也不建目录。
        let host = Paths::new(&dir);
        let other = host
            .resolve(Settings {
                binary: BinarySource::Managed,
                appdata: AppData::Host,
            })
            .unwrap();
        assert_eq!(other.appdata, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_keys_compare_numerically_not_lexically() {
        assert!(version_key("2026.10.0") > version_key("2026.8.0"));
        assert!(version_key("2026.8.0") > version_key("2025.12.9"));
        assert_eq!(
            version_key("nonsense"),
            vec![0],
            "读不出来的段当 0，不 panic"
        );
    }
}
