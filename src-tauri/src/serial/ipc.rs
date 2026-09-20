//! **串口接进 IPC**（plan 1101）—— 让真 app 能开一个串口会话。
//!
//! 这个模块是"库那一侧"（`serial`）与"界面那一侧"之间的接线，一共三件事：
//!
//! | 事 | 在哪 | 为什么在这里 |
//! |---|---|---|
//! | 池行的过 IPC 表示 | [`SerialEntry`] + [`vault_serials`] | 界面要**看得见**池里有哪些配置才能挑一条 |
//! | 枚举结果的过 IPC 表示 | [`SerialPort`] + [`serial_ports`] | 同上：界面要看得见本机有哪些端口，才谈得上挑一条或手输一条 |
//! | 取值域两侧的映射 | [`SerialParity`] / [`SerialFlow`] / [`SerialPortKind`] | 存储 crate 不依赖 specta，串口 crate 不带 serde（各自只认自己的类型） |
//! | 一条会话命令 | [`open_serial_session`] | `Sessions::register` 已经是 `<T: Transport>`，串口只是第三种载体 |
//!
//! ## 四条形状
//!
//! 1. **参数显式，不是池行 id**。SSH 那条命令收的是 `hostId`，因为认证材料在库里；
//!    而打开一个串口**不碰库**（没有秘密），收下六个字段即可 —— 于是它没有 `locked` 这一档，
//!    并且 plan 1102 的"手输路径 + 参数可编辑"用的是**同一条**命令。
//! 2. **同步命令**。打开一个本地设备是一次系统调用，没有握手、没有对端要等；
//!    而参数在**碰设备之前**就校验完（`SerialSettings::validate` 与两个 `TryFrom`）。
//!    它与 `open_session`（本地 PTY）同档，而不是与 `open_ssh_session` 同档。
//! 3. **与 local / SSH 共用同一条尾巴**（[`crate::session::open_terminal`]）：注册 → 频道 →
//!    收尾线程。串口没有窗口尺寸、没有退出结局、没有本地进程（`Capabilities::NONE`），
//!    这三条都由载体自己声明，这里一行特例都不写。
//! 4. **枚举只是一条只读命令**（plan 1102）：它不登记状态、不碰库、不试着打开设备 ——
//!    于是"照枚举的列表打开"、"手输一个路径打开"、"照池里的一行打开"是**并列**的三条输入，
//!    谁都不挡谁。⚠️ 列出来的端口**不保证打得开**（问题 #150）：udev 报 devnode 时
//!    **不检查**它在 `/dev` 下是否存在（实测：本机列出 32 条 `/dev/ttyS*`，`/dev` 下一条都没有）。

use crate::serial::{
    DataBits, Flow, Parity, PortInfo, PortKind, SerialError, SerialSettings, SerialTransport,
    StopBits,
};
use crate::store::pools::serial as pool;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, Webview};

use crate::session::{self, IpcError, RawChannel, SessionHandle, Sessions};
use crate::store::ipc::vault::{Vault, VaultError};

/// 串口配置行的**过 IPC 表示**。
///
/// 与 [`crate::store::ipc::pools::HostId`] 同一条理由用 `u32` 代理 `i64`：生成器拒绝把 64 位整数导出成
/// TS（BigInt 精度问题，问题 #32），而截断会把"要开的那条配置"变成**另一条**。
pub type SerialId = u32;

/// 行 id → 过 IPC 的表示。装不下就报错，**绝不截断**。
fn serial_id(id: i64) -> Result<SerialId, VaultError> {
    SerialId::try_from(id).map_err(|_| VaultError::Unusable {
        message: format!("serial 配置池的行 id 超出可表示范围（{id}）"),
    })
}

/// 校验位过 IPC 的形状。
///
/// 与 `crate::store::pools::serial::Parity` 分开：存储 crate **不依赖 specta**
/// （那是 app 钉住版本的东西），串口 crate 也**不带 serde**。两侧的映射都写成穷尽 `match`，
/// 于是任一侧加一种取值时**这里编译不过** —— 而不是悄悄少一个分支。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SerialParity {
    None,
    Even,
    Odd,
}

impl From<pool::Parity> for SerialParity {
    fn from(parity: pool::Parity) -> Self {
        match parity {
            pool::Parity::None => Self::None,
            pool::Parity::Even => Self::Even,
            pool::Parity::Odd => Self::Odd,
        }
    }
}

