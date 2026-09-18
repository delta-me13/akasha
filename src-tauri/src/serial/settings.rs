//! 串口连接参数。
//!
//! 四个枚举的取值域与 **serial 配置池**（`store` 的 `serials` 表）对齐：
//! 库里的 `CHECK` 允许 `data_bits IN (5,6,7,8)` 与 `stop_bits IN (1,2)`，
//! 校验位与流控取自固定的三值集合。**不合法的取值在这里写不出来** ——
//! 于是"9 个数据位"这类输入不会拖到打开串口时才被拒。
//!
//! 库里的行是本类型的**原始取值来源**：往里搬字段时用 `DataBits::try_from` /
//! `StopBits::try_from`，越界值在那里被拒并报出字段与取值（plan 0802）。
//! "哪一行、哪个字段"的搬运属于 app。

use crate::serial::error::SerialError;

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

impl TryFrom<u8> for DataBits {
    type Error = SerialError;

    /// 从 serial 配置池里的取值还原。
    ///
    /// 取值域与库的 `CHECK`（`data_bits IN (5,6,7,8)`）一致。越界值意味着那一行坏了
    /// （库本该拦住它），所以报错里带上**实际取值**：用户要改的是那一行那个字段。
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            5 => Ok(Self::Five),
            6 => Ok(Self::Six),
            7 => Ok(Self::Seven),
            8 => Ok(Self::Eight),
            other => Err(SerialError::Settings {
                field: "data_bits",
                value: other.to_string(),
            }),
        }
    }
}

/// 停止位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopBits {
    /// 1 位。
    One,
    /// 2 位。
    Two,
}

impl TryFrom<u8> for StopBits {
    type Error = SerialError;

    /// 见 [`DataBits::try_from`]：取值域与库的 `CHECK`（`stop_bits IN (1,2)`）一致。
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            other => Err(SerialError::Settings {
                field: "stop_bits",
                value: other.to_string(),
            }),
        }
    }
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

    /// 在碰设备之前能判定的部分。
    ///
    /// 只查两件：
    ///
    /// - **路径非空**：手动指定路径是这条路唯一的输入，空值没有任何可尝试的东西；
    /// - **波特率不为 0**：上游把 0 当成"不要设置波特率"的暗号（它给 PTY 用），而 serial 配置池
    ///   的 `CHECK` 要求 `baud > 0` —— 产品里没有它的位置，留着只会变成一个
    ///   **静默不生效**的参数。
    ///
    /// 其余的取值不合法在类型层就写不出来（四个枚举的取值域是封闭的）。
    pub fn validate(&self) -> Result<(), SerialError> {
        if self.path.trim().is_empty() {
            return Err(SerialError::NoPath);
        }
        if self.baud == 0 {
            return Err(SerialError::Settings {
                field: "baud",
                value: "0".to_string(),
            });
        }
        Ok(())
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

    #[test]
    fn every_value_the_pool_allows_round_trips() {
        // 池的 `CHECK` 允许 5..=8 与 1..=2；这个入口就是"从库里搬过来"的那一步。
        for (raw, expected) in [
            (5u8, DataBits::Five),
            (6, DataBits::Six),
            (7, DataBits::Seven),
            (8, DataBits::Eight),
        ] {
            assert_eq!(DataBits::try_from(raw).unwrap(), expected);
        }
        for (raw, expected) in [(1u8, StopBits::One), (2, StopBits::Two)] {
            assert_eq!(StopBits::try_from(raw).unwrap(), expected);
        }
    }

    #[test]
    fn an_out_of_range_value_names_the_field_and_the_value() {
        // 报错要能直接指到字段与取值：库里出现 9 或 3 时，用户要改的是那一行那个字段。
        for raw in [0u8, 4, 9] {
            let rendered = DataBits::try_from(raw).unwrap_err().to_string();
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
            let rendered = StopBits::try_from(raw).unwrap_err().to_string();
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
    fn an_empty_path_or_a_zero_baud_rate_is_rejected_before_anything_is_opened() {
        assert!(matches!(
            SerialSettings::new("  ", 9600).validate(),
            Err(SerialError::NoPath)
        ));
        let err = SerialSettings::new("/dev/ttyUSB0", 0)
            .validate()
            .unwrap_err();
        assert!(err.to_string().contains("baud"), "{err}");
    }
}
