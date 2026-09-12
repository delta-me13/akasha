//! 库的**落点与状态**（plan 0403）：app 侧只接这两件。
//!
//! ## 为什么只接这两件
//!
//! `akasha-store` 里已经有四套池的 CRUD，但"**谁持有解好的 `Connection`、口令从哪来、
//! 什么时候抹掉**"是一个**生命周期决定**：它自己的判据（锁定之后要抹什么、单实例下第二个
//! 进程怎么办、可搬迁性怎么验）还没定 —— 那是 plan 0407。先接"库在哪、建过没有"是因为
//! 这两件是纯函数加一次 `stat`，不需要先回答上面那些问题，却能立刻带来两样东西：
//!
//! 1. `AGENTS.md` §7 那条"真实路径走通"第一次有对象：E2E 能在**真 app** 上 invoke 它，
//!    而不是只在单测里成立；
//! 2. P2 的落点（数据目录相对可执行文件推导）第一次**走通真路径** ——
//!    `config::data_dir_of` 的便携分支此前只有单测看过。
//!
//! ## 没有注册 probe
//!
//! `lifecycle` / `single_instance` 那些 probe 的存在理由是"这个状态**没有别的观察口**，
//! 不看日志就只能猜"。这里不同：状态本身就是一条命令（前端要用它），再挂一个 probe
//! 只是把同一个函数读第二遍 —— **多一条观察路径，但不多一点信息**。
//! `AGENTS.md` §7 要的"前后端状态一致"由 E2E 自己 `stat` 同一个路径来对账
//! （见 `tests/vault_status.rs`）。
//!
//! ## 这条命令不需要口令
//!
//! 连 `user_version` 都在加密的第一页里：不开库就读不到。所以能不开库说出来的只有
//! "文件在不在、是不是空的"，而**"有文件但打不开"不是状态**（那是 `open` 的错误：
//! 口令错或不是个库）—— 所以这里没有第四个状态，也没有"错误口令"这条分支。

use akasha_store::{VaultState, vault_path};
use tauri::{AppHandle, Wry};

use crate::config;

/// 库文件的状态。**与 `akasha_store::VaultState` 分开**：这是 IPC 类型（要生成 TS），
/// 而存储 crate 不该为了生成 TS 去依赖被 app 钉住版本的 `specta`。
///
/// 两边的映射写成穷尽 `match`（下面的 `From`）：存储层加了状态，**这里编译不过** ——
/// 而不是悄悄少一个分支，让前端在一个它不认识的值上做默认动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// 文件不存在：还没有建过。
    Missing,
    /// 文件在但是 **0 字节**：还没有密钥落在那里（这种文件用什么口令都能"打开"，
    /// 见 ADR-0002 §7）。与 `Missing` 分开是因为用户的下一步动作不同。
    Empty,
    /// 有内容。能不能打开是解锁那一步的事（plan 0407）。
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
}

/// 取库的状态时可能出的错。
///
/// 与 `IpcError` 分开：域不同，前端能据此做的动作也不同（这里是"环境没准备好"，
/// 而不是"某个会话坏了"）。
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
}

/// 库在哪、建过没有。**只读元数据**：不打开库、不创建目录、不要口令。
///
/// 它**不记日志**：那是一个查询（前端可能反复调），而日志留给状态**变化** ——
/// 这里没有变化，只有"此刻是什么"。
#[tauri::command]
#[specta::specta]
pub fn vault_status(app: AppHandle<Wry>) -> Result<VaultStatus, VaultError> {
    let dir = config::data_dir_of(&app).ok_or(VaultError::NoDataDir)?;
    let path = vault_path(&dir);
    let state = akasha_store::vault_state(&path).map_err(|err| VaultError::Io {
        message: err.to_string(),
    })?;

    Ok(VaultStatus {
        path: path.display().to_string(),
        state: state.into(),
    })
}
