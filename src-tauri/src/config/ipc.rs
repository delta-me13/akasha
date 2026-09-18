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
//!
//! ⚠️ 本模块同时是**数据目录从哪来**的唯一落点（下面 [`data_dir`] 的三条），
//! 而这两件事的错误处理**方向相反**：配置读不出来**降级**（可选能力），
//! 而**用户明确要的便携目录写不进去则拒绝启动**（[`require_writable`]，plan 0405）——
//! 替他决定写到哪里，正是"数据在哪"这件事最不能静默的地方。

use std::path::{Path, PathBuf};

use crate::config::{CloseBehavior, Config};
use tauri::{AppHandle, Manager, Wry};

/// 便携标记：**bin 同目录**里存在这个目录，就把数据放在那里（`docs/portable.md` §4）。
///
/// 目录本身就是标记，不需要额外再放一个标记文件：两种"其实是一个意思"的写法，
/// 只会让下一个读代码的人多问一次"这两个有什么区别"。
pub const PORTABLE_DIR: &str = "akasha-data";

/// 配置文件名（数据目录内）。
pub const FILE_NAME: &str = "config.json";

/// 判"这个目录能不能写"用的探针文件名（写完立刻删掉）。
///
/// 带点前缀是为了让它一眼就是我们的东西；用 `create`（不是 `create_new`）打开，
/// 所以上一次崩溃留下的残骸不会让 app 起不来。
const WRITE_PROBE: &str = ".akasha-writable";

/// 便携目录存在但写不进去时的**退出码**（`portable.md` §4 第 3 条）。
///
/// 单独定一个非零值是为了让它**可断言**：`2` = "我拒绝启动，因为你要的便携目录写不了"。
/// E2E 用例按这个值判定，所以别改成 1（1 是"起崩了"的通用值，分不开两件事）。
pub const EXIT_NOT_WRITABLE: i32 = 2;

/// 便携目录写不进去。
///
/// 单独一个类型是为了让"这条规则"在签名上看得见：返回裸 `std::io::Error` 的话，
/// 调用方（`lib.rs` 的启动路径）看不出失败的是哪条判据。
#[derive(Debug, thiserror::Error)]
#[error("not writable: {source}")]
pub struct NotWritable {
    #[source]
    source: std::io::Error,
}

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
///
/// ⚠️ **它也用来写**（plan 0902 起）：JSON 没有"改一个字段"这回事，写就是把整份重写，
/// 所以读与写必须共用同一个形状 —— 分成两份 struct 一定会有一份漏掉后来加的字段，
/// 而那次漏掉的后果是"用户写的配置被静默抹掉"。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    /// 只认字符串：写 `3` 或 `true` 的人想表达的不是"用默认值"，所以整份文件判为非法。
    close_behavior: Option<String>,
    /// Bitwarden 的两个轴（plan 0902）。与 `close_behavior` 并列，但**不参与 [`Config`]**：
    /// 它不是"关窗语义"那一类启动期配置，而是按需读取的使用设置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bitwarden: Option<FileBitwarden>,
}

/// `bitwarden` 那一段的两个轴（ADR-0007 D4）。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FileBitwarden {
    binary: Option<String>,
    appdata: Option<String>,
}

/// 写配置时可能出的问题。
///
/// ⚠️ **既有文件坏了就拒绝写**：整份重写意味着"读不出来"时继续写会把用户写的
/// `close_behavior` 一起抹掉。这条判据比"能不能写进去"更重要。
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("config file is not valid: {0}")]
    Malformed(String),
    #[error("{path}: {reason}")]
    Io { path: String, reason: String },
}

/// 字符串 → 二进制来源。不认识的取值取默认值（口径同 [`load`]：配置读不出来不挡任何事）。
fn binary_source(raw: Option<&str>) -> crate::bw::BinarySource {
    raw.and_then(crate::bw::BinarySource::parse)
        .unwrap_or_default()
}

/// 字符串 → 状态目录。同上。
fn appdata_mode(raw: Option<&str>) -> crate::bw::AppData {
    raw.and_then(crate::bw::AppData::parse).unwrap_or_default()
}

