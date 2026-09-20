//! 端口枚举。
//!
//! 只做一件事：把上游的 `SerialPortInfo` 变成这个 crate 的类型，并让输出**可以断言**
//! （顺序稳定、同一路径只出现一次）。三条必须记住的事实（出处与实测见 plan 0802 的「前置检查」）：
//!
//! - **空表是正常结果**：本机没有串口，或者运行期拿不到 `libudev` 上下文（上游那时返回空表）。
//!   只有系统调用失败才是 `Err` —— “没有端口”与“枚举失败”因此是两种结果。
//! - ⚠️ **列出来的端口不保证能打开**：Linux 的 libudev 分支按 udev 设备给 devnode，
//!   不检查那个节点在 `/dev` 下是否存在（实测：本机列出 32 条 `/dev/ttyS*`，全都打不开）。
//!   所以调用方必须准备好 `open()` 失败这条路径 —— 它带路径与 OS 原因。
//! - ⚠️ **上游在 Windows 上一条元数据都不报**（plan 0803）：它的 Windows 实现把整台机器
//!   报成 `SerialPortType::Unknown`，于是枚举在那里只剩名字。所以那一侧另有一条
//!   **平台专有**的来源（[`PortKind::Windows`]），它在 `cfg(windows)` 之外一个值都构造不出来。

use crate::serial::error::SerialError;

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
    /// **Windows 专有的一档**：从注册表的 PnP 设备项读到的描述（plan 0803）。
    ///
    /// 上游在 Windows 上给不出 USB / PCI / 蓝牙（它把一切都报成 `Unknown`），
    /// 而那边的端口**确实有**可读的描述与硬件 id —— 所以这里新增一档，而不是把
    /// 有描述的设备伪装成 `Unknown`。两项都可能缺：缺了就是 `None`，不填假值。
    ///
    /// ⚠️ **其余平台上这个变体一个值都构造不出来**：取值只来自 `cfg(windows)` 的那条
    /// 注册表路径。留着它是为了让 IPC 层与界面只有**一套形状** —— 那两边若也按平台条件编译，
    /// 平台差异就被推给每一个消费者了。
    Windows {
        /// 设备描述（PnP 设备项的 `FriendlyName`，例如 `USB-SERIAL CH340 (COM3)`）。
        friendly_name: Option<String>,
        /// 硬件 id（PnP 设备项的 `HardwareID` 的第一项，例如 `USB\\VID_1A86&PID_7523`）。
        hardware_id: Option<String>,
    },
}

/// 枚举本机的串口，按路径排序。
///
/// 见模块文档：空表是正常结果；列出来的端口不保证能打开。
pub fn ports() -> Result<Vec<PortInfo>, SerialError> {
    let raw = serialport::available_ports().map_err(|source| SerialError::Enumerate { source })?;
    Ok(describe(normalize(&raw)))
}

/// 补上上游报不出来的描述 —— `ports()` 里另一半**平台相关**的逻辑。
///
/// 分成两条 `cfg` 分支而不是在函数体里写 `if`：Windows 那一侧要读注册表（那条路径在别的
/// 平台上根本不该被编译），而另一侧的正确行为就是**什么都不做**。
#[cfg(windows)]
fn describe(ports: Vec<PortInfo>) -> Vec<PortInfo> {
    windows::describe(ports)
}

/// 其余平台：上游给的类别已经够用（udev 报得出 USB / PCI / 蓝牙），没有第二处来源。
#[cfg(not(windows))]
fn describe(ports: Vec<PortInfo>) -> Vec<PortInfo> {
    ports
}

/// 映射 + 排序 + 去重 —— `ports()` 里可测的那一半。
///
/// 单独拎出来是为了能用手造的上游值测它：真机上拿不到“USB 与重复路径”这些形态。
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
        PortKind::Windows { .. } => 1,
        PortKind::Pci => 2,
        PortKind::Bluetooth => 3,
        PortKind::Unknown => 4,
    }
}

