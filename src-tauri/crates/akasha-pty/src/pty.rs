//! `portable-pty` 实现：本地 PTY 是 [`Transport`] 的第一个实现。

use std::fmt;
use std::io::{Read, Write};

use portable_pty::{Child, MasterPty, PtyPair, PtySize, native_pty_system};

use crate::shell::ShellLaunch;
use crate::transport::{Capabilities, ExitStatus, TerminalSize, Transport, TransportError};

/// 本地 PTY 上的一个字节载体（`AGENTS.md` §3.3 说的"PTY 子进程"就是这个）。
///
/// ⚠️ **没有 `Drop` 实现，这是刻意的**：收尾必须走 [`Transport::shutdown`]，
/// 由它显式 `kill` + `wait`。若在 `Drop` 里悄悄 kill，就再也分不清
/// "调用方收尾了"与"调用方忘了、被兜住了" —— 而后者正是要让它暴露的
/// （`AGENTS.md` §3.3：drop 不能代替显式 kill + wait）。
pub struct PtyTransport {
    /// 留着它只为 `resize`；读写两端已从它这里取走。
    master: Box<dyn MasterPty + Send>,
    /// 写端：`take_writer()` 只能取一次，所以是 `Option`。
    writer: Option<Box<dyn Write + Send>>,
    /// 读端：`output_stream()` 取走后为 `None`。
    reader: Option<Box<dyn Read + Send>>,
    child: Box<dyn Child + Send + Sync>,
    /// 收尸结果。存下来是为了 `shutdown` 幂等：`wait` 只该被调用一次。
    status: Option<ExitStatus>,
}

impl PtyTransport {
    /// 用平台默认登录 shell 起一个 PTY。
    pub fn spawn_default(size: TerminalSize) -> Result<Self, TransportError> {
        Self::spawn(&ShellLaunch::default_shell(), size)
    }

    /// 按指定的启动参数起一个 PTY。
    pub fn spawn(launch: &ShellLaunch, size: TerminalSize) -> Result<Self, TransportError> {
        let pty_system = native_pty_system();
        let PtyPair { slave, master } = pty_system
            .openpty(size.into())
            .map_err(context_err("打开 PTY 失败"))?;

        let child = slave
            .spawn_command(launch.to_command_builder())
            .map_err(context_err("启动子进程失败"))?;

        // **必须**丢掉从端：留着它，主端读端就永远等不到 EOF ——
        // 子进程退出后 read 仍会一直阻塞，表现为"输出读到一半就卡死"。
        drop(slave);

        let reader = master
            .try_clone_reader()
            .map_err(context_err("取 PTY 读端失败"))?;
        let writer = master
            .take_writer()
            .map_err(context_err("取 PTY 写端失败"))?;

        Ok(Self {
            master,
            writer: Some(writer),
            reader: Some(reader),
            child,
            status: None,
        })
    }

    /// 子进程 pid（诊断与将来的进程管理用）。
    ///
    /// 语义上它就是**会话首进程**，所以 `Transport::session_leader()` 直接返回它；
    /// 这一个留在 `PtyTransport` 上是给"只想看 pid"的调用方用的。
    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }
}

impl Transport for PtyTransport {
    fn capabilities(&self) -> Capabilities {
        Capabilities::PTY
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        if self.status.is_some() {
            return Err(TransportError::Closed);
        }
        let writer = self.writer.as_mut().ok_or(TransportError::Closed)?;
        writer.write_all(bytes)?;
        // 必须立刻 flush：终端输入**不能**留在缓冲里等下一次写入，
        // 否则用户敲的键要等到下一次输出才生效（表现为"按键延迟"）。
        writer.flush()?;
        Ok(())
    }

    fn output_stream(&mut self) -> Option<Box<dyn Read + Send>> {
        self.reader.take()
    }

    fn resize(&mut self, size: TerminalSize) -> Result<(), TransportError> {
        self.master
            .resize(size.into())
            .map_err(context_err("调整 PTY 尺寸失败"))
    }

