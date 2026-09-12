//! 配置的**文件载体**（plan 0303）：把数据目录里的一个文件变成 [`Config`]。
//!
//! 为什么现在只有"读"：本步只解决"关窗语义要能配"。**不建数据库**（阶段 4）、
//! **不做配置 UI**（`scope.md` §5.2：先做后端）、**也不在启动时写盘** ——
//! 启动路径上每多一次写就多一条"写不了就起不来"（`AGENTS.md` §3.3），
//! 而文件缺失本来就是一个正常状态：用默认值。
//!
//! ⚠️ **读配置不挡启动**：文件读不到、写坏了、值不认识 —— 一律用默认值 + 一条日志。
//! 配置是可选能力，不是启动前提。
//!
//! 格式（`<数据目录>/config.json`，JSON 没有注释，所以这里把取值写全）：
//!
//! ```json
//! { "close_behavior": "tray" }
//! ```
//!
//! * `close_behavior`：`"tray"`（默认，关窗 = 收托盘）或 `"exit"`（关窗 = 直接退出）；
//! * 文件不存在 / `{}` → 全部取默认值；
//! * **多余的字段是错误**（`deny_unknown_fields`）：`close_behaviour` 这种拼错
//!   若被静默忽略，用户会以为配置生效了 —— 那是最难查的一类"配置不生效"。

use std::path::{Path, PathBuf};

use akasha_core::{CloseBehavior, Config};
use tauri::{AppHandle, Manager, Wry};

/// 便携标记：**bin 同目录**里存在这个目录，就把数据放在那里（`docs/portable.md` §4）。
///
/// 目录本身就是标记，不需要额外再放一个标记文件：两种"其实是一个意思"的写法，
/// 只会让下一个读代码的人多问一次"这两个有什么区别"。
pub const PORTABLE_DIR: &str = "akasha-data";

/// 配置文件名（数据目录内）。
pub const FILE_NAME: &str = "config.json";

/// 配置读不出来的两种情形。
///
/// 分两支是因为**用户的下一步动作不同**：`Malformed` 要去改 JSON（或删掉那个字段），
/// `UnknownBehavior` 只要把值改成两个合法取值之一。都写成一个"配置坏了"，
/// 日志就答不出"我该改什么"。
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// 不是合法 JSON / 字段类型不符 / 出现未知字段（`serde_json` 的说明里带行列号）。
    #[error("config file is not valid: {0}")]
    Malformed(#[source] serde_json::Error),
    /// `close_behavior` 的值不认识。
    #[error("unknown close_behavior value {0:?} (expected \"tray\" or \"exit\")")]
    UnknownBehavior(String),
}

/// 配置文件的样子 —— **只描述文件**，不描述生效的配置。
///
/// 与 [`Config`] 分开：前者允许"字段缺失"（缺失是正常的，用默认值），后者永远有值。
/// 合成一个就得给每个字段编一个"未设置"的哨兵值，而哨兵值迟早会被当成真值用。
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    /// 只认字符串：写 `3` 或 `true` 的人想表达的不是"用默认值"，所以整份文件判为非法。
    close_behavior: Option<String>,
}

/// 解析配置文本。**纯函数** —— 路径与日志都在调用方，所以"坏文件怎么办"这条判据能单测。
///
/// 值不认识时返回错误而**不是**默认值：默认值由调用方在记完日志之后给，
/// 这样"配置没生效"与"配置坏了"在日志里是两件事。
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let file: File = serde_json::from_str(text).map_err(ConfigError::Malformed)?;
    let Some(raw) = file.close_behavior else {
        return Ok(Config::default());
    };
    let Some(behavior) = CloseBehavior::parse(&raw) else {
        return Err(ConfigError::UnknownBehavior(raw));
    };
    Ok(Config {
        close_behavior: behavior,
    })
}

/// 数据目录（`docs/portable.md` §4 的三条，按顺序）：
///
/// 1. bin 同目录存在 [`PORTABLE_DIR`] → 用它（用户明确要便携）；
/// 2. 否则退回 OS 标准数据目录（`app_data_dir`，平台各自解析）；
/// 3. 两个都没有 → `None`。
///
/// ⚠️ **这里不创建任何目录**。开发构建的 bin 目录是可写的，自动创建会让"便携模式"
/// 在没人要求的时候生效 —— 而 `cargo clean` / 换个构建目录之后数据又凭空消失。
/// 埋这种地雷换来的"少一步"不值得。
///
/// 拆成纯函数（输入是 exe 目录与 OS 目录）是为了让第 1 条能被单测钉住：
/// 便携分支是"搬走文件夹数据还在"（P2）的实现处，它不该只靠手测。
fn data_dir(exe_dir: Option<&Path>, os_dir: Option<PathBuf>) -> Option<PathBuf> {
    match exe_dir.map(|dir| dir.join(PORTABLE_DIR)) {
        Some(portable) if portable.is_dir() => Some(portable),
        _ => os_dir,
    }
}

/// 这台机器上**生效的**数据目录（便携目录优先，见 [`data_dir`]）。
///
/// 公开它是因为数据目录不止放配置文件：加密库也落在**同一个**目录里
/// （ADR-0002 D1）—— 分两个目录的话，"搬走文件夹"就只搬走一半。
/// 谁放什么由各自的模块决定（配置在 [`FILE_NAME`]，库在 `akasha_store::vault_path`）。
pub fn data_dir_of(app: &AppHandle<Wry>) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let os_dir = app.path().app_data_dir().ok();
    data_dir(exe_dir.as_deref(), os_dir)
}

