//! 真实调用：枚举本机的串口（plan 0802）。
//!
//! ⚠️ 断言的是**输出形状**（顺序稳定、无重复、路径非空），不是**条数** ——
//! 条数随机器变，形状才是承诺。
//!
//! ⚠️ **不断言"枚举到的端口都能打开"**：实测本机列出 32 条 `/dev/ttyS*`，而 `/dev` 下
//! 一个都没有（libudev 那套实现不检查节点是否存在）。而且真去打开它们在别的机器上会碰到
//! 用户接着的真实设备 —— 这条边界写在 `enumerate.rs` 的模块文档里。

use akasha_serial::ports;

#[test]
fn the_listing_is_sorted_and_free_of_duplicates() {
    let listed = ports().expect("枚举失败");

    let mut sorted = listed.clone();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(listed, sorted, "枚举结果没有按路径给出稳定顺序");

    let mut paths: Vec<&str> = listed.iter().map(|port| port.path.as_str()).collect();
    paths.dedup();
    assert_eq!(paths.len(), listed.len(), "同一路径出现了两次");

    for port in &listed {
        assert!(!port.path.trim().is_empty(), "枚举给出了一条空路径");
    }

    // 机器相关的那一半：打在输出里，供人工判读（不是判据）。
    eprintln!("枚举到 {} 条端口：{:?}", listed.len(), paths);
}

#[cfg(target_os = "linux")]
#[test]
fn every_listed_port_has_a_sysfs_entry() {
    // libudev 那套实现（也含关掉 feature 后的 sysfs 那套）都从 `/sys/class/tty` 取设备，
    // 所以每一条都应当在那里找得到 —— 这条把"路径从哪来"钉住，而不依赖条数。
    let listed = ports().expect("枚举失败");
    for port in &listed {
        let name = std::path::Path::new(&port.path)
            .file_name()
            .unwrap_or_else(|| panic!("枚举给出的路径没有文件名：{}", port.path));
        let sysfs = std::path::Path::new("/sys/class/tty").join(name);
        assert!(
            sysfs.exists(),
            "{} 在 {} 里没有对应项",
            port.path,
            sysfs.display()
        );
    }
}
