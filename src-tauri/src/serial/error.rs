//! 本 crate 的错误分域（`AGENTS.md` §3.4）。
//!
//! 三件事各有各的读法，所以分成三组：
//!
//! - `Enumerate`：**列出端口**这一步失败。它与"本机没有端口"是两回事 —— 后者是空表；
//! - `Settings` / `NoPath`：**参数**不合法。两者都在碰设备之前就能判定，
//!   报的是**字段名与取值**（用户要改的是那一个字段）；
//! - `Open` / `Handle`：**打开这一个具体设备**失败。两者都带**路径** ——
//!   用户要去看的是那个设备，不是我们代码里的哪一行（同 `ssh` 的 `SshError::File`）。
//! - `DeviceGone`：设备**开着的时候**没了。它不改变任何一次调用的结果，而是解释这条会话
//!   为什么结束（plan 1103）—— 「会话结束」这件事此前只能表现为"什么原因都没有"。

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
    /// 本来开着，现在设备没了（被拔掉 / 对端消失）。
    ///
    /// 与 [`SerialError::Open`] 是两回事：那个说的是"打不开"，这个说的是"开着开着没了"——
    /// 后者用户的下一步动作是"重新打开一条新会话"，不是"去看参数"。
    ///
    /// ⚠️ 这里带的是**描述**而不是 `#[source]`：那个 `io::Error` 由读端产生，
    /// 而读端必须把它原样交回上游的读循环（那里对错误与 EOF 一视同仁），
    /// 于是留下的只能是它的一份文本快照（`io::Error` 不能克隆）。
    #[error("串口设备已断开：{path}（{description}）")]
    DeviceGone {
        /// 设备路径。
        path: String,
        /// 读端那个错误的可读描述（`io::Error` 的 `Display`）。
        description: String,
    },
}
