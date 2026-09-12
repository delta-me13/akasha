// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // ⚠️ 必须排在 Tauri 之前：看门狗**就是本可执行文件**再跑一次（plan 0205），
    // 认领晚了就已经起了一个窗口。它没有窗口、没有 IPC，也不碰 Tauri。
    if akasha_lib::watchdog::run_if_watchdog() {
        return;
    }
    akasha_lib::run()
}