impl From<SerialParity> for Parity {
    fn from(parity: SerialParity) -> Self {
        match parity {
            SerialParity::None => Self::None,
            SerialParity::Even => Self::Even,
            SerialParity::Odd => Self::Odd,
        }
    }
}

/// 流控过 IPC 的形状。理由同 [`SerialParity`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SerialFlow {
    None,
    Software,
    Hardware,
}

impl From<pool::Flow> for SerialFlow {
    fn from(flow: pool::Flow) -> Self {
        match flow {
            pool::Flow::None => Self::None,
            pool::Flow::Software => Self::Software,
            pool::Flow::Hardware => Self::Hardware,
        }
    }
}

impl From<SerialFlow> for Flow {
    fn from(flow: SerialFlow) -> Self {
        match flow {
            SerialFlow::None => Self::None,
            SerialFlow::Software => Self::Software,
            SerialFlow::Hardware => Self::Hardware,
        }
    }
}

/// 界面看得见的一条串口配置。
///
/// `data_bits` / `stop_bits` 保持**库里的原始数值**（`5..=8` / `1..=2`）：取值范围由库的
/// `CHECK` 与 `TryFrom` 两头夹着，而这一层多做一个枚举只会让"越界取值"多一个说不清来路的地方。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SerialEntry {
    /// 池里的行 id。
    pub id: SerialId,
    /// 用户给这条配置起的名字（池里唯一）。标签页标题用它。
    pub name: String,
    /// 设备路径（`/dev/ttyUSB0` / `COM3`）。
    pub port: String,
    pub baud: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: SerialParity,
    pub flow: SerialFlow,
}

/// 库里 serial 配置池的全部行（按名字排序 —— 顺序确定，界面才不会每次刷新换一个样）。
///
/// 库锁着 → [`VaultError::Locked`]：**只有"列出来"这一步需要解锁**（池在库里）；
/// 真正打开设备的那条命令不碰库。
#[tauri::command]
#[specta::specta]
pub fn vault_serials(vault: State<'_, Vault>) -> Result<Vec<SerialEntry>, VaultError> {
    let rows = vault
        .with_conn(pool::serials)
        .map_err(VaultError::from_conn)?;
    rows.into_iter()
        .map(|row| {
            Ok(SerialEntry {
                id: serial_id(row.id)?,
                name: row.name,
                port: row.port,
                baud: row.baud,
                data_bits: row.data_bits,
                stop_bits: row.stop_bits,
                parity: row.parity.into(),
                flow: row.flow.into(),
            })
        })
        .collect()
}

/// 一条端口的**过 IPC 表示**。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SerialPort {
    /// 设备路径（Unix 上是 `/dev/ttyUSB0` 一类，Windows 上是 `COM3`）。
    pub path: String,
    /// 这条路径由什么硬件暴露。
    pub kind: SerialPortKind,
}

/// 端口的硬件类别过 IPC 的形状。理由同 [`SerialParity`]：`serial` 不带 serde / specta，
/// 两侧各认自己的类型，映射写成穷尽 `match`。
///
/// `Usb` 的五项与 `Windows` 的两项**都可能缺**（设备自己没报、udev 的硬件库也没有、
/// 注册表里那个值不存在）：缺了就是 `None`，不填假值（`AGENTS.md` §3.4 的"字段值不得虚构"）。
///
/// ⚠️ `Windows` 这一档**只在 Windows 上产生**（plan 0803）：取值只来自那边的注册表。
/// 其余平台上它也还在类型里 —— 前端因此只有一套形状，不必按平台分支。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SerialPortKind {
    /// USB 转串口。
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
    /// **Windows 专有的一档**：注册表的 PnP 设备项给出的描述（plan 0803）。
    ///
    /// 上游在 Windows 上把一切都报成 `Unknown`（见 `serial::enumerate` 的模块文档），
    /// 所以那边的端口只有走这一档才带得出描述。两项都可能缺。
    Windows {
        /// 设备描述（例如 `USB-SERIAL CH340 (COM3)`）。
        ///
        /// ⚠️ **必须逐字段写 `rename`**：枚举上的 `rename_all` 只改**变体名**，
        /// 不改结构体变体里的字段名（与 `SerialEntry` 那种普通结构体不同）。
        /// 漏掉它，生成物里就是 `friendly_name`，而前端读的是 `friendlyName` ——
        /// 类型都对得上，界面上却一直显示不出来。
        #[serde(rename = "friendlyName")]
        friendly_name: Option<String>,
        /// 硬件 id（例如 `USB\\VID_1A86&PID_7523`）。
        #[serde(rename = "hardwareId")]
        hardware_id: Option<String>,
    },
    /// 判定不出来。
    Unknown,
}