    fn exited(&mut self) -> Result<Option<ExitStatus>, TransportError> {
        if let Some(status) = &self.status {
            return Ok(Some(status.clone()));
        }
        match self.child.try_wait()? {
            Some(raw) => {
                let status = ExitStatus::from(raw);
                self.status = Some(status.clone());
                Ok(Some(status))
            }
            None => Ok(None),
        }
    }

    /// PTY 的会话首进程就是这个 shell（`portable-pty` 在 `pre_exec` 里 `setsid()` 了）。
    /// 看门狗与 [`Self::shutdown`] 都靠它认会话 —— 两处必须是同一个 pid。
    fn session_leader(&self) -> Option<u32> {
        self.process_id()
    }

    fn shutdown(&mut self) -> Result<Option<ExitStatus>, TransportError> {
        // 幂等：`wait` 只该被调用一次，第二次直接给出上次的结局。
        if let Some(status) = &self.status {
            return Ok(Some(status.clone()));
        }

        // 顺序是有理由的，别换：
        //
        // 1. **先收整个会话，再收子进程** —— `kill_session` 靠"sid == 首进程 pid"来认会话，
        //    而 pid 只有在首进程**还活着**时才不会被复用成别的会话（见 `crate::teardown`）。
        // 2. 会话里那些**忽略 SIGHUP** 的进程（`nohup` / `trap "" HUP` / 守护化的）不会被
        //    `Child::kill()` 收走，也不会被内核的 hangup 收走 —— 只有点名 SIGKILL 才行。
        //    这正是 plan 0204 实测到的残留。
        // 3. 这一步**不能省**：不 wait 就会留下僵尸进程。
        if let Some(pid) = self.child.process_id() {
            crate::teardown::kill_session(pid);
        }

        // kill 的错误**不外抛**：它唯一现实的失败原因是"进程恰好在这一刻已经没了"，
        // 而那正是我们想要的结局 —— 真实结局由紧随其后的 wait() 给出。
        // 注意这不是"用 drop 兜底"：显式 wait 就在下一行。
        let _ = self.child.kill();

        // 收尸。
        let raw = self.child.wait()?;
        let status = ExitStatus::from(raw);
        self.status = Some(status.clone());
        Ok(Some(status))
    }
}