/// 数据目录里配置文件的路径。
fn config_path(app: &AppHandle<Wry>) -> Option<PathBuf> {
    data_dir_of(app).map(|dir| dir.join(FILE_NAME))
}

/// 读配置。**永不失败**：任何问题都落回默认值，并留下一条能定位原因的日志。
///
/// 三条日志对应三种"用户该做什么"：文件没有（用默认值即可）、文件坏了（去改）、
/// 连路径都取不到（环境问题，看 OS 数据目录那条线是否可用）。
pub fn load(app: &AppHandle<Wry>) -> Config {
    let fallback = Config::default();
    let Some(path) = config_path(app) else {
        // 值不撒谎：路径拿不到就不写 `path` 字段（`docs/logging.md`），
        // 但**生效的取值照写** —— 那一条是确定的。
        tracing::warn!(
            close_behavior = fallback.close_behavior.as_str(),
            "config path unavailable"
        );
        return fallback;
    };

    match std::fs::read_to_string(&path) {
        Ok(text) => match parse(&text) {
            Ok(config) => {
                tracing::info!(
                    close_behavior = config.close_behavior.as_str(),
                    path = %path.display(),
                    "config loaded"
                );
                config
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    close_behavior = fallback.close_behavior.as_str(),
                    path = %path.display(),
                    "config invalid"
                );
                fallback
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // 大多数机器上就是这一条：没有配置文件 = 默认行为。级别是 info 而不是 warn，
            // 因为它同时回答了"配置文件该放哪"——这是用户唯一能看到这个路径的地方。
            tracing::info!(
                close_behavior = fallback.close_behavior.as_str(),
                path = %path.display(),
                "config not found"
            );
            fallback
        }
        Err(err) => {
            tracing::warn!(
                %err,
                close_behavior = fallback.close_behavior.as_str(),
                path = %path.display(),
                "config unreadable"
            );
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用的临时目录。本仓库没有 `tempfile` 依赖，所以名字里带上 pid 以免撞车，
    /// 用完各自删掉（临时目录在沙箱里也是可写的）。
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("akasha-config-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn empty_object_means_all_defaults() {
        assert_eq!(parse("{}").expect("空对象是合法的"), Config::default());
    }

    #[test]
    fn close_behavior_is_read_from_the_file() {
        for behavior in CloseBehavior::ALL {
            let text = format!("{{\"close_behavior\":\"{}\"}}", behavior.as_str());
            assert_eq!(
                parse(&text).expect("合法取值").close_behavior,
                behavior,
                "文件里写的取值必须真的被读到"
            );
        }
    }

    #[test]
    fn unknown_value_is_an_error_not_a_default() {
        let err = parse(r#"{"close_behavior":"exitt"}"#).expect_err("不认识的值要报错");
        assert!(
            matches!(&err, ConfigError::UnknownBehavior(value) if value == "exitt"),
            "错误里要带着那个值，否则日志答不出该改什么：{err:?}"
        );
    }

    #[test]
    fn broken_json_is_an_error() {
        assert!(matches!(
            parse("{\"close_behavior\":"),
            Err(ConfigError::Malformed(_))
        ));
    }

    #[test]
    fn a_mistyped_field_is_an_error_not_silence() {
        // 拼错字段名是最难查的一类"配置不生效"：静默忽略会让人以为配置已经生效。
        assert!(matches!(
            parse(r#"{"close_behaviour":"exit"}"#),
            Err(ConfigError::Malformed(_))
        ));
    }

    #[test]
    fn a_wrong_type_is_an_error() {
        assert!(matches!(
            parse(r#"{"close_behavior":3}"#),
            Err(ConfigError::Malformed(_))
        ));
    }

    #[test]
    fn the_portable_dir_next_to_the_binary_wins() {
        let exe_dir = scratch("portable");
        let portable = exe_dir.join(PORTABLE_DIR);
        std::fs::create_dir_all(&portable).expect("建便携数据目录");

        assert_eq!(
            data_dir(Some(&exe_dir), Some(PathBuf::from("/os-data"))),
            Some(portable),
            "bin 同目录有 akasha-data/ 就该用它 —— 这是可搬迁性（P2）的落点"
        );

        let _ = std::fs::remove_dir_all(&exe_dir);
    }

    #[test]
    fn without_the_portable_dir_the_os_dir_is_used() {
        let exe_dir = scratch("no-portable");
        let os_dir = PathBuf::from("/os-data");

        assert_eq!(data_dir(Some(&exe_dir), Some(os_dir.clone())), Some(os_dir));

        let _ = std::fs::remove_dir_all(&exe_dir);
    }

    #[test]
    fn no_directory_at_all_is_none() {
        assert_eq!(data_dir(None, None), None);
    }

    #[test]
    fn the_vault_and_the_config_file_share_a_directory() {
        // ADR-0002 D1：库与 `config.json` 在**同一个**数据目录里 —— 搬迁时它们要么
        // 一起走、要么一起留。这条判据的机器可查部分就是"父目录相同"。
        let exe_dir = scratch("shared-dir");
        let portable = exe_dir.join(PORTABLE_DIR);
        std::fs::create_dir_all(&portable).expect("建便携数据目录");

        let dir = data_dir(Some(&exe_dir), None).expect("便携目录生效");
        let config = dir.join(FILE_NAME);
        let vault = akasha_store::vault_path(&dir);

        assert_eq!(config.parent(), vault.parent());
        assert_eq!(
            vault.file_name().and_then(|name| name.to_str()),
            Some(akasha_store::STORE_FILE_NAME)
        );

        let _ = std::fs::remove_dir_all(&exe_dir);
    }
}