impl From<PortKind> for SerialPortKind {
    fn from(kind: PortKind) -> Self {
        match kind {
            PortKind::Usb {
                vid,
                pid,
                serial,
                manufacturer,
                product,
            } => Self::Usb {
                vid,
                pid,
                serial,
                manufacturer,
                product,
            },
            PortKind::Windows {
                friendly_name,
                hardware_id,
            } => Self::Windows {
                friendly_name,
                hardware_id,
            },
            PortKind::Pci => Self::Pci,
            PortKind::Bluetooth => Self::Bluetooth,
            PortKind::Unknown => Self::Unknown,
        }
    }
}

impl From<PortInfo> for SerialPort {
    fn from(port: PortInfo) -> Self {
        Self {
            path: port.path,
            kind: port.kind.into(),
        }
    }
}

/// 本机枚举到的串口，**按路径排序**、同一路径只出现一次（顺序与去重由 `serial` 的
/// `normalize` 定）。
///
/// ⚠️ **列在这里不等于打得开**（问题 #150）：Linux 那边按 udev 设备给 devnode，**不检查**它在
/// `/dev` 下是否存在。所以这条命令回答的是"系统认为有哪些端口"，而"能不能开"只有
/// [`open_serial_session`] 知道 —— 界面因此**不得**据这张表挡掉手输路径。
///
/// 空表是正常结果（本机没有串口，或者运行期拿不到 `libudev` 上下文）；只有系统调用失败才是
/// [`SerialIpcError::Enumerate`] —— "没有端口"与"列不出来"是两件事。
#[tauri::command]
#[specta::specta]
pub fn serial_ports() -> Result<Vec<SerialPort>, SerialIpcError> {
    let found = crate::serial::ports().map_err(SerialIpcError::from)?;
    Ok(found.into_iter().map(SerialPort::from).collect())
}

/// IPC 边界的串口错误。变体按**用户的下一步动作**分（同 `VaultError` / `SshIpcError` 的原则）。
#[derive(Debug, thiserror::Error, Serialize, specta::Type)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum SerialIpcError {
    /// 参数不合法。报的是**字段与取值** —— 用户要改的是那一个字段（池行里那一格，
    /// 或者 plan 1102 的表单上那一栏）。
    #[error("串口参数不合法：{field} = {value}")]
    Settings { field: String, value: String },

    /// 打开这个设备失败。**路径在这里**，因为用户要去看的是那个设备、不是我们代码里的哪一行
    /// （与 `serial` 的 `SerialError::Open` / `Handle` 同一条口径）。
    #[error("串口打不开：{path}（{message}）")]
    Open { path: String, message: String },

    /// 列不出本机端口（系统调用失败）。**用户的下一步动作是手输一条路径** ——
    /// 界面在这一档旁边留着那条输入即可，不该把它读成"本机没有串口"
    /// （空表才是那个意思，见 [`serial_ports`]）。
    #[error("列不出本机串口端口：{message}")]
    Enumerate { message: String },

    /// 内部状态不可用（会话表中毒、收尾线程起不来、频道句柄无效）。
    #[error("内部状态不可用：{message}")]
    Internal { message: String },
}

