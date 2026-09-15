//! 串口连接参数。
//!
//! 四个枚举的取值域与 **serial 配置池**（`akasha-store` 的 `serials` 表）对齐：
//! 库里的 `CHECK` 允许 `data_bits IN (5,6,7,8)` 与 `stop_bits IN (1,2)`，
//! 校验位与流控取自固定的三值集合。**不合法的取值在这里写不出来** ——
//! 于是"9 个数据位"这类输入不会拖到打开串口时才被拒。
//!
//! 池的行 ↔ 本类型的映射（含"库里出现不合法值时怎么报"）是 plan 0802 的事。

/// 数据位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataBits {
    /// 5 位。
    Five,
    /// 6 位。
    Six,
    /// 7 位。
    Seven,
    /// 8 位。
    Eight,
}

/// 停止位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopBits {
    /// 1 位。
    One,
    /// 2 位。
    Two,
}

/// 校验位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    /// 不校验。
    None,
    /// 偶校验。
    Even,
    /// 奇校验。
    Odd,
}

/// 流控。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// 没有流控。
    None,
    /// XON/XOFF（软件流控）。
    Software,
    /// RTS/CTS（硬件流控）。
    Hardware,
}

/// 打开一个串口要的全部参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialSettings {
    /// 设备路径（Unix 上是 `/dev/ttyUSB0` 一类，Windows 上是 `COM3`）。
    ///
    /// 它是**操作系统给的设备名**，不是我们的文件位置（`scope.md` §3）：
    /// 换一台机器它可能不存在，但那不是"移动数据目录之后才失效"。
    pub path: String,
    /// 波特率。
    ///
    /// **不限制成标准档位**：现实里有非标准波特率的设备，把它挡在外面等于让用户换一个工具 ——
    /// 这与 serial 配置池的取值域一致（那里的 `CHECK` 只要求 `baud > 0`）。
    pub baud: u32,
    /// 数据位。
    pub data_bits: DataBits,
    /// 停止位。
    pub stop_bits: StopBits,
    /// 校验位。
    pub parity: Parity,
    /// 流控。
    pub flow: Flow,
}

impl SerialSettings {
    /// 一条设备路径 + 波特率，其余取最常见的取值（8 数据位、1 停止位、不校验、无流控）。
    pub fn new(path: impl Into<String>, baud: u32) -> Self {
        Self {
            path: path.into(),
            baud,
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow: Flow::None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn the_common_framing_is_8n1_without_flow_control() {
        // "常用默认"是一条对外承诺（plan 0802 的参数界面会以它为初值），所以钉在类型层。
        let settings = SerialSettings::new("/dev/ttyUSB0", 115200);
        assert_eq!(settings.data_bits, DataBits::Eight);
        assert_eq!(settings.stop_bits, StopBits::One);
        assert_eq!(settings.parity, Parity::None);
        assert_eq!(settings.flow, Flow::None);
        assert_eq!(settings.baud, 115200);
    }
}