/// 读 Bitwarden 的两个轴。**永不失败**：文件没有 / 坏了 / 值不认识一律用默认值 + 一条日志。
///
/// 收**数据目录**而不是 `AppHandle`：`.setup()` 里数据目录已经算出来了，而命令侧要的是
/// "同一个目录"这件事 —— 两处各自再算一遍 `data_dir_of` 迟早会有一处先改。
pub fn bitwarden(dir: &Path) -> crate::bw::Settings {
    let fallback = crate::bw::Settings::default();
    let path = dir.join(FILE_NAME);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return fallback;
    };
    let Ok(file) = serde_json::from_str::<File>(&text) else {
        tracing::warn!(path = %path.display(), "bitwarden settings unreadable");
        return fallback;
    };
    let Some(settings) = file.bitwarden else {
        return fallback;
    };
    crate::bw::Settings {
        binary: binary_source(settings.binary.as_deref()),
        appdata: appdata_mode(settings.appdata.as_deref()),
    }
}

/// 写 Bitwarden 的两个轴。
///
/// 顺序：读整份 → 只换 `bitwarden` → 写临时文件 → 改名。改名是原子的，所以不存在
/// "写到一半的配置"（配置坏掉的后果是下一次启动回到默认值，而不是启动不了）。
pub fn save_bitwarden(dir: &Path, settings: crate::bw::Settings) -> Result<(), WriteError> {
    let path = dir.join(FILE_NAME);
    let mut file = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<File>(&text)
            .map_err(|err| WriteError::Malformed(err.to_string()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => File::default(),
        Err(err) => {
            return Err(WriteError::Io {
                path: path.display().to_string(),
                reason: err.to_string(),
            });
        }
    };
    file.bitwarden = Some(FileBitwarden {
        binary: Some(settings.binary.as_str().to_owned()),
        appdata: Some(settings.appdata.as_str().to_owned()),
    });

    let text = serde_json::to_string_pretty(&file)
        .map_err(|err| WriteError::Malformed(err.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text.as_bytes()).map_err(|err| WriteError::Io {
        path: temporary.display().to_string(),
        reason: err.to_string(),
    })?;
    std::fs::rename(&temporary, &path).map_err(|err| WriteError::Io {
        path: path.display().to_string(),
        reason: err.to_string(),
    })
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
        // 重连参数还没有文件形态（plan 0605 的非目标）—— 用模型里的默认值，
        // 而不是在这里另写一套数：默认值只有 `Reconnect::default()` 一处。
        reconnect: Config::default().reconnect,
        // 传输的并发上限同理（plan 0704 的非目标）。
        transfer: Config::default().transfer,
    })
}

/// 生效的重连策略（ADR-0003 D13 的参数）。
///
/// 还没登记过（`.setup()` 没跑到 / 单独用命令测）就取模型里的默认值 —— 与
/// `lifecycle` 那条降级同一条口径：配置读不到不该挡住任何东西，而这里的默认值
/// 本来就是"3 次 + 1s/2s/4s"。
pub fn reconnect(app: &tauri::AppHandle) -> crate::config::Reconnect {
    use tauri::Manager;
    app.try_state::<Config>()
        .map_or_else(crate::config::Reconnect::default, |config| config.reconnect)
}

/// 生效的传输并发上限（ADR-0006 D6 的参数）：**一个 SFTP 会话同时搬几个文件**。
///
/// 与 [`reconnect`] 同一条口径：还没登记过（`.setup()` 没跑到 / 单独用命令测）就取模型里的
/// 默认值 —— 配置读不到不该挡住任何东西，而"上限是多少"这个问题永远答得出来。
pub fn in_flight(app: &tauri::AppHandle) -> u32 {
    use tauri::Manager;
    app.try_state::<Config>()
        .map_or_else(crate::config::Transfer::default, |config| config.transfer)
        .in_flight
}

/// bin 同目录里那个便携数据目录（`portable.md` §4 第 1 条）——
/// **只在这个目录确实存在时**才是 `Some`（存在就是"要便携"的标记，见 [`PORTABLE_DIR`]）。
///
/// 抽成函数是为了让 [`data_dir`] 与 [`portable_data_dir`] 用**同一个**表达式判断：
/// 两处各写一遍 `join` + `is_dir`，迟早会有一处先改。
fn portable_dir(exe_dir: Option<&Path>) -> Option<PathBuf> {
    let dir = exe_dir?.join(PORTABLE_DIR);
    dir.is_dir().then_some(dir)
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
    portable_dir(exe_dir).or(os_dir)
}