/// `serial` 的说法 → 这一侧。
///
/// ⚠️ `Open` / `Handle` 合并成一档（用户的下一步动作相同：去看那个设备），
/// 两者的区别留在 `message` 里（原文来自上游）。
impl From<SerialError> for SerialIpcError {
    fn from(err: SerialError) -> Self {
        match err {
            // 这一支**不带取值**，所以调用点会用真实输入替换它（见 [`open`]）。
            SerialError::NoPath => Self::Settings {
                field: "port".to_owned(),
                value: String::new(),
            },
            SerialError::Settings { field, value } => Self::Settings {
                field: field.to_owned(),
                value,
            },
            SerialError::Open { path, source } => Self::Open {
                path,
                message: source.to_string(),
            },
            SerialError::Handle { path, source } => Self::Open {
                path,
                message: format!("句柄不可用（{source}）"),
            },
            // 枚举失败**不挂 `Internal`**：那一档说的是"我们自己的状态坏了"，
            // 而这里坏的是系统里的那一次调用，用户能做的也不是重启 app 而是手输路径。
            SerialError::Enumerate { source } => Self::Enumerate {
                message: source.to_string(),
            },
            // 设备消失**到不了这里**：它发生在会话已经开起来之后，由读端记下来、经
            // `session_ended` 的文案交给界面（plan 1103），没有任何命令返回它。
            // 留这一支是因为这是一张共用的表：漏了它就是"契约多一档、这里少一个分支"。
            SerialError::DeviceGone { path, description } => Self::Internal {
                message: format!("{path}：{description}"),
            },
        }
    }
}

impl From<IpcError> for SerialIpcError {
    fn from(err: IpcError) -> Self {
        Self::Internal {
            message: err.to_string(),
        }
    }
}

/// 开一个串口会话要的**全部参数**（过 IPC 的形态）。
///
/// 为什么是一个结构体而不是六个平铺参数：一来六个参数加 `app` / `webview` / `channel`
/// 会越过 clippy 的上限，二来它本来就是**一个概念**（"用这套参数开一个设备"）——
/// 生成物里因此只有一项，前端把池行原样填进来即可。
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SerialParams {
    /// 设备路径（`/dev/ttyUSB0` / `COM3`）。
    pub port: String,
    pub baud: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: SerialParity,
    pub flow: SerialFlow,
}

/// 参数 → 载体要的形态。
///
/// 两个 `TryFrom` 就是"越界取值报字段与取值"那条判据的落点：取值域写在 `DataBits` /
/// `StopBits` 里（与库的 `CHECK` 对齐），这里不重复一份。
fn settings(params: SerialParams) -> Result<SerialSettings, SerialIpcError> {
    Ok(SerialSettings {
        path: params.port,
        baud: params.baud,
        data_bits: DataBits::try_from(params.data_bits).map_err(SerialIpcError::from)?,
        stop_bits: StopBits::try_from(params.stop_bits).map_err(SerialIpcError::from)?,
        parity: params.parity.into(),
        flow: params.flow.into(),
    })
}

/// 打开设备，并把失败翻成这一侧的说法。
///
/// `NoPath` 是唯一一个**不带取值**的失败（它只说"路径是空的"），而用户要改的正是他刚给的那一串
/// —— 所以这一支在**这里有值可用**的地方补上，其余一律照搬。
fn open(settings: &SerialSettings) -> Result<SerialTransport, SerialIpcError> {
    SerialTransport::open(settings).map_err(|err| match err {
        SerialError::NoPath => SerialIpcError::Settings {
            field: "port".to_owned(),
            value: settings.path.clone(),
        },
        other => SerialIpcError::from(other),
    })
}

