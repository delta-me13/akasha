//! plan 0803 的**端到端**验收：Windows 上枚举出来的端口带得出描述。
//!
//! 判据（ROADMAP 阶段 8 第二条的 Windows 面）=「枚举在 Windows 上报出这台机器上的端口，
//! 而不只是名字」。逐条落点：
//!
//! | 判据 | 断言在哪 |
//! |---|---|
//! | 描述说得清是什么设备 | 每一条 `windows` 档至少带一项（`friendlyName` 或 `hardwareId`）——
//!   两项都缺的 `Windows` 档是**不该产生**的（`described_kind` 会退回 `Unknown`） |
//! | 其余平台不受影响 | 每一条都**不是** `windows` 档 —— 这一档的取值只来自注册表 |
//! | 枚举结果进了界面 | 面板上的条数 == `serial_ports` 的条数，且每一行的类别那一栏都有话说 |
//! | 没插设备时仍然可用 | `windows` 档为空 ⇒ **显式跳过并写明原因**（空表是正常结果，不是失败） |
//! | 有端口就不能只有名字 | Windows 上枚举出端口却一条 `windows` 档都没有 ⇒ **红** |
//!   —— 少了这一条，上面那个循环在真机上会**空过**（同 `AGENTS.md` §6：一次"全部通过"
//!   无法区分"它在工作"与"它没有匹配到任何内容"） |
//!
//! ⚠️ **这一条在本机（无串口设备）走不到"真的有设备"那一支**：它执行的是空表分支。
//! 判据的另一半（`hardwareId` 与 `SERIALCOMM` 逐字符对上）要一台**有串口设备的 Windows 主机**，
//! 那一条见 plan 0803「还缺的那一层」。这里不写它，是因为写了就等于声称验过。
//!
//! ## 为什么不造设备
//!
//! 另三条串口用例（`serial_session` / `serial_ports_ui` / `serial_device_gone`）都自己造一对
//! PTY、把从端的路径当设备，而**这条路上 Windows 造不出来**（ConPTY 没有设备节点，见
//! `support::fake_serial_skip_reason`）—— 那三条因此全平台跳过。本条反过来：它不造任何设备，
//! 验的是**这台机器上真实存在的东西**（注册表里的 PnP 设备项）有没有走到界面上。
//! 于是它是这三条之外**唯一**在 Windows 上真的会执行的串口用例。
//!
//! ⚠️ 它**不开会话**：不碰设备、不占独占锁，所以与那三条并列而互不影响。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

mod support;

use serde_json::Value;
use support::{open_serial_panel, skip_unless_e2e, text};

/// 这一条**没有** `fake_serial_skip_reason` 那道门：它不需要假设备，任何平台都跑得起来。
/// 区别只在于断言的方向 —— 见下。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_enumerated_ports_say_what_they_are() {
    if skip_unless_e2e() {
        return;
    }
    // 与那三条不同：**不**在这里跳过 Windows。见文件头。
    let Some((mut client, _fixture, _path)) = support::connect_and_prepare().await else {
        return;
    };

    let listed = client
        .invoke_command("serial_ports", None)
        .await
        .expect("serial_ports 调不通 —— 它登记进 bindings.rs 了吗？");
    let ports = listed.as_array().cloned().unwrap_or_default();
    eprintln!("枚举：本机 {} 条：{listed}", ports.len());

    if windows_ports(&ports).is_empty() {
        // 不是失败：空表是正常结果（`serial/enumerate.rs` 的模块文档第一条）。
        eprintln!(
            "跳过「每一条都带得出描述」：这台机器上没有 windows 档的端口 —— 枚举给出 {} 条",
            ports.len()
        );
        // ⚠️ **Windows 上枚举出了端口、却一条描述都没有**，那是这条链断了的形状，不是"没设备"。
        // 少了这一条断言，整条用例会在真机上**空过**：循环体一次都不执行，而它本该是这条判据的
        // 唯一落点。设备数为 0 时上面已经跳过，所以这里只盯"有端口"的那一半。
        //
        // 已知边界：驱动器不走 Serial 函数驱动时它可能不出现在 `SERIALCOMM` 里，于是端口在而
        // 描述缺席。那种机器上这一条会红 —— 而它红得对，因为界面上那个端口确实只有名字。
        #[cfg(windows)]
        assert!(
            ports.is_empty(),
            "Windows 上枚举出 {} 条端口，却没有一条带描述 —— 注册表那条链断了？（{listed}）",
            ports.len()
        );
    } else {
        for port in windows_ports(&ports) {
            let described = port
                .pointer("/kind/windows/friendlyName")
                .and_then(Value::as_str)
                .is_some()
                || port
                    .pointer("/kind/windows/hardwareId")
                    .and_then(Value::as_str)
                    .is_some();
            assert!(
                described,
                "windows 档的端口一项描述都没有 —— 那与 unknown 档没有区别：{port}"
            );
        }
        eprintln!(
            "描述：{} 条 windows 档的端口各自带得出描述",
            windows_ports(&ports).len()
        );
    }

    // 另一侧：**其余平台上一条都不该是 windows 档**（那一档的取值只来自注册表）。
    if !cfg!(windows) {
        assert_eq!(
            windows_ports(&ports).len(),
            0,
            "非 Windows 平台上出现了 windows 档的端口 —— 那一档的取值只来自注册表：{listed}"
        );
        eprintln!("其余平台：{} 条里没有一条是 windows 档", ports.len());
    }

    // ── 界面那一半：枚举结果**到了界面上**（不是只在命令的返回值里）──────────
    open_serial_panel(&mut client).await;
    let shown = text(
        &client
            .eval_js(
                "document.querySelector('.serial-ports')?.getAttribute('data-serial-ports') ?? ''",
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        shown,
        ports.len().to_string(),
        "面板上的条数与后端枚举不一致"
    );

    // 每一条枚举结果在界面上都有一行，且那一行的类别那一栏**不是空的**
    // （`describeKind` 对任何一档都给得出话说 —— 它若不认识新加的那一档，这里就是空白）。
    let empty_kinds = text(
        &client
            .eval_js(
                "Array.from(document.querySelectorAll('.serial-port-item')).filter((b) => !b.querySelector('.serial-port-kind')?.textContent?.trim()).length.toString()"
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        empty_kinds, "0",
        "有端口的类别那一栏是空的：前端不认识那一档？"
    );
    eprintln!("界面：{shown} 行，每一行的类别都有话说");

    let _ = client.invoke_command("vault_lock", None).await;
}

/// 枚举结果里 `kind` 是 `windows` 档的那些。
fn windows_ports(ports: &[Value]) -> Vec<&Value> {
    ports
        .iter()
        .filter(|port| port.pointer("/kind/windows").is_some())
        .collect()
}
