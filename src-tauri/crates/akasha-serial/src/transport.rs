//! 串口载体：打开、写、读、收尾。

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use akasha_pty::{Capabilities, ExitStatus, Transport, TransportError};
use serialport::SerialPort;

use crate::error::SerialError;
use crate::settings::{DataBits, Flow, Parity, SerialSettings, StopBits};

/// 读端多久醒一次看停止标志。
///
/// 它同时是 `read` 至多阻塞多久，因此也是 [`SerialTransport::shutdown`] 最坏多晚被读端
/// 看见的上界。⚠️ **不能是 0**：0 让每次 `read` 立刻超时，读循环就变成忙等。
const READ_TICK: Duration = Duration::from_millis(100);

/// 串口载体。
///
/// 两个句柄：一个写，一个交给 [`Transport::output_stream`] 读。**读端只能取走一次** ——
/// 这是 trait 的契约（两个读端会互相偷字节），实现方式是把读端存成 `Option`。
pub struct SerialTransport {
    /// 写端，也是收尾时 flush 的那个。
    port: Box<dyn SerialPort>,
    /// 读端。`None` = 已经被 `output_stream()` 取走。
    reader: Option<SerialReader<Box<dyn SerialPort>>>,
    /// 停止标志，与读端共享。
    stop: Arc<AtomicBool>,
    /// 读端那个错误的**文本快照**，与读端共享（见 [`SerialReader::failure`]）。
    failure: Arc<Mutex<Option<String>>>,
    /// 是否已经收尾。`shutdown` 幂等，且收尾之后的写必须被拒。
    closed: bool,
}

impl SerialTransport {
    /// 串口的能力位：**什么都没有**。
    ///
    /// 没有窗口尺寸（`resize` 走 trait 的默认实现，答 `Unsupported`）、没有退出结局
    /// （`exited()` 恒为 `Ok(None)`）、本地没有进程（`session_leader()` 是 `None`，
    /// 看门狗没有可回收的东西）。`scope.md` §2 的表格就是这三条。
    ///
    /// 写成关联常量而不是方法体里的字面量：它是本 crate 的公共事实，
    /// 单测可以直接断言它，不必先打开一个真设备。
    pub const CAPABILITIES: Capabilities = Capabilities::NONE;

    /// 按参数打开一个串口。
    ///
    /// 路径是**显式**的（见模块文档）：不依赖端口枚举，所以枚举不可用时这条路照常可用。
    pub fn open(settings: &SerialSettings) -> Result<Self, SerialError> {
        // 能在碰设备之前判定的都在这里（空路径与波特率 0）：报出字段与取值，
        // 而不是等 OS 给出一个只有它看得懂的原因。
        settings.validate()?;
        let port = serialport::new(&settings.path, settings.baud)
            .data_bits(map_data_bits(settings.data_bits))
            .stop_bits(map_stop_bits(settings.stop_bits))
            .parity(map_parity(settings.parity))
            .flow_control(map_flow(settings.flow))
            // 读端靠这个超时"醒来看一眼停止标志"（见 READ_TICK）。
            .timeout(READ_TICK)
            .open()
            .map_err(|source| SerialError::Open {
                path: settings.path.clone(),
                source,
            })?;
        // 第二个句柄在**打开时**就复制出来：`output_stream` 只能返回 `Option`，
        // 把"复制失败"混进"已经被取走"，会让调用方看到的 None 有两种截然不同的含义。
        let read_half = port.try_clone().map_err(|source| SerialError::Handle {
            path: settings.path.clone(),
            source,
        })?;
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        Ok(Self {
            port,
            reader: Some(SerialReader {
                source: read_half,
                stop: Arc::clone(&stop),
                // 路径交给读端：报错里要说清是**哪一台设备**（同 `SerialError::Open` 的理由）。
                path: settings.path.clone(),
                failure: Arc::clone(&failure),
            }),
            stop,
            failure,
            closed: false,
        })
    }
}

impl Transport for SerialTransport {
    fn capabilities(&self) -> Capabilities {
        Self::CAPABILITIES
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        self.port.write_all(bytes)?;
        // 串口上"写进缓冲"与"已经发出去"不是一回事：`flush` 是 `tcdrain`。
        self.port.flush()?;
        Ok(())
    }