/// 同一条路径上“更具体”的那一档胜出 —— 补进来的描述与上游的 `Unknown` 撞上时用它。
///
/// 只在 `cfg(windows)` 的那条路径上用到：其余平台上 `Windows` 这一档根本构造不出来。
#[cfg(windows)]
fn more_specific(a: &PortKind, b: &PortKind) -> bool {
    rank(a) < rank(b)
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

#[cfg(windows)]
mod windows {
    //! Windows 上的第二处来源：**设备的 PnP 设备项**（plan 0803）。
    //!
    //! # 为什么需要第二处来源
    //!
    //! 上游的 Windows 实现（`serialport` 的 `windows/com.rs`）在 `collect_ports` 里把
    //! 每一个端口都建成 `SerialPortType::Unknown` —— 那是它刻意的：它只要名字，因为
    //! **打开**端口不需要别的。于是枚举在 Windows 上只剩一串 `COM3` 这样的名字。
    //!
    //! # 描述在哪
    //!
    //! 设备的描述登记在 PnP 设备项里：
    //!
    //! | 键 | 给出什么 |
    //! |---|---|
    //! | `HKLM\SYSTEM\CurrentControlSet\Enum\<枚举器>\<设备>\<实例>` | `FriendlyName` / `HardwareID` / `PortName` |
    //!
    //! 与设备项的**实例**那一层里 `PortName` 等于 COM 名的那一条，就是我们手上这条端口。
    //!
    //! ⚠️ **不能拿 `SERIALCOMM` 的键名去拼设备项路径**。它的值是 COM 名，而键名是
    //! `\Device\Serial<m>` —— 微软的文档写明 `<m>` 是 Serial 驱动给设备的编号，
    //! **不是** COM 号，也不含枚举器与实例（`External Naming of COM Ports`）。
    //! 早期实现按"驱动键名"拼过一次，得到的是一串查不到的路径；现在走的是
    //! `SERIALCOMM` 给端口集合、设备项给描述，两边用 **COM 名**对齐。
    //!
    //! # 为什么用 `winreg` 而不是手写 `extern`
    //!
    //! 直接声明 `RegOpenKeyExW` 那几个函数需要 `unsafe`，而 `AGENTS.md` §3.4 把 `unsafe`
    //! 限定在存储模块一处（由 `no-unsafe-outside-store` 强制）—— “读一次注册表”不是那条
    //! 豁免的理由。`winreg` 是同一批 API 的**安全包装**：句柄生命周期、缓冲区长度与
    //! 错误码都交给一个成熟的实现，本模块因此一行 `unsafe` 都不需要。
    //!
    //! # 失败怎么算
    //!
    //! **一条都不报错**。`super::ports` 的契约是“空表是正常结果、只有系统调用失败才是 `Err`”，
    //! 而这一层是**补充信息**：设备项读不到（权限被拒、键不存在、值缺失）时，枚举结果
    //! 退回上游给的那份（只有名字）—— 那正是 plan 0803 之前的行为，而它可用。

    use std::collections::HashMap;

    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::types::FromRegValue;

    use super::{PortInfo, PortKind, more_specific};

    /// 设备项的总根：枚举器 → 设备 → 实例。
    const ENUM: &str = "SYSTEM\\CurrentControlSet\\Enum";

    /// 系统登记的串口：`\Device\Serial<m>` → COM 名。
    ///
    /// ⚠️ 这个键**只**用来回答“系统认为有哪些串口”，它的键名**不能**用来拼设备项路径
    /// （见模块文档）。
    const SERIALCOMM: &str = "HARDWARE\\DEVICEMAP\\SERIALCOMM";

    /// 设备项实例里的端口名。它与 `SERIALCOMM` 的值**逐字符可比**，是两边对齐的钥匙。
    const PORT_NAME: &str = "PortName";

    /// 扫描的深度上限（枚举器 → 设备 → 实例）。
    ///
    /// 真实的设备项就是这三层。
    const MAX_DEPTH: u8 = 3;

    /// 一次枚举里**至多打开多少个键**。
    ///
    /// [`MAX_DEPTH`] 只挡住"树比预期深"，**挡不住树比预期宽** —— 一台插满设备的主机上
    /// `Enum` 下的实例数没有上界，而这一层是**补充信息**，不该为它让 `serial_ports`
    /// 变得不可预期地慢。额度用完就停下：拿到多少描述算多少，枚举结果的正确性不受影响
    /// （最坏情况是退回"只有名字"，见模块文档的"失败怎么算"）。
    const MAX_KEYS: usize = 4096;

    /// 把描述填进枚举结果。
    ///
    /// 两件事**合在一次遍历里**：填描述，以及把上游**没列出来**、而 `SERIALCOMM` 登记着的端口
    /// 补进来。后者是上游那条链的一个已知缺口（它靠 `SetupDiGetClassDevs`，在受限环境里可能
    /// 返回空），而这一层读的是另一个来源 —— 两个来源各自不完整时，并集比任何一侧都全。
    pub(super) fn describe(mut ports: Vec<PortInfo>) -> Vec<PortInfo> {
        let registered = registered_ports();
        let described = described_ports();
        for port in &mut ports {
            let Some(description) = described.get(&port.path) else {
                continue;
            };
            // 上游在 Windows 上只会给出 `Unknown`；真给出别的档时留信息量大的那条
            // （与 `normalize` 的去重同一条口径）。
            if more_specific(description, &port.kind) {
                port.kind = description.clone();
            }
        }
        for name in registered {
            if ports.iter().any(|port| port.path == name) {
                continue;
            }
            // 上游没列出它，但系统登记着：补进来（描述能取到就带上，取不到就 `Unknown`）。
            let kind = described.get(&name).cloned().unwrap_or(PortKind::Unknown);
            ports.push(PortInfo { path: name, kind });
        }
        ports.sort_by(|a, b| a.path.cmp(&b.path));
        ports
    }

    /// 系统登记的串口名（`SERIALCOMM` 的**值**）。读不到就是空表。
    ///
    /// 只取值、**不用键名**：键名是 `\Device\Serial<m>`，与设备项路径没有对应关系。
    pub(super) fn registered_ports() -> Vec<String> {
        let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(SERIALCOMM) else {
            // 这台机器没有串口时这个键根本不存在 —— 空表是正常结果，不是失败。
            return Vec::new();
        };
        key.enum_values()
            .filter_map(|item| item.ok())
            // 用 `FromRegValue` 而不是 `Display`：后者对 `REG_MULTI_SZ` 与二进制值
            // 也打印得出来，而那两个都不是这里的“COM 名”。
            .filter_map(|(_, value)| String::from_reg_value(&value).ok())
            .collect()
    }

    /// 走一遍设备项树，取出每个带 `PortName` 的实例：COM 名 → 它的描述。
    ///
    /// 这是本模块**唯一**会走子树的地方，所以深度有上限、每一层读不出来就跳过。
    pub(super) fn described_ports() -> HashMap<String, PortKind> {
        let mut found = HashMap::new();
        let Ok(root) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(ENUM) else {
            return found;
        };
        let mut budget = MAX_KEYS;
        walk(&root, 0, &mut budget, &mut found);
        found
    }

    /// 递归一层层找 `PortName`（边界见 [`MAX_DEPTH`] 与 [`MAX_KEYS`]）。
    fn walk(key: &RegKey, depth: u8, budget: &mut usize, found: &mut HashMap<String, PortKind>) {
        if depth >= MAX_DEPTH {
            // 到底了：这一层就是实例层，它要么带 `PortName`、要么不是串口。
            return;
        }
        for name in key.enum_keys().filter_map(|item| item.ok()) {
            // 额度用完就停 —— 少拿几条描述不影响枚举结果（最坏退回"只有名字"）。
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let Ok(child) = key.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            // 实例层：带 `PortName` 的就是我们要的那一条。
            if let Ok(port) = child.get_value::<String, _>(PORT_NAME) {
                if let Some(kind) = described_kind(&child) {
                    found.insert(port, kind);
                }
                continue;
            }
            walk(&child, depth + 1, budget, found);
        }
    }

    /// 一个设备项里的描述。两项都读不到就是 `None`（不留空壳，见函数末尾）。
    fn described_kind(key: &RegKey) -> Option<PortKind> {
        let friendly_name = key.get_value::<String, _>("FriendlyName").ok();
        // `HardwareID` 是**多字符串值**：`winreg` 直接解码成 `Vec<String>`，
        // 最具体的那一项在最前（不必自己去拆分隔符）。
        let hardware_id = key
            .get_value::<Vec<String>, _>("HardwareID")
            .ok()
            .and_then(|ids| ids.into_iter().next());
        // 两项都读不到时**不留一个空壳**：`Windows {}` 与 `Unknown` 对界面是同一件事，
        // 而前者会让“这一档是从注册表来的”这句话变成假的。
        if friendly_name.is_none() && hardware_id.is_none() {
            return None;
        }
        Some(PortKind::Windows {
            friendly_name,
            hardware_id,
        })
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

    #[test]
    fn the_described_kind_outranks_unknown_but_not_usb() {
        // 排序用的次序同时决定"两处来源撞上时留哪一条"：注册表给的描述比上游的 `Unknown`
        // 具体，但比不上 udev 报得出的 USB 五项。这一条在两条平台上都成立
        // （`Windows` 这一档本身只在 Windows 上构造得出来，次序却是共用的一份）。
        assert!(
            rank(&PortKind::Windows {
                friendly_name: Some("USB-SERIAL CH340".into()),
                hardware_id: None,
            }) < rank(&PortKind::Unknown)
        );
        assert!(
            rank(&PortKind::Usb {
                vid: 1,
                pid: 2,
                serial: None,
                manufacturer: None,
                product: None,
            }) < rank(&PortKind::Windows {
                friendly_name: None,
                hardware_id: None,
            })
        );
    }

    /// 其余平台：`describe` 是恒等 —— 上游给的类别已经够用，没有第二处来源。
    #[cfg(not(windows))]
    #[test]
    fn describing_is_the_identity_off_windows() {
        let before = normalize(&[usb("/dev/ttyUSB0", 0x0403, 0x6001)]);
        let after = describe(before.clone());
        assert_eq!(before, after);
    }

    /// `SERIALCOMM` 的**值**才是 COM 名，键名不是（plan 0803 踩过一次的地方）。
    ///
    /// 这一条钉住的是"两边用 COM 名对齐"这条口径：只要能拿到一台带 `PortName` 的设备项，
    /// 它报出来的名字就该与系统登记的那一份逐字符相同。⚠️ **本机没有串口**，所以这里只断言
    /// 形状（读得到就非空、读不到就是空表），"真的对上了"由 E2E `windows_ports` 在真机上给。
    #[cfg(windows)]
    #[test]
    fn registered_ports_are_com_names_or_nothing() {
        let names = windows::registered_ports();
        for name in &names {
            assert!(
                !name.is_empty(),
                "登记表里出现了一个空名字 —— 那不是 COM 名（值类型读错时会这样）"
            );
        }
        // 不断言条数：本机没有串口时是 0 条，有设备的机器上是别的数 —— 空表本来就是
        // 正常结果（模块文档第一条）。这里只钉住"读得到的东西都是 COM 名的形状"。
    }

    /// 设备项那一侧：读出来的描述里，两项至少有一项（`described_kind` 不返回空壳）。
    ///
    /// 本机没有串口，所以这条在 CI 的 Windows 格子上多半只是"表是空的"；它真正的价值是
    /// **把那条路走一遍**：走通说明键树与权限都没问题，而不是等到有设备时才发现读不到。
    #[cfg(windows)]
    #[test]
    fn describing_the_devices_never_yields_an_empty_kind() {
        let described = windows::described_ports();
        for (port, found) in &described {
            assert!(!port.is_empty(), "PortName 是空的");
            match found {
                PortKind::Windows {
                    friendly_name,
                    hardware_id,
                } => assert!(
                    friendly_name.is_some() || hardware_id.is_some(),
                    "两项都缺时不该产出 Windows 档：{port}"
                ),
                other => panic!("设备项产出了非 Windows 档：{other:?}"),
            }
        }
        // 同上：条数随机器变，不断言。这条的价值是**把那条路走一遍** ——
        // 走通说明键树结构与权限都没问题，而不是等到有设备时才发现读不到。
    }
}