/// 本进程可执行文件所在的目录。
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// 本进程 bin 同目录的便携数据目录（存在才有）。
///
/// 与 [`data_dir_of`] 分开是因为**启动时的可写性检查只针对便携这条**：
/// 退回 OS 目录那条路上"写不了"仍然只是降级（配置用默认值），不该拦启动。
pub fn portable_data_dir() -> Option<PathBuf> {
    portable_dir(exe_dir().as_deref())
}

/// 便携目录**可写吗** —— 判定方式是**真的写一个探针文件**，不是看 mode 位。
///
/// 为什么不用 mode 位：它看不出 ACL、只读挂载、squashfs 这类"位是好的、写就是不行"
/// 的情形，而这一条的判据恰恰是"写不进去"。
/// 代价是启动路径上多一次写盘 —— 只发生在**用户明确要便携**的那条路上，
/// 而且探针文件紧接着就删掉（删不掉不算失败：那只是残骸）。
pub fn require_writable(dir: &Path) -> Result<(), NotWritable> {
    let probe = dir.join(WRITE_PROBE);
    match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(source) => Err(NotWritable { source }),
    }
}

/// 这台机器上**生效的**数据目录（便携目录优先，见 [`data_dir`]）。
///
/// 公开它是因为数据目录不止放配置文件：加密库也落在**同一个**目录里
/// （ADR-0002 D1）—— 分两个目录的话，"搬走文件夹"就只搬走一半。
/// 谁放什么由各自的模块决定（配置在 [`FILE_NAME`]，库在 `akasha_store::vault_path`）。
pub fn data_dir_of(app: &AppHandle<Wry>) -> Option<PathBuf> {
    let os_dir = app.path().app_data_dir().ok();
    data_dir(exe_dir().as_deref(), os_dir)
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

    /// 便携目录**存在**与"算出来的路径"是同一个判断（[`portable_dir`]）——
    /// 启动时的可写性检查用的就是它，两处不能各写一遍。
    #[test]
    fn the_portable_dir_is_reported_only_when_it_exists() {
        let exe_dir = scratch("portable-presence");
        let portable = exe_dir.join(PORTABLE_DIR);

        assert_eq!(
            portable_dir(Some(&exe_dir)),
            None,
            "目录还不存在时不该说\"要便携\"——这时 app 该退回 OS 数据目录"
        );

        std::fs::create_dir_all(&portable).expect("建便携数据目录");
        assert_eq!(portable_dir(Some(&exe_dir)), Some(portable.clone()));
        assert_eq!(
            portable_dir(None),
            None,
            "连 exe 目录都取不到时没有便携目录可言"
        );

        let _ = std::fs::remove_dir_all(&exe_dir);
    }

    /// 可写的目录必须**通过**（正对照：少了它，下面那条"不可写被拒"分不清
    /// "检查在工作"与"检查把什么都拒了"）。
    #[test]
    fn a_writable_dir_passes_and_leaves_no_probe_behind() {
        let dir = scratch("writable");

        require_writable(&dir).expect("刚建的目录当然可写");

        assert_eq!(
            std::fs::read_dir(&dir).expect("读目录").count(),
            0,
            "探针文件必须删掉，别在用户的数据目录里留东西"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写不进去的目录必须被**判**出来。
    ///
    /// 造"不可写"的方式是**结构性**的：把探针指到一个普通文件底下 —— `ENOTDIR`
    /// 连 root 也绕不过去，所以这条判据与本机权限、文件系统、平台**都无关**。
    /// （用 `chmod 500` 造的话，以 root 跑或在不理会 mode 位的文件系统上就造不出来，
    /// 那条路留给 `tests/portable.rs`：它要的是**app 真的看见一个不可写目录**，
    /// 那里可以显式跳过并写明原因。）
    #[test]
    fn a_dir_that_cannot_be_written_is_refused() {
        let dir = scratch("under-a-file");
        let file = dir.join("regular-file");
        std::fs::write(&file, b"").expect("放一个普通文件");

        let err =
            require_writable(&file.join("under-a-file")).expect_err("文件底下的路径建不出探针");
        assert!(
            err.to_string().contains("not writable"),
            "错误要说清是哪条判据：{err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
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
