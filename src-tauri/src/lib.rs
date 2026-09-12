// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
pub mod bindings;
pub mod session;

/// 模板留下的探针命令：用来验证 IPC 通道本身是通的（`docs/STATUS.md` 的 IPC 端到端检查）。
///
/// `#[specta::specta]` 是必须的 —— 少了它，`collect_commands!` 会直接编译失败
/// （比"前端调不到"这种运行期错误好得多）。
#[tauri::command]
#[specta::specta]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 命令清单只有 `bindings::builder()` 一处：**同一份**既喂 `invoke_handler`
    //（运行期分发），也喂 `just gen-types`（生成 TS）。分成两份必然漂移。
    let builder = bindings::builder();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(session::Sessions::default())
        .invoke_handler(builder.invoke_handler())
        .plugin(victauri_plugin::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
