//! akasha 的 Bitwarden CLI 客户端：**从哪拿到 `bw`、怎么调它、它说的状态是什么**。
//!
//! 形状由 [ADR-0007](../../../../docs/adr/0007-bitwarden-cli-acquisition.md) 定：两个轴各自
//! 可切（二进制来源 × CLI 状态目录），默认都取宿主机上的那一份；运行时下载只取 OSS 变体。
//! 上游条款、命令面与实测输出在 [`docs/bitwarden.md`](../../../../docs/bitwarden.md)。
//!
//! ## 为什么不把业务逻辑写在 `src-tauri` 里
//!
//! 与本仓库其余 crate 同一条理由（`AGENTS.md` §3.1）：`src-tauri` 只做 IPC 编组。
//! 这里没有一行 Tauri 依赖，于是"下载一个 45 MB 的压缩包、判定变体、解析状态"
//! 全都能脱离 app 测试。
//!
//! ## 模块边界
//!
//! | 模块 | 管什么 |
//! |---|---|
//! | [`location`] | 两个轴的取值与解析：宿主机 `PATH` 里的 `bw` / 数据目录里已下载的那一份；CLI 的状态目录 |
//! | [`acquire`] | 运行时下载：解析最新 `cli-v*` 版本 → 取资产 → 算 SHA-256 → 解包 → 落盘 |
//! | [`cli`] | 一次命令调用的形状：参数、子进程环境、超时、输出捕获与失败分类 |
//! | [`status`] | 把 `bw status --raw` 的 JSON 翻成本 crate 的类型 |
//! | [`items`] | 把 `bw list items --raw` 的输出里**只**那几条 SSH key 条目挑出来（plan 0903） |
//! | [`session`] | session key 的落点：**只在内存里，且住在受保护页**（ADR-0007 D7） |
//!
//! ## 这一版刻意不做的
//!
//! - 不实现 vault 密码学（`docs/scope.md` §10 的非目标）。
//! - 不写回上游：本 crate 只有读与登录/锁定这几条命令，没有 `create` / `edit`。
//! - **不并发**：一次一个 `bw` 进程。CLI 自己在 `data.json` 上没有任何互斥，
//!   同时跑两条命令会互相覆盖状态 —— 串行化由 app 侧那把锁保证。

pub mod acquire;
pub mod cli;
pub mod error;
pub mod items;
pub mod location;
pub mod session;
pub mod status;
pub mod testing;
pub mod variant;

pub use acquire::{Http, Installed, Sources};
pub use cli::{Cli, Output, Timeouts};
pub use error::BwError;
pub use items::{Inventory, SshKeyItem};
pub use location::{AppData, BinarySource, Located, Paths, Settings};
pub use session::Session;
pub use status::{State, Status};
pub use variant::Variant;

/// 一次命令默认等多久。
///
/// 分档是因为它们的量级差着两个数量级：纯本地命令是毫秒级，而 `bw` 本身是个约 140 MB 的
/// Node SEA，启动就要几百毫秒；`login` / `unlock` 还要联网并跑一次 KDF。
pub mod timeout {
    use std::time::Duration;

    /// `--version` / `--help` / `config server` 这类纯本地命令。
    pub const LOCAL: Duration = Duration::from_secs(20);
    /// `status`：不联网，但要把 CLI 的状态文件读起来。
    pub const STATUS: Duration = Duration::from_secs(20);
    /// `login` / `unlock` / `sync`：联网 + KDF。
    pub const NETWORK: Duration = Duration::from_secs(120);
    /// 下载一个约 45 MB 的资产（含上游的跳转）。
    pub const DOWNLOAD: Duration = Duration::from_secs(600);
}
