//! 本 crate 的错误分域（`AGENTS.md` §3.4）。
//!
//! 三件事各有各的读法，所以分成三组：
//!
//! - `Enumerate`：**列出端口**这一步失败。它与"本机没有端口"是两回事 —— 后者是空表；
//! - `Settings` / `NoPath`：**参数**不合法。两者都在碰设备之前就能判定，
//!   报的是**字段名与取值**（用户要改的是那一个字段）；
//! - `Open` / `Handle`：**打开这一个具体设备**失败。两者都带**路径** ——
//!   用户要去看的是那个设备，不是我们代码里的哪一行（同 `akasha-ssh` 的 `SshError::File`）。

/// 串口的失败。
#[derive(Debug, thiserror::Error)]
pub enum SerialError {
    /// 路径是空的。手动指定路径是这条路唯一的输入，空值没有任何可尝试的东西。
    #[error("串口路径为空")]
    NoPath,
    /// 参数不合法。
    ///
    /// 字段名用的是 **serial 配置池的列名**（`baud` / `data_bits` / `stop_bits`），
    /// 便于与库里的那一行对照。
    #[error("串口参数不合法：{field} = {value}")]
    Settings {
        /// 字段名。
        field: &'static str,
        /// 实际取值。
        value: String,
    },
    /// 枚举端口失败。
    #[error("串口枚举失败（{source}）")]
    Enumerate {
        /// 上游的错误（含它的 `ErrorKind` 与可读描述）。
        #[source]
        source: serialport::Error,
    },
    /// 打不开。
    #[error("串口打不开：{path}（{source}）")]
    Open {
        /// 设备路径（原样回显，便于对照用户的输入）。
        path: String,
        /// 上游的错误（含它的 `ErrorKind` 与可读描述）。
        #[source]
        source: serialport::Error,
    },
    /// 打开了，但复制不出第二个句柄。
    #[error("串口句柄不可用：{path}（{source}）")]
    Handle {
        /// 设备路径。
        path: String,
        /// 上游的错误。
        #[source]
        source: serialport::Error,
    },
}
