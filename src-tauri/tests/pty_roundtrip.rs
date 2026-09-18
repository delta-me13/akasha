//! 真实 tty 上的往返：把一对 PTY 的**从端**当串口打开。
//!
//! 为什么需要它：单测里的假读端能覆盖读循环的每一条判定，但 `open()` 这条路
//! （设备名 → 句柄 → termios → 参数）只有真设备才走得到，而那正是串口最容易出错的一段。
//! PTY 从端是 Unix 上能拿到的最接近的真 tty，且**不需要任何硬件**；
//! `serialport` 打开时会调 `cfmakeraw`，于是从端是原始模式，字节不被行规程改写。
//!
//! ⚠️ 只覆盖 Unix：Windows 上没有"把一个 tty 设备名当串口打开"的等价物
//! （`portable-pty` 在 Windows 上走 ConPTY，没有设备名）。那条路交给 CI 的三平台
//! `just check`（类型检查）与真机。
//!
//! ⚠️ **Linux only，macOS 排除在外**：macOS 上 `open` 这一步就失败 ——
//! `Open { path: "/dev/ttys003", source: "Not a typewriter" }`。原因是上游 `serialport`
//! 在 Apple 目标上用 `IOSSIOSPEED` 设波特率，而它对 pty 返回 `ENOTTY`
//! （`serialport-4.10.1/src/posix/termios.rs` 自己写着这一条）。于是"把 PTY 从端当串口"
//! 这套脚手架在 macOS 上不成立 —— 被验的代码没有机会执行，留一条永远红的用例没有意义。
//! 真串口设备不受影响（它们接受这个 ioctl）；参数映射在 macOS 上仍由 `settings.rs`
//! 的映射单测守着。

#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Duration;

use akasha_pty::{Capabilities, TerminalSize, Transport, TransportError};
use akasha_lib::serial::{SerialSettings, SerialTransport};
use portable_pty::{PtyPair, PtySize, native_pty_system};

/// 造一对 PTY，并把两端都持有到用例结束。
///
/// `pair` 必须活到用例结束：从端那一路的文件描述符一旦全部关闭，主端的读就会以 EIO
/// 结束 —— 而那与"串口收尾"长得一模一样，会让断言失去意义。
struct Fixture {
    transport: SerialTransport,
    pair: PtyPair,
}

fn fixture() -> Option<Fixture> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("造一对 PTY 失败");
    // ⚠️ 设备名在**主端**上问：从端只有 fd（`tty_name` 是 `MasterPty` 的方法）。
    let path = pair.master.tty_name()?;
    let transport = SerialTransport::open(&SerialSettings::new(path.to_string_lossy(), 9600))
        .expect("把 PTY 从端当串口打开失败");
    Some(Fixture { transport, pair })
}

/// 带期限地读满 want 字节。
///
/// 读端是阻塞的，没有期限的话一次失败会表现为**挂住**而不是失败 ——
/// 而挂住的用例在 CI 上只会把整条流水线拖到超时。
fn read_exactly_within(reader: Box<dyn Read + Send>, want: usize, within: Duration) -> Vec<u8> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = vec![0u8; want];
        let _ = tx.send(reader.read_exact(&mut buf).map(|()| buf));
    });
    match rx.recv_timeout(within) {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(err)) => panic!("读失败：{err}"),
        Err(_) => panic!("{within:?} 内没有读到 {want} 字节"),
    }
}

/// 期限之内流**是否已经结束**（读到 0 字节）。
fn stream_ends_within(reader: Box<dyn Read + Send>, within: Duration) -> bool {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = reader;
        let _ = tx.send(reader.read(&mut [0u8; 8]));
    });
    match rx.recv_timeout(within) {
        Ok(Ok(0)) => true,
        Ok(Ok(n)) => panic!("还读到 {n} 个字节，流没有结束"),
        Ok(Err(err)) => panic!("读失败：{err}"),
        Err(_) => false,
    }
}

#[test]
fn bytes_written_to_the_transport_reach_the_other_end() {
    let Some(mut fixture) = fixture() else {
        eprintln!("跳过：拿不到 PTY 从端的设备名");
        return;
    };
    let reader = fixture
        .pair
        .master
        .try_clone_reader()
        .expect("取主端读句柄失败");
    fixture.transport.write(b"ping").expect("写入串口失败");
    let got = read_exactly_within(reader, 4, Duration::from_secs(5));
    assert_eq!(got, b"ping");
}

#[test]
fn bytes_written_by_the_other_end_are_read_from_the_transport() {
    let Some(mut fixture) = fixture() else {
        eprintln!("跳过：拿不到 PTY 从端的设备名");
        return;
    };
    let mut writer = fixture.pair.master.take_writer().expect("取主端写句柄失败");
    let output = fixture.transport.output_stream().expect("取读端失败");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = writer.write_all(b"pong");
        let _ = writer.flush();
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("写给主端超时");
    let got = read_exactly_within(output, 4, Duration::from_secs(5));
    assert_eq!(got, b"pong");
}

#[test]
fn the_capabilities_are_what_a_serial_port_lacks() {
    let Some(mut fixture) = fixture() else {
        eprintln!("跳过：拿不到 PTY 从端的设备名");
        return;
    };
    assert_eq!(fixture.transport.capabilities(), Capabilities::NONE);
    // 没有窗口尺寸：必须由 trait 的默认实现答 Unsupported，而不是静默成功。
    assert!(matches!(
        fixture.transport.resize(TerminalSize::DEFAULT),
        Err(TransportError::Unsupported(_))
    ));
    // 没有结局、没有本地进程。
    assert_eq!(fixture.transport.exited().expect("exited 失败"), None);
    assert_eq!(fixture.transport.session_leader(), None);
    // 读端只能取走一次（两个读端会互相偷字节）。
    assert!(fixture.transport.output_stream().is_some());
    assert!(fixture.transport.output_stream().is_none());
}

#[test]
fn shutdown_ends_the_read_side_and_is_idempotent() {
    let Some(mut fixture) = fixture() else {
        eprintln!("跳过：拿不到 PTY 从端的设备名");
        return;
    };
    let output = fixture.transport.output_stream().expect("取读端失败");
    assert_eq!(fixture.transport.shutdown().expect("收尾失败"), None);
    // 读端在 READ_TICK 的量级内看到流结束 —— 这正是"收尾不靠设备事件"的判据。
    assert!(
        stream_ends_within(output, Duration::from_secs(2)),
        "收尾之后读端没有结束"
    );
    // 幂等：第二次调用返回同一个结局，不报错。
    assert_eq!(fixture.transport.shutdown().expect("第二次收尾失败"), None);
    // 收尾之后的写必须被拒：继续写没有意义，而静默丢弃会让上层以为字节已经发出去了。
    assert!(matches!(
        fixture.transport.write(b"after-shutdown"),
        Err(TransportError::Closed)
    ));
}
