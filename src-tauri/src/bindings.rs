//! IPC 类型边界的**唯一真相源**（`AGENTS.md` §5）。
//!
//! Rust 这边声明命令，TS 那边由生成器产出 `src/ipc/bindings.ts` —— 方向永远只有一条：
//! **Rust → TS**。前端手写第二份签名，就是从这里开始漂移的。
//!
//! 三个使用点，改命令时都要跟着走：
//!
//! 1. [`builder`] 里登记命令（漏登记 = 前端调不到，且**没有编译错误**）；
//! 2. `just gen-types`（`src/bin/gen-types.rs`）重跑生成物并提交差异；
//! 3. `just gen-types-check` 在门禁里比对生成物是否已提交（防"改了 Rust 忘了生成"）。

use tauri_specta::{Builder, collect_commands, collect_events};

/// 收集全部 command / event 的生成器。
///
/// 同一份 builder 既给 app 当 `invoke_handler`，也给 `just gen-types` 导出 TS ——
/// **只有一份清单**，所以生成物与运行期分发不可能对不上。
///
/// ⚠️ 事件比命令多一步：`Builder::mount_events` 必须在 `.setup()` 里调用（见 `lib.rs`），
/// 否则**发事件时会 panic**（`EventRegistry not found`）—— 命令没有这个问题。
pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            crate::greet,
            crate::vault::vault_status,
            crate::vault::vault_unlock,
            crate::vault::vault_lock,
            // 池的只读读取（plan 0504）：界面据此列出主机。
            crate::pools::vault_hosts,
            // 池的**第一条写路径**（plan 0506）：从 `~/.ssh/config` 导入。
            // ⚠️ 它同步就行：读一个小文件 + 一次事务都在毫秒级，而这条命令**不握手**
            //（会阻塞几秒的那种活儿在 `open_ssh_session` 那条 async 命令上）。
            crate::pools::import_ssh_config,
            // 转发规则池的只读读取（plan 0601）：界面据此列出"有哪些隧道可以打开"。
            crate::pools::vault_forwards,
            // 串口配置池的只读读取（plan 1101）：界面据此列出"有哪些串口可以打开"。
            crate::serial::vault_serials,
            // 本机端口的枚举（plan 1102）。**同步**：一次系统调用，与 `open_serial_session` 同档。
            // ⚠️ 它列出来的端口不保证打得开（问题 #150）—— 它只是一份"系统认为有哪些"的清单。
            crate::serial::serial_ports,
            // 串口会话（plan 1101）。**同步**：打开一个本地设备没有握手，而参数在碰设备
            // 之前就校验完了 —— 与 `open_session`（本地 PTY）同档，不是与 `open_ssh_session` 同档。
            crate::serial::open_serial_session,
            crate::session::open_session,
            // SSH 会话（plan 0504）。⚠️ 它**必须**留着 async：命令体里有一次会阻塞几秒的
            // 握手（最长 `connect_timeout`），而同步命令跑在处理 IPC 请求的那条线程上 ——
            // 挡住它就等于挡住全部 IPC，包括用户回答问题要用的那三条。
            crate::ssh::ipc::open_ssh_session,
            crate::session::write_session,
            crate::session::resize_session,
            crate::session::close_session,
            // 提问往返（plan 0504）：后端问 → 前端答。
            crate::prompt::ssh_prompt_credential,
            crate::prompt::ssh_prompt_host_key,
            crate::prompt::ssh_prompt_cancel,
            // 隧道（plan 0601）。⚠️ `tunnel_open` / `tunnel_retry` 必须是 async：
            // 命令体里有一次会阻塞几秒的握手（同 `open_ssh_session` 的理由）；
            // `tunnel_stop` 只改状态与断开连接，同步即可。
            crate::tunnel::tunnel_open,
            crate::tunnel::tunnel_retry,
            crate::tunnel::tunnel_stop,
            // SFTP（plan 0701 / 0702）。⚠️ `sftp_connect`、`sftp_list` 与 `sftp_close`
            // 必须是 async：命令体里各有会等几秒的往返（握手 + 开子系统 / 列目录 /
            // 等传输的清理落地），而同步命令跑在**处理 IPC 请求的那条线程**上 ——
            // 挡住它就等于挡住全部 IPC。
            // `sftp_transfer` 同步：它只把任务排上 runtime 就返回（搬运本身在后台）。
            crate::ssh::ipc::sftp::sftp_open,
            crate::ssh::ipc::sftp::sftp_connect,
            crate::ssh::ipc::sftp::sftp_list,
            crate::ssh::ipc::sftp::sftp_transfer,
            crate::ssh::ipc::sftp::sftp_transfer_cancel,
            crate::ssh::ipc::sftp::sftp_transfers,
            crate::ssh::ipc::sftp::sftp_sides,
            // 面板重新打开时接回已有的会话（关面板不停会话，见 `sftp_sessions` 的文档）。
            crate::ssh::ipc::sftp::sftp_sessions,
            crate::ssh::ipc::sftp::sftp_close,
            // Bitwarden（plan 0902 / 0905）：两个轴、运行时下载、登录 / 解锁 / 锁定 / 登出。
            // ⚠️ `bw_cli_install` 是 async：它要拉约 45 MB（同步命令会把处理 IPC 请求的
            // 那条线程占住几十秒，同 `open_ssh_session` 的理由）。
            // 其余几条是同步的：它们跑的是本地进程，量级在几百毫秒到几秒（`bw` 自己是个
            // 约 140 MB 的 Node SEA，启动就要几百毫秒）—— 登录 / 解锁要联网 + KDF，
            // 最长可到 `timeout::NETWORK`（120 秒），但那是用户按下按钮之后等在原地的那几秒，
            // 而不是"每条 IPC 都被挡住"。
            crate::bw::bw_cli_status,
            crate::bw::bw_cli_settings,
            crate::bw::bw_cli_install,
            crate::bw::bw_status,
            crate::bw::bw_server_set,
            crate::bw::bw_login,
            crate::bw::bw_unlock,
            crate::bw::bw_lock,
            crate::bw::bw_logout,
            crate::bw::bw_sync,
            // 只读导入 SSH key 条目（plan 0903）。同步：一次本地进程调用 + 一次事务，
            // 与面板上其余几条同档（读整个 vault 那一步会多花几百毫秒到几秒）。
            crate::bw::bw_import_keys,
            // 离线缓存的两条检查（plan 0904）：自检**不联网、不起 bw**，比对要 session。
            crate::bw::bw_cache_verify,
            crate::bw::bw_cache_check,
        ])
        .events(collect_events![
            crate::session::SessionEnded,
            crate::prompt::PromptRequest,
            crate::prompt::PromptDismissed,
            // 隧道的状态变化（plan 0601）：ADR-0003 D12 的"状态变化发事件"。
            crate::tunnel::TunnelStateChanged,
        ])
}

/// 生成物的落点，**相对 manifest 而不是相对 cwd** —— 从哪个目录跑都落到同一个地方。
pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/ipc/bindings.ts");