impl From<TerminalSize> for PtySize {
    fn from(size: TerminalSize) -> Self {
        // 像素尺寸留 0：内核在 PTY 上基本不用它，编造一个值只会误导。
        PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl From<portable_pty::ExitStatus> for ExitStatus {
    fn from(raw: portable_pty::ExitStatus) -> Self {
        match raw.signal() {
            Some(name) => ExitStatus::Signal(name.to_string()),
            None => ExitStatus::Code(raw.exit_code()),
        }
    }
}

/// 把上游（portable-pty 用 `anyhow`）的错误折进 [`TransportError::Io`]，并带上上下文。
///
/// 泛型 `E: Display` 是刻意的：这样**不必为了写出错误的类型名而给本 crate 增加 `anyhow` 直接依赖**
/// —— 类型由调用处推断。
///
/// 不保留 `source()` 链：这里的每一处失败都是**致命的启动期错误**，
/// 调用方要做的是把它显示出来，而不是按类型分支处理。
fn context_err<E: fmt::Display>(context: &'static str) -> impl FnOnce(E) -> TransportError {
    move |err| TransportError::Io(std::io::Error::other(format!("{context}：{err}")))
}

// ⚠️ 必须同时带 `test`：只写 `cfg(unix)` 会把测试代码**编进正式产物**（并因"没人用"而报警告）。
// 真实 PTY 的用例只在 unix 上跑；Windows 那一套等有 ConPTY 环境时再补。
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use rustix::process::{Pid, Signal};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::{Duration, Instant};

    /// 起一个跑探测命令的 `/bin/sh`：不碰用户登录 shell，结果才可复现。
    fn sh(size: TerminalSize) -> PtyTransport {
        PtyTransport::spawn(&ShellLaunch::new("/bin/sh"), size).expect("spawn /bin/sh 失败")
    }

    /// 在**截止时间**内把输出读到出现 `needle` 为止。
    ///
    /// 为什么不用固定 sleep（`AGENTS.md` §0 第 5 条）：shell 启动耗时随机器变化，
    /// 猜一个 sleep 要么慢要么 flaky。这里由读端驱动，一到就返回。
    fn read_until(mut output: Box<dyn Read + Send>, needle: &[u8], timeout: Duration) -> Vec<u8> {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            loop {
                match output.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let deadline = Instant::now() + timeout;
        let mut seen: Vec<u8> = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => seen.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if seen.windows(needle.len()).any(|w| w == needle) {
                break;
            }
        }
        seen
    }

    #[test]
    fn real_shell_executes_what_we_typed() {
        let mut transport = sh(TerminalSize::new(80, 24));
        assert_eq!(transport.capabilities(), Capabilities::PTY);
        let output = transport.output_stream().expect("第一次取输出流必须成功");

        // 探针刻意写成 `printf 'AKASHA%s\n' _PROBE`：**终端会把我们敲进去的字原样回显**，
        // 所以直接找 "AKASHA_PROBE" 证明不了任何事 —— 回显里根本没有它
        // （回显的是 AKASHA%s 与 _PROBE 两截）。只有 shell 真的执行了才会拼出它。
        transport
            .write(b"printf 'AKASHA%s\\n' _PROBE\n")
            .expect("write 失败");

        let seen = read_until(output, b"AKASHA_PROBE", Duration::from_secs(15));
        assert!(
            seen.windows(b"AKASHA_PROBE".len())
                .any(|w| w == b"AKASHA_PROBE"),
            "shell 应当执行探针命令，实际收到：{:?}",
            String::from_utf8_lossy(&seen)
        );

        let status = transport.shutdown().expect("shutdown 失败");
        assert!(
            status.is_some(),
            "PTY 声明了 exit_status 能力，shutdown 就必须给出结局"
        );
    }

    #[test]
    fn output_stream_can_only_be_taken_once() {
        let mut transport = sh(TerminalSize::DEFAULT);
        let first = transport.output_stream().expect("第一次应当成功");

        assert!(
            transport.output_stream().is_none(),
            "第二次必须是 None —— 同一载体的两个读端会互相偷字节"
        );

        drop(first);
        transport.shutdown().expect("shutdown 失败");
    }

    #[test]
    fn resize_works_and_shutdown_is_idempotent() {
        let mut transport = sh(TerminalSize::new(80, 24));

        assert_eq!(
            transport.exited().expect("exited 失败"),
            None,
            "刚起的 shell 不该已经退出"
        );
        transport
            .resize(TerminalSize::new(120, 40))
            .expect("PTY 支持 resize，不该返回 Unsupported");

        let first = transport.shutdown().expect("shutdown 失败");
        let second = transport
            .shutdown()
            .expect("重复 shutdown 必须成功（幂等）");
        assert_eq!(first, second, "重复 shutdown 必须给出同一个结局");
        assert!(
            transport.exited().expect("exited 失败").is_some(),
            "shutdown 已 wait 收尸，exited() 必须立刻给出结局"
        );
        assert!(
            matches!(transport.write(b"x"), Err(TransportError::Closed)),
            "关闭之后再写必须是 Closed，不能静默丢弃"
        );
    }

    /// 进程是否**还在**：`kill(pid, 0)` 存在即 `Ok`、不存在即 `ESRCH`。
    ///
    /// ⚠️ 不读 `/proc/<pid>`：那个目录只在 Linux 上存在，用它做判据会让本用例在 macOS 上
    /// 因为"路径不存在"而失败 —— 失败的并不是被验的行为（与 `teardown` 的诱饵断言同一条口径）。
    fn alive(pid: u32) -> bool {
        Pid::from_raw(pid as i32).is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
    }

    /// 在**截止时间**内等进程变成"在"或"不在"。
    fn appears(pid: u32, exists: bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if alive(pid) == exists {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// `AKPROBE=<pid>` 里的 pid（屏幕上第一处**带数字**的那个）。
    fn parse_probe_pid(seen: &[u8]) -> Option<u32> {
        let text = String::from_utf8_lossy(seen);
        let (_, rest) = text.rsplit_once("AKPROBE=")?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }

    /// **忽略 SIGHUP 的进程也必须被收掉** —— 这是 plan 0204 的核心判据。
    ///
    /// 为什么这条用例能分辨"只 kill shell"与"收掉整个会话"：探针明确忽略 SIGHUP，
    /// 于是内核在 master 关闭时发的那轮 SIGHUP 对它无效，`Child::kill()` 也够不着它
    /// （它不是 shell 本身）。只有点名 SIGKILL 才收得走。
    ///
    /// `set -m` 打开作业控制，让这个作业拿到**自己的进程组** —— 这样被验的就是
    /// "认 session 去收"而不是顺手的 `killpg`。
    #[test]
    fn shutdown_collects_processes_that_ignore_sighup() {
        let mut transport = sh(TerminalSize::DEFAULT);
        let output = transport.output_stream().expect("第一次取输出流必须成功");

        // 探针写成 `printf 'AKPROBE%s\n' =$!`：**回显**里只有 `AKPROBE%s`，
        // 所以下面按 `AKPROBE=` 找，命中的一定是 shell 求值后的输出（同 `real_shell_…` 用例）。
        // ⚠️ `$!` **不得**加引号：macOS 的 `/bin/sh` 是 bash 3.2，行内出现 `!` 会做历史展开，
        // `"=$!"` 直接报 `event not found`，探针根本不会起来。bash 只在 `!` 后面不是
        // 空白 / 换行 / `=` / `(` 时才展开 —— 让 `!` 紧跟行尾即可；不带引号的 `$!` 也不会分词
        // （结果只有数字）。
        transport
            .write(b"set -m; (trap \"\" HUP; exec sleep 300) & printf 'AKPROBE%s\\n' =$!\n")
            .expect("write 失败");

        let seen = read_until(output, b"AKPROBE=", Duration::from_secs(15));
        let pid = parse_probe_pid(&seen)
            .unwrap_or_else(|| panic!("没读到探针 pid：{:?}", String::from_utf8_lossy(&seen)));
        assert!(
            appears(pid, true),
            "探针 {pid} 应当还活着（否则这条用例什么都没验）"
        );

        transport.shutdown().expect("shutdown 失败");

        // ⚠️ "探针必须被收掉"这条**只在 Linux 上成立**：非 Linux 的 unix 没有可移植的会话枚举
        // （要 `proc_listpids` + `getsid`），`kill_session` 因此退化成 `killpg`，而本探针被
        // `set -m` 放进了**自己的进程组** —— 恰好是 `killpg` 够不着的那一类。缺口与理由见
        // `teardown` 模块的平台差异表（plan 0204）。
        #[cfg(target_os = "linux")]
        assert!(
            appears(pid, false),
            "忽略 SIGHUP 的子进程 {pid} 必须被收掉 —— 只 kill 那个 shell 是收不走的"
        );

        // 非 Linux 上"探针活下来"是已知缺口，**不是**可以用例留下的残留：这里点名 SIGKILL
        // 收掉它，并把"收得掉"当成断言 —— 它同时证明上面那次存活是真的（探针确实还在）。
        #[cfg(not(target_os = "linux"))]
        {
            if let Some(probe) = Pid::from_raw(pid as i32) {
                let _ = rustix::process::kill_process(probe, Signal::KILL);
            }
            assert!(appears(pid, false), "清理探针 {pid} 失败");
        }
    }
}
