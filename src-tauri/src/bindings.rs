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
            crate::session::open_session,
            // SSH 会话（plan 0504）。⚠️ 它**必须**留着 async：命令体里有一次会阻塞几秒的
            // 握手（最长 `connect_timeout`），而同步命令跑在处理 IPC 请求的那条线程上 ——
            // 挡住它就等于挡住全部 IPC，包括用户回答问题要用的那三条。
            crate::ssh::open_ssh_session,
            crate::session::write_session,
            crate::session::resize_session,
            crate::session::close_session,
            // 提问往返（plan 0504）：后端问 → 前端答。
            crate::prompt::ssh_prompt_credential,
            crate::prompt::ssh_prompt_host_key,
            crate::prompt::ssh_prompt_cancel,
        ])
        .events(collect_events![
            crate::session::SessionEnded,
            crate::prompt::PromptRequest,
            crate::prompt::PromptDismissed,
        ])
}

/// 生成物的落点，**相对 manifest 而不是相对 cwd** —— 从哪个目录跑都落到同一个地方。
pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/ipc/bindings.ts");