/// 打开一个串口会话，输出经 `channel` 以 **raw 字节**送出。
///
/// 与 [`crate::session::open_session`] 的关系：**同一条尾巴**（注册 → 频道 → 收尾线程），
/// 差的是载体怎么来 —— 本地那条是 `PtyTransport::spawn_default()`，这条是"照这六个字段开一个设备"。
///
/// ⚠️ 设备被拔掉时读端报错、那条流结束、会话**自己结束**（`retire` → `session_ended`）
/// —— 这条路径与 local / SSH 是同一条。**"原因可读"不在本条**（plan 1103）。
#[tauri::command]
#[specta::specta]
pub fn open_serial_session(
    app: AppHandle,
    webview: Webview,
    channel: RawChannel,
    params: SerialParams,
) -> Result<SessionHandle, SerialIpcError> {
    let channel = channel.into_channel(webview)?;
    let settings = settings(params)?;
    let transport = open(&settings)?;

    tracing::info!(
        port = settings.path.as_str(),
        baud = settings.baud,
        "serial session opening"
    );

    let sessions = app.state::<Sessions>();
    session::open_terminal(&app, &sessions, transport, channel).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    /// 一组"常用"参数，只改要断言的那一栏 —— 让每条用例只想一件事。
    fn params() -> SerialParams {
        SerialParams {
            port: "/dev/ttyUSB0".to_owned(),
            baud: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: SerialParity::None,
            flow: SerialFlow::None,
        }
    }

    #[test]
    fn every_pool_value_maps_to_a_setting() {
        // 池的取值域是封闭的（两个枚举 + 库的 CHECK）：**每一个**都必须有去处。
        for parity in pool::Parity::ALL {
            let _: Parity = SerialParity::from(parity).into();
        }
        for flow in pool::Flow::ALL {
            let _: Flow = SerialFlow::from(flow).into();
        }
    }

    #[test]
    fn the_six_fields_become_the_settings_the_transport_takes() {
        let settings = settings(SerialParams {
            data_bits: 7,
            stop_bits: 2,
            parity: SerialParity::Even,
            flow: SerialFlow::Software,
            ..params()
        })
        .unwrap();
        assert_eq!(settings.path, "/dev/ttyUSB0");
        assert_eq!(settings.baud, 115_200);
        assert_eq!(settings.data_bits, DataBits::Seven);
        assert_eq!(settings.stop_bits, StopBits::Two);
        assert_eq!(settings.parity, Parity::Even);
        assert_eq!(settings.flow, Flow::Software);
    }

    #[test]
    fn an_out_of_range_value_names_the_field_and_the_value() {
        // 取值越界要能直接指到那一栏：库里本该拦住它，而 plan 1102 的表单可以手输。
        for raw in [0u8, 4, 9] {
            let err = settings(SerialParams {
                data_bits: raw,
                ..params()
            })
            .unwrap_err();
            let rendered = err.to_string();
            assert!(
                rendered.contains("data_bits"),
                "错误里没有字段名：{rendered}"
            );
            assert!(
                rendered.contains(&raw.to_string()),
                "错误里没有取值：{rendered}"
            );
        }
        for raw in [0u8, 3, 9] {
            let err = settings(SerialParams {
                stop_bits: raw,
                ..params()
            })
            .unwrap_err();
            let rendered = err.to_string();
            assert!(
                rendered.contains("stop_bits"),
                "错误里没有字段名：{rendered}"
            );
            assert!(
                rendered.contains(&raw.to_string()),
                "错误里没有取值：{rendered}"
            );
        }
    }

    #[test]
    fn an_empty_path_is_rejected_with_the_value_the_user_gave() {
        // 空路径是"碰设备之前"就能判定的一条，而报错里要带上用户给的那一串
        // （`NoPath` 本身不带取值，所以这一支的证据在调用点上）。
        let settings = settings(SerialParams {
            port: "   ".to_owned(),
            ..params()
        })
        .unwrap();
        // 载体不实现 `Debug`（它持有句柄），所以这里不能用 `unwrap_err`。
        let err = match open(&settings) {
            Ok(_) => panic!("一个空路径竟然打开了设备"),
            Err(err) => err,
        };
        assert!(
            matches!(&err, SerialIpcError::Settings { field, value } if field == "port" && value == "   "),
            "{err:?}"
        );
    }

    #[test]
    fn a_missing_device_reports_the_path_it_tried() {
        // 打不开是**这条路上最常见的一类失败**（设备不在 / 没权限 / 被占用），
        // 报错里必须有那个路径。用一个几乎不可能存在的名字，不碰任何真实设备。
        let settings = SerialSettings::new("/dev/serial-1101-does-not-exist", 9600);
        let err = match open(&settings) {
            Ok(_) => panic!("一个不存在的设备竟然打开了"),
            Err(err) => err,
        };
        let rendered = err.to_string();
        assert!(
            rendered.contains("/dev/serial-1101-does-not-exist"),
            "错误里没有路径：{rendered}"
        );
        assert!(matches!(err, SerialIpcError::Open { .. }), "{err:?}");
    }

    #[test]
    fn every_port_kind_maps_to_an_ipc_kind() {
        // 五档都要有去处，`Usb` 的五项一个不丢（装反了就是 vid / pid 互换）。
        let usb = PortInfo {
            path: "/dev/ttyUSB0".to_owned(),
            kind: PortKind::Usb {
                vid: 0x1a86,
                pid: 0x7523,
                serial: Some("SERIAL".to_owned()),
                manufacturer: Some("MANUFACTURER".to_owned()),
                product: Some("PRODUCT".to_owned()),
            },
        };
        let mapped = SerialPort::from(usb);
        assert_eq!(mapped.path, "/dev/ttyUSB0");
        match mapped.kind {
            SerialPortKind::Usb {
                vid,
                pid,
                serial,
                manufacturer,
                product,
            } => {
                assert_eq!((vid, pid), (0x1a86, 0x7523), "vid / pid 装反了");
                assert_eq!(serial.as_deref(), Some("SERIAL"));
                assert_eq!(manufacturer.as_deref(), Some("MANUFACTURER"));
                assert_eq!(product.as_deref(), Some("PRODUCT"));
            }
            other => panic!("USB 端口映射成了别的一档：{other:?}"),
        }

        for (kind, expected) in [
            (PortKind::Pci, SerialPortKind::Pci),
            (PortKind::Bluetooth, SerialPortKind::Bluetooth),
            (PortKind::Unknown, SerialPortKind::Unknown),
        ] {
            let mapped = SerialPort::from(PortInfo {
                path: "/dev/x".to_owned(),
                kind,
            });
            assert_eq!(mapped.kind, expected);
        }
    }

    #[test]
    fn a_windows_port_carries_its_description_across_the_boundary() {
        // 这一档的全部意义就是那两个字符串（plan 0803）：装反或者丢掉一项，
        // 界面上就会出现一台“看得见名字、看不见是什么”的设备 —— 而那正是上游在
        // Windows 上的形态，本 plan 要消掉的正是它。
        //
        // ⚠️ 这一条在**任何平台**上都跑得起来：构造一个值不需要 Windows，
        // 只有**产生**它才需要（`serial::enumerate` 的注册表那条路）。
        // 所以映射这一半的证据是全平台共用的，跨平台的那一半由 E2E `windows_ports` 给。
        // 硬件 id 里的分隔符就是两个反斜杠（注册表里的形态），逐字符写出来。
        // ⚠️ 名字不能与下面解构出来的那一栏同名：同名会让断言引用到解构的那一个，
        // 于是变成一条"装反了也通过"的假断言（这一处踩过）。
        let expected_hardware_id = "USB\\VID_1A86&PID_7523".to_owned();
        let mapped = SerialPort::from(PortInfo {
            path: "COM7".to_owned(),
            kind: PortKind::Windows {
                friendly_name: Some("USB-SERIAL CH340 (COM7)".to_owned()),
                hardware_id: Some(expected_hardware_id.clone()),
            },
        });
        assert_eq!(mapped.path, "COM7");
        match mapped.kind {
            SerialPortKind::Windows {
                friendly_name,
                hardware_id,
            } => {
                assert_eq!(friendly_name.as_deref(), Some("USB-SERIAL CH340 (COM7)"));
                assert_eq!(hardware_id.as_deref(), Some(expected_hardware_id.as_str()));
            }
            other => panic!("Windows 端口映射成了别的一档：{other:?}"),
        }

        // 两项都缺时留在原地：不填空串顶替（`AGENTS.md` §3.4）。
        let mapped = SerialPort::from(PortInfo {
            path: "COM8".to_owned(),
            kind: PortKind::Windows {
                friendly_name: None,
                hardware_id: None,
            },
        });
        assert_eq!(
            mapped.kind,
            SerialPortKind::Windows {
                friendly_name: None,
                hardware_id: None,
            }
        );
    }

    #[test]
    fn a_usb_port_without_descriptions_keeps_them_empty() {
        // 五项都可能缺（设备自己没报、udev 的硬件库也没有）：缺了就是 `None`，
        // 不填 0 / unknown 顶替 —— 界面据此少显示一行，而不是显示一句假话。
        let mapped = SerialPort::from(PortInfo {
            path: "/dev/ttyUSB1".to_owned(),
            kind: PortKind::Usb {
                vid: 0,
                pid: 0,
                serial: None,
                manufacturer: None,
                product: None,
            },
        });
        match mapped.kind {
            SerialPortKind::Usb {
                serial,
                manufacturer,
                product,
                ..
            } => {
                assert_eq!(serial, None);
                assert_eq!(manufacturer, None);
                assert_eq!(product, None);
            }
            other => panic!("{other:?}"),
        }
    }
}
