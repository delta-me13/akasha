//! 端口枚举。
//!
//! 只做一件事：把上游的 `SerialPortInfo` 变成这个 crate 的类型，并让输出**可以断言**
//! （顺序稳定、同一路径只出现一次）。两条必须记住的事实（出处与实测见 plan 0802 的「前置检查」）：
//!
//! - **空表是正常结果**：本机没有串口，或者运行期拿不到 `libudev` 上下文（上游那时返回空表）。
//!   只有系统调用失败才是 `Err` —— "没有端口"与"枚举失败"因此是两种结果。
//! - ⚠️ **列出来的端口不保证能打开**：Linux 的 libudev 分支按 udev 设备给 devnode，
//!   不检查那个节点在 `/dev` 下是否存在（实测：本机列出 32 条 `/dev/ttyS*`，全都打不开）。
//!   所以调用方必须准备好 `open()` 失败这条路径 —— 它带路径与 OS 原因。

use crate::error::SerialError;

/// 一个串口设备。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    /// 设备路径（Unix 上是 `/dev/ttyUSB0` 一类，Windows 上是 `COM3`）。
    pub path: String,
    /// 这条路径由什么硬件暴露。
    pub kind: PortKind,
}

/// 端口的硬件类别（上游 `SerialPortType` 的映射）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortKind {
    /// USB 转串口。
    ///
    /// 五项都取自 udev 的属性，可能缺：`None` = 设备自己没报，udev 的硬件库也没有。
    Usb {
        /// 厂商号。
        vid: u16,
        /// 产品号。
        pid: u16,
        /// 设备自报的序列号。
        serial: Option<String>,
        /// 厂商名。
        manufacturer: Option<String>,
        /// 产品名。
        product: Option<String>,
    },
    /// 主板上的 PCI 串口。
    Pci,
    /// 蓝牙串口（`rfcomm`）。
    Bluetooth,
    /// 判定不出来。
    Unknown,
}

/// 枚举本机的串口，按路径排序。
///
/// 见模块文档：空表是正常结果；列出来的端口不保证能打开。
pub fn ports() -> Result<Vec<PortInfo>, SerialError> {
    let raw = serialport::available_ports().map_err(|source| SerialError::Enumerate { source })?;
    Ok(normalize(&raw))
}

/// 映射 + 排序 + 去重 —— `ports()` 里可测的那一半。
///
/// 单独拎出来是为了能用手造的上游值测它：真机上拿不到"USB 与重复路径"这些形态。
fn normalize(raw: &[serialport::SerialPortInfo]) -> Vec<PortInfo> {
    let mut ports: Vec<PortInfo> = raw.iter().map(map_port).collect();
    // 顺序稳定：先按路径，同一路径再按信息量（USB 最具体，Unknown 最含糊）。
    ports.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| rank(&a.kind).cmp(&rank(&b.kind)))
    });
    // 同一路径只留一条。上游对同一个 devnode 给出两条记录时，留最具体的那一档。
    ports.dedup_by(|a, b| a.path == b.path);
    ports
}

/// 排序用的信息量次序（越小越具体）。它同时决定去重时留下哪一条。
fn rank(kind: &PortKind) -> u8 {
    match kind {
        PortKind::Usb { .. } => 0,
        PortKind::Pci => 1,
        PortKind::Bluetooth => 2,
        PortKind::Unknown => 3,
    }
}