    fn output_stream(&mut self) -> Option<Box<dyn Read + Send>> {
        self.reader
            .take()
            .map(|reader| Box::new(reader) as Box<dyn Read + Send>)
    }

    /// 读端那个错误的那句话（plan 1103）。
    ///
    /// 与 [`Transport::shutdown`] 的分工见 trait 的文档：串口没有退出码，所以设备被拔掉时
    /// 会话层唯一能说的就是这一句。收尾**之后**调用得到的是同一句话（它只增不减）——
    /// 收尾立起停止标志，读端此后给的是 EOF，不会再往这一格里写。
    fn stream_error(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn shutdown(&mut self) -> Result<Option<ExitStatus>, TransportError> {
        // 幂等（trait 的契约）：第二次调用不重做，也不报错。
        if !self.closed {
            self.closed = true;
            // 先立标志再 flush：读端最多再阻塞一个 READ_TICK 就会结束。
            self.stop.store(true, Ordering::SeqCst);
            // 尽力把已经交出去的字节发完。设备已经被拔掉时这一步会失败，
            // 而那不影响"载体已经收尾"这个事实。
            let _ = self.port.flush();
        }
        // `Ok(None)` 不是"成功"，是"这个载体没有结局可报"（串口没有退出码）。
        Ok(None)
    }
}

/// 读端：把"读超时"（设备安静）与"该收尾了"（停止标志）从字节流里分出来。
///
/// 它存在的唯一理由，是这两件事在 `Read` 的契约里没有位置：`Ok(0)` 是 EOF，
/// 而串口**没有**"对端关闭"这种事件 —— "现在没有数据"与"流结束了"必须由**我们**分开表达。
///
/// 所以：超时 → 再看一眼停止标志 → 没立起来就继续读；停止标志立起来了 → 返回 `Ok(0)`。
/// 上游（`akasha_pty::spawn_batcher`）据此交付残批并结束那条流。
struct SerialReader<S> {
    source: S,
    stop: Arc<AtomicBool>,
    /// 设备路径（报错里要说清是**哪一台**设备）。
    path: String,
    /// 遇到**真实**读错误时把 [`SerialError::DeviceGone`] 那句话写在这里，与载体共享。
    ///
    /// 为什么记在这里而不是交给上游：`spawn_batcher` 的读循环对错误与 EOF 一视同仁
    /// （它同时服务 PTY 与 SSH，那条判定不能为串口改），所以"为什么结束"出了这个结构体
    /// 就再也拿不回来了。快照成 `String` 是因为 `io::Error` 不能克隆，而它还要原样交回去。
    failure: Arc<Mutex<Option<String>>>,
}

impl<S: Read> Read for SerialReader<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Ok(0);
            }
            match self.source.read(buf) {
                // 上游的两个平台实现都用 `Err(TimedOut)` 表示超时（见 READ_TICK），
                // 所以这条不是它们的行为。留着它是因为两种误判的代价不对称：
                // 把 `Ok(0)` 当 EOF，会让"设备安静一会儿"变成"会话结束"，而且没有任何提示。
                Ok(0) => continue,
                Ok(n) => return Ok(n),
                Err(err) if is_idle(&err) => continue,
                Err(err) => {
                    // 记下来再交回去：这里是唯一见到它的地方（见 `failure` 字段）。
                    let said = SerialError::DeviceGone {
                        path: self.path.clone(),
                        description: err.to_string(),
                    }
                    .to_string();
                    *self
                        .failure
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(said);
                    return Err(err);
                }
            }
        }
    }
}

