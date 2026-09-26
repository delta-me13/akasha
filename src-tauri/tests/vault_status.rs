//! `AGENTS.md` §7 第一条的落点：**真实路径走通** —— 在真 app 上 invoke `vault_status`，
//! 再用测试进程自己看到的东西对账。
//!
//! 为什么单独一个用例：plan 0403 是"数据目录怎么推出来的"（P2）第一次**走通真路径** ——
//! 在那之前它只有单测（`config.rs` 里那条纯函数）。单测能证明"函数算得对"，
//! 证明不了"app 里那个 `AppHandle` 走的是同一条路"。
//!
//! ## 这条命令为什么没有 `wait_for`
//!
//! §7 要求异步命令一律 `wait_for`，不许 `sleep` 猜。这条命令**是同步的**：
//! 它只读一次文件元数据就返回，`invoke_command` 的返回值就是结果本身。
//! 所以这里没有等待 —— 不是忘了，而是没有异步的那一段。
//!
//! ## 与"前端状态一致"这件事
//!
//! 库里这条状态**没有前端 UI**（各池的呈现形式未定，见 plan 0403 的非目标），
//! 所以"前后端一致"的对账对象是**测试进程自己 `stat` 同一个路径**得到的状态 ——
//! app 说 `missing` 而磁盘上明明有文件，这条就要红。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use std::path::{Path, PathBuf};

use victauri_test::VictauriClient;

/// 便携数据目录名（`config::PORTABLE_DIR` 的约定，`portable.md` §4 第 1 条）。
const PORTABLE_DIR: &str = "akasha-data";
/// 库文件名（ADR-0002 D1）。
const VAULT_FILE: &str = "akasha.db";

fn skip_unless_e2e() -> bool {
    if !victauri_test::is_e2e() {
        eprintln!("跳过: 未设置 VICTAURI_E2E=1（该变量由 just test-e2e 设置）");
        return true;
    }
    false
}

#[tokio::test]
async fn vault_status_agrees_with_what_the_filesystem_says() {
    if skip_unless_e2e() {
        return;
    }

    let mut client = VictauriClient::discover()
        .await
        .expect("连不上 app —— 用 `just test-e2e`（它会自己起 app）");

    let status = client
        .invoke_command("vault_status", None)
        .await
        .expect("vault_status 调不通 —— 它登记进 bindings.rs 了吗？");

    let path = status
        .pointer("/path")
        .and_then(serde_json::Value::as_str)
        .expect("返回值里必须有 path（None 说明数据目录取不到 —— 那是环境问题，不是「库不存在」）");
    let reported = status
        .pointer("/state")
        .and_then(serde_json::Value::as_str)
        .expect("返回值里必须有 state");

    // ① 落点的**文件名**是磁盘格式的一部分（ADR-0002 D1）：改了等于迁移用户数据。
    let path = PathBuf::from(path);
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(VAULT_FILE),
        "库文件名不对：{path:?}"
    );

    // ② 状态必须与**这个进程自己看到**的文件系统一致 —— 这就是"真实路径"那条判据。
    let on_disk = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() == 0 => "empty",
        Ok(_) => "present",
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => "missing",
        Err(err) => panic!("连元数据都读不到（{}）：{err}", path.display()),
    };
    assert_eq!(
        reported, on_disk,
        "app 说 {reported}、磁盘上是 {on_disk}（{path:?}）—— 两边必须一致"
    );

    // ③ P2：便携目录存在时，它必须在 **bin 同目录**里（"文件搬走了数据跟着走"的全部含义）。
    //    这里不猜 target 目录在哪 —— 猜路径会在 `CARGO_TARGET_DIR` 一变就假红 ——
    //    而是直接看那个目录的兄弟里有没有 app 自己的可执行文件。
    match path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
    {
        Some(PORTABLE_DIR) => {
            let exe_dir = path
                .parent()
                .and_then(Path::parent)
                .expect("便携目录的父目录");
            assert!(
                exe_dir.join("akasha").is_file() || exe_dir.join("akasha.exe").is_file(),
                "便携目录 {} 不在 bin 同目录里（{} 里没有 akasha 可执行文件）—— \
                 那它就不叫「便携」（portable.md §4 第 1 条）",
                path.display(),
                exe_dir.display()
            );
            eprintln!("报告: 落点=便携目录 路径={}", path.display());
        }
        // bin 同目录还没有 `akasha-data/` 时，app 按 `portable.md` §4 第 2 条退回 OS 数据目录。
        // 这是**环境**（那台机器上有没有那个目录），不是回归 —— 显式说出来，不假装验过。
        other => eprintln!("报告: 落点=OS 数据目录 父目录={other:?}"),
    }
}
