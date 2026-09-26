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
//! | 有端口、但没有一条带描述 | 同样**显式跳过并写明原因**，并把枚举结果原样打出来 ——
//!   "这台机器本来就没有描述"（固件留下的 `COM2`）与"注册表里有描述、枚举没带出来"
//!   从这一条读数里**分不开**，硬判红会把前者（CI 的 runner）当成后者。后者要一台有真实
//!   串口设备的主机才看得见：plan 0803「还缺的那一层」 |
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
    eprintln!("服务端: 端口数={}", ports.len());

    if windows_ports(&ports).is_empty() {
        // 不是失败：空表是正常结果（`serial/enumerate.rs` 的模块文档第一条）。
        eprintln!(
            "跳过: 这台机器上没有 windows 档的端口，枚举 {} 条",
            ports.len()
        );
        // ⚠️ 有端口、却一条 `windows` 档都没有：**两种情形在这里分不开** ——
        // ① 这台机器上的端口在注册表里本就没有描述（固件留下的 `COM2` 这种：`SERIALCOMM`
        //    里有它、`Enum` 下没有对应的 PnP 项），此时**没有可验的对照**；
        // ② 注册表里有描述、而枚举没把它带出来，那才是这条链断了。
        // 判据只能靠"这台机器上有没有一条带描述的端口"来定，而那件事正是本用例在读的读数 ——
        // 所以这里显式跳过并**把枚举结果原样打出来**（`AGENTS.md` §7）：CI 的 runner 就是 ①，
        // 它上面只有一条 `unknown` 的 `COM2`。② 留给有真实串口设备的主机（plan 0803 的「还缺的那一层」）。
        #[cfg(windows)]
        eprintln!("跳过: 这台机器上枚举出的端口在注册表里没有描述，没有可验的对照 —— {listed}");
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
        eprintln!("服务端: windows 档端口数={}", windows_ports(&ports).len());
    }

    // 另一侧：**其余平台上一条都不该是 windows 档**（那一档的取值只来自注册表）。
    if !cfg!(windows) {
        assert_eq!(
            windows_ports(&ports).len(),
            0,
            "非 Windows 平台上出现了 windows 档的端口 —— 那一档的取值只来自注册表：{listed}"
        );
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

    let _ = client.invoke_command("vault_lock", None).await;
}

/// 枚举结果里 `kind` 是 `windows` 档的那些。
fn windows_ports(ports: &[Value]) -> Vec<&Value> {
    ports
        .iter()
        .filter(|port| port.pointer("/kind/windows").is_some())
        .collect()
}