/// 这个错误说的是"现在没有数据"，而不是"流坏了"。
///
/// `Interrupted` 也算：被信号打断从来不是失败（`akasha_pty` 的读循环同样重试它）。
fn is_idle(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

/// 参数映射：本 crate 的枚举 → `serialport` 的枚举。
///
/// 把 `serialport` 的类型关在本文件里：升级它要改的是这几行，
/// 而不是 app 的命令与前端（同 `akasha-ssh` 对 `russh` 的处理）。
const fn map_data_bits(bits: DataBits) -> serialport::DataBits {
    match bits {
        DataBits::Five => serialport::DataBits::Five,
        DataBits::Six => serialport::DataBits::Six,
        DataBits::Seven => serialport::DataBits::Seven,
        DataBits::Eight => serialport::DataBits::Eight,
    }
}

/// 见 [`map_data_bits`]。
const fn map_stop_bits(bits: StopBits) -> serialport::StopBits {
    match bits {
        StopBits::One => serialport::StopBits::One,
        StopBits::Two => serialport::StopBits::Two,
    }
}

/// 见 [`map_data_bits`]。
const fn map_parity(parity: Parity) -> serialport::Parity {
    match parity {
        Parity::None => serialport::Parity::None,
        Parity::Even => serialport::Parity::Even,
        Parity::Odd => serialport::Parity::Odd,
    }
}

/// 见 [`map_data_bits`]。
const fn map_flow(flow: Flow) -> serialport::FlowControl {
    match flow {
        Flow::None => serialport::FlowControl::None,
        Flow::Software => serialport::FlowControl::Software,
        Flow::Hardware => serialport::FlowControl::Hardware,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;
    use std::collections::VecDeque;

    /// 按脚本给出读结果的假读端：给出一个 `Err` 就把它用掉，给出字节就交付它。
    struct Scripted {
        steps: VecDeque<io::Result<Vec<u8>>>,
    }

    impl Scripted {
        fn new(steps: Vec<io::Result<Vec<u8>>>) -> Self {
            Self {
                steps: steps.into(),
            }
        }
    }

    impl Read for Scripted {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.steps.pop_front() {
                Some(Ok(bytes)) => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    Ok(n)
                }
                Some(Err(err)) => Err(err),
                None => Ok(0),
            }
        }
    }

    /// 一旦被读就失败：用来证明"停止之后不再碰数据源"。
    struct NeverRead;

    impl Read for NeverRead {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            panic!("停止之后还在读数据源");
        }
    }

    /// 脚本化读端用的设备路径（断言"那句话说得清是哪台设备"）。
    const PATH: &str = "/dev/akasha-serial-probe";

    fn reader(steps: Vec<io::Result<Vec<u8>>>) -> SerialReader<Scripted> {
        SerialReader {
            source: Scripted::new(steps),
            stop: Arc::new(AtomicBool::new(false)),
            path: PATH.to_string(),
            failure: Arc::new(Mutex::new(None)),
        }
    }

    /// 读端这一刻记下的那句话（生产路径上由 [`SerialTransport::stream_error`] 读）。
    fn recorded<S>(reader: &SerialReader<S>) -> Option<String> {
        reader.failure.lock().expect("失败格锁中毒").clone()
    }

    #[test]
    fn a_read_timeout_is_not_the_end_of_the_stream() {
        // 这条是本模块存在的主要理由：串口上"设备安静"要表现为"继续等"。
        // 上游把超时给成 Err(TimedOut)（POSIX 的 poll 与 Windows 的 ReadFile 都核对过源码），
        // 读循环把它当"暂时没有数据"；若哪天有人改成"读错误即结束"，
        // 症状是串口一安静会话就结束 —— 这里就是那道拦截。
        let mut reader = reader(vec![
            Err(io::Error::from(io::ErrorKind::TimedOut)),
            Err(io::Error::new(io::ErrorKind::WouldBlock, "没有数据")),
            Ok(b"ping".to_vec()),
        ]);
        let mut buf = [0u8; 4];
        assert_eq!(reader.read(&mut buf).unwrap(), 4);
        assert_eq!(&buf, b"ping");
        // 反例：安静不是故障 —— 原因那一格必须还是空的，否则每次"设备先沉默一会儿"
        // 都会在会话结束时冒出一句不该有的原因。
        assert!(recorded(&reader).is_none());
    }

    #[test]
    fn an_interrupted_read_is_retried() {
        // 被信号打断从来不是失败：一次 EINTR 吞掉整条输出流的形态见 akasha-pty 的读循环。
        let mut reader = reader(vec![
            Err(io::Error::from(io::ErrorKind::Interrupted)),
            Ok(b"x".to_vec()),
        ]);
        let mut buf = [0u8; 1];
        assert_eq!(reader.read(&mut buf).unwrap(), 1);
    }

    #[test]
    fn a_zero_length_read_is_treated_as_idle_not_as_eof() {
        // 见 read() 里那条注释：上游不这样表示超时，但这两种误判的代价不对称，
        // 所以"没有字节"必须是"继续等"，不能是"流结束"。
        let mut reader = reader(vec![Ok(Vec::new()), Ok(b"late".to_vec())]);
        let mut buf = [0u8; 4];
        assert_eq!(reader.read(&mut buf).unwrap(), 4);
        assert_eq!(&buf, b"late");
    }

    #[test]
    fn a_real_error_still_ends_the_stream() {
        // 设备被拔掉这类错误**必须**传出去：它不仅结束这条流，
        // 也是后续"会话自己结束"那条路径的判据来源。把一切都吞掉会让设备消失变成静默停摆。
        let mut reader = reader(vec![Err(io::Error::from(io::ErrorKind::BrokenPipe))]);
        let mut buf = [0u8; 1];
        assert_eq!(
            reader.read(&mut buf).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        // 而且**要说得出是哪台设备**：会话说"结束了"，用户要知道的是哪一条路断了。
        let said = recorded(&reader).expect("真实错误必须被记下来");
        assert!(said.contains(PATH), "原因里没有设备路径：{said}");
        assert!(said.contains("已断开"), "原因里没说发生了什么：{said}");
    }

    #[test]
    fn a_stopped_reader_ends_the_stream_without_touching_the_source() {
        // 收尾路径：立起标志之后，读端立刻给出 EOF（合批器据此交付残批并结束），
        // 而且不再向设备要字节 —— 设备可能已经拔掉了。
        let mut reader = SerialReader {
            source: NeverRead,
            stop: Arc::new(AtomicBool::new(true)),
            path: PATH.to_string(),
            failure: Arc::new(Mutex::new(None)),
        };
        let mut buf = [0u8; 8];
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
        assert!(
            recorded(&reader).is_none(),
            "我们自己收尾不是设备故障：那条 EOF 不该记成原因"
        );
    }

    /// 拔掉设备（关掉 PTY 主端）时的完整形态：读端报错 + 那句话说得清是哪台设备。
    ///
    /// 它同时是 plan 1103「前置检查」留下的证据：探针实测"从端那一路的 `read` 立刻返回
    /// `BrokenPipe`，**不是** `Ok(0)`" —— 后者会被 [`SerialReader`] 当成"设备安静"继续等，
    /// 于是这条会话永远不结束。上游为什么是 `Err`：`serialport` 的 POSIX 读端先 `poll`，
    /// 见到 `POLLHUP` / `POLLNVAL` 报 `BrokenPipe`，否则报 `Other(EIO)` —— 真设备被拔掉
    /// （EIO 那一支）与这里走的是同一个判定。
    ///
    /// ⚠️ **只在 Linux 上**：这条脚手架把 PTY 从端当串口打开，而 macOS 走不到断言 ——
    /// `open` 就报 `Not a typewriter`（上游对 pty 设波特率用 `IOSSIOSPEED`，返回 `ENOTTY`，
    /// 见同文件里 `a_pseudo_terminal_…` 的说明）。真设备不受影响。
    #[cfg(target_os = "linux")]
    #[test]
    fn an_unplugged_device_says_why_the_stream_ended() {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("造一对 PTY 失败");
        let path = pair
            .master
            .tty_name()
            .expect("拿不到 PTY 从端的设备名")
            .to_string_lossy()
            .into_owned();
        let mut transport = SerialTransport::open(&SerialSettings::new(path.clone(), 115200))
            .expect("把 PTY 从端当串口打开失败");
        let mut output = transport.output_stream().expect("取输出流失败");
        assert!(transport.stream_error().is_none(), "设备还在时不该有原因");

        // 关掉主端 = 拔掉设备。
        drop(pair);
        let mut buf = [0u8; 8];
        assert!(
            output.read(&mut buf).is_err(),
            "拔掉设备必须是 Err：Ok(0) 会被读端当成\"设备安静\"，这条会话就永远不结束"
        );

        let said = transport
            .stream_error()
            .expect("读端失败之后必须说得出原因");
        // 两样都要有：**哪台设备**（用户要去看的那个东西）与**发生了什么**。
        assert!(said.contains(&path), "原因里没有设备路径：{said}");
        assert!(said.contains("已断开"), "原因里没说发生了什么：{said}");
        // 问几次都是同一句话：收尾之后读端给的是 EOF，不会把这一格改掉。
        assert_eq!(transport.stream_error().as_deref(), Some(said.as_str()));
        assert_eq!(transport.shutdown().expect("收尾失败"), None);
        assert_eq!(transport.stream_error().as_deref(), Some(said.as_str()));
    }

    #[test]
    fn capabilities_are_none() {
        // 能力位是数据类型：serial 没有窗口尺寸、没有退出结局（scope.md §2）。
        //
        // ⚠️ 两个常量之间的断言要放进 const 块：写在外面会被 clippy 判
        // `assertions_on_constants`（问题 #83），而 `just clippy` 带 `-D warnings`。
        // 放进去之后它还是同一句断言，只是改在编译期求值。
        const {
            assert!(!SerialTransport::CAPABILITIES.resize);
            assert!(!SerialTransport::CAPABILITIES.exit_status);
        }
        assert_eq!(SerialTransport::CAPABILITIES, Capabilities::NONE);
    }

    #[test]
    fn an_empty_path_is_rejected_before_anything_is_opened() {
        // 手动指定路径是这条路唯一的输入：空值要在**碰设备之前**就被说清楚。
        let settings = SerialSettings::new("   ", 9600);
        assert!(matches!(
            SerialTransport::open(&settings),
            Err(SerialError::NoPath)
        ));
    }

    #[test]
    fn a_missing_device_reports_the_path_it_tried() {
        // 报错里必须有路径：用户要去看的是那个设备，而不是我们代码里的哪一行
        // （同 akasha-ssh 的 SshError::File）。
        let settings = SerialSettings::new("/dev/akasha-serial-does-not-exist", 9600);
        // 用 match 而不是 unwrap_err：载体本身不（也不该）实现 Debug ——
        // 它持有的是句柄，而句柄没有可读的表示。
        let err = match SerialTransport::open(&settings) {
            Ok(_) => panic!("一个不存在的设备竟然打开了"),
            Err(err) => err,
        };
        let rendered = err.to_string();
        assert!(
            rendered.contains("/dev/akasha-serial-does-not-exist"),
            "错误里没有路径：{rendered}"
        );
        assert!(matches!(err, SerialError::Open { .. }));
    }

    /// 真 tty 上的回读：内核留得住的那几项必须等于请求值。
    ///
    /// ⚠️ **只在 Linux 上**：macOS 上根本走不到断言 —— `open` 就失败（`Not a typewriter`）。
    /// 原因是上游 `serialport` 在 Apple 目标上**用 `IOSSIOSPEED` 设波特率**，而它对 pty 返回
    /// `ENOTTY`（`serialport-4.10.1/src/posix/termios.rs` 自己写着这一条：attempting to set the
    /// baud rate on a pseudo terminal via this ioctl call will fail with the ENOTTY error），
    /// 于是"把 PTY 从端当串口"这条脚手架在 macOS 上不成立。真串口设备不受影响（它们接受这个
    /// ioctl）。可移植的判据仍由映射的全量单测（`settings.rs` 的 `TryFrom`）守着。
    #[cfg(target_os = "linux")]
    #[test]
    fn a_pseudo_terminal_reports_the_parameters_it_can_hold() {
        // ⚠️ 这里**只**断言波特率 / 停止位 / 流控。数据位与校验位在 PTY 上会被归一化
        // （实测：请求 7 数据位回读 8 位、请求偶校验回读不校验），在 PTY 上断言那两项
        // 只会得到一条"用例对、被测代码无从判断"的断言 —— 它们的证据是映射的全量单测
        // （settings.rs 的 `TryFrom`）与真机（plan 0802 的「待验证」）。
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("造一对 PTY 失败");
        // ⚠️ 设备名在**主端**上问（`tty_name` 是 `MasterPty` 的方法）。
        let path = pair.master.tty_name().expect("拿不到 PTY 从端的设备名");
        let mut settings = SerialSettings::new(path.to_string_lossy(), 115200);
        settings.data_bits = DataBits::Seven;
        settings.stop_bits = StopBits::Two;
        settings.parity = Parity::Even;
        settings.flow = Flow::Software;

        let transport = SerialTransport::open(&settings).expect("把 PTY 从端当串口打开失败");
        assert_eq!(transport.port.baud_rate().unwrap(), 115200);
        assert_eq!(
            transport.port.stop_bits().unwrap(),
            serialport::StopBits::Two
        );
        assert_eq!(
            transport.port.flow_control().unwrap(),
            serialport::FlowControl::Software
        );
        // 主端要活到断言结束：从端那一路的 fd 全关之后，从端就不存在了。
        drop(pair);
    }
}