/// 上游类型 → 本 crate 类型。`serialport` 的枚举只出现在这个文件里
/// （同 `transport.rs` 对参数枚举的处理：升级它要改的是这里）。
fn map_port(raw: &serialport::SerialPortInfo) -> PortInfo {
    use serialport::SerialPortType;

    PortInfo {
        path: raw.port_name.clone(),
        kind: match &raw.port_type {
            SerialPortType::UsbPort(info) => PortKind::Usb {
                vid: info.vid,
                pid: info.pid,
                serial: info.serial_number.clone(),
                manufacturer: info.manufacturer.clone(),
                product: info.product.clone(),
            },
            SerialPortType::PciPort => PortKind::Pci,
            SerialPortType::BluetoothPort => PortKind::Bluetooth,
            SerialPortType::Unknown => PortKind::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::{SerialPortInfo, SerialPortType, UsbPortInfo};

    fn usb(path: &str, vid: u16, pid: u16) -> SerialPortInfo {
        SerialPortInfo {
            port_name: path.to_string(),
            port_type: SerialPortType::UsbPort(UsbPortInfo {
                vid,
                pid,
                serial_number: Some("S-1".into()),
                manufacturer: Some("厂家".into()),
                product: Some("适配器".into()),
            }),
        }
    }

    fn plain(path: &str, port_type: SerialPortType) -> SerialPortInfo {
        SerialPortInfo {
            port_name: path.to_string(),
            port_type,
        }
    }

    #[test]
    fn every_upstream_category_maps_to_one_of_ours() {
        // 四种都到位：上游加一种而不映射的话，这里会看到 Unknown 悄悄顶替它。
        let mapped = normalize(&[
            usb("/dev/ttyUSB0", 0x1a86, 0x7523),
            plain("/dev/ttyS0", SerialPortType::PciPort),
            plain("/dev/rfcomm0", SerialPortType::BluetoothPort),
            plain("/dev/ttyAMA0", SerialPortType::Unknown),
        ]);
        let kinds: Vec<PortKind> = mapped.into_iter().map(|port| port.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PortKind::Bluetooth,
                PortKind::Unknown,
                PortKind::Pci,
                PortKind::Usb {
                    vid: 0x1a86,
                    pid: 0x7523,
                    serial: Some("S-1".into()),
                    manufacturer: Some("厂家".into()),
                    product: Some("适配器".into()),
                },
            ],
            "映射结果与预期不符（顺序按路径：rfcomm0 / ttyAMA0 / ttyS0 / ttyUSB0）"
        );
    }

    #[test]
    fn missing_usb_metadata_stays_missing() {
        // 拿不到就是拿不到：不得填 0 或 unknown 顶替（日志与字段的同一纪律）。
        let mapped = normalize(&[SerialPortInfo {
            port_name: "COM3".into(),
            port_type: SerialPortType::UsbPort(UsbPortInfo {
                vid: 0x0403,
                pid: 0x6001,
                serial_number: None,
                manufacturer: None,
                product: None,
            }),
        }]);
        assert_eq!(
            mapped[0].kind,
            PortKind::Usb {
                vid: 0x0403,
                pid: 0x6001,
                serial: None,
                manufacturer: None,
                product: None,
            }
        );
    }

    #[test]
    fn the_result_is_sorted_by_path() {
        let mapped = normalize(&[
            plain("COM9", SerialPortType::Unknown),
            plain("COM1", SerialPortType::Unknown),
            plain("COM5", SerialPortType::Unknown),
        ]);
        let paths: Vec<&str> = mapped.iter().map(|port| port.path.as_str()).collect();
        assert_eq!(paths, vec!["COM1", "COM5", "COM9"]);
    }

    #[test]
    fn a_repeated_path_keeps_the_most_specific_entry() {
        // 上游对同一个 devnode 给两条记录时（一条带了 USB 信息、一条没有），
        // 留信息量大的那条 —— 否则列出来的设备会凭空丢掉厂商与序列号。
        let mapped = normalize(&[
            plain("/dev/ttyUSB0", SerialPortType::Unknown),
            usb("/dev/ttyUSB0", 0x0403, 0x6001),
        ]);
        assert_eq!(mapped.len(), 1);
        assert!(matches!(mapped[0].kind, PortKind::Usb { .. }));
    }

    #[test]
    fn no_ports_stays_an_empty_list() {
        // 空表是正常结果，不是失败（模块文档的第一条）。
        assert!(normalize(&[]).is_empty());
    }
}
