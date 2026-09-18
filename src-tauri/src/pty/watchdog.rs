//! 看门狗：**app 被 SIGKILL 之后，还有进程替会话收尾**（plan 0205）。
//!
//! # 为什么必须另起一个进程
//!
//! `tauri dev` 的重编译重启走的是 `child.kill()` → **SIGKILL**。SIGKILL 不可捕获，
//! 进程里**没有任何一行代码会执行** —— 所以 plan 0204 那条"退出前显式回收"的路
//! 在这条路径上从原理上就不通（实测：新一代 app 起来了，上一轮的探针还活着）。
//!
//! 内核在这种时刻只留给我们一件事：**持有管道写端的所有进程都消失时，读端收到 EOF**。
//! 于是机制的全部就是：另起一个进程读一条管道。app 怎么死都不重要 —— 只要它死了，
//! 写端就关了，看门狗醒来收尾。
//!
//! 这也是它比"用 `PR_SET_PDEATHSIG` 让子进程随父而死"可靠的地方：那个"父"是
//! **创建子进程的那个线程**，而会话是从命令/线程池里起的 —— 一个 worker 线程退下去
//! 就会误杀用户的会话。而且 `PDEATHSIG` 只作用于**直接子进程**：它带不走 shell 里
//! 那些忽略 SIGHUP 的孙进程，而那正是本模块要收的人。完整取舍见 ADR-0005。
//!
//! # 协议
//!
//! 单向、逐行，只有两种行：
//!
//! ```text
//! +<会话首进程 pid>   登记一个会话（app 起会话时写）
//! -<会话首进程 pid>   撤销登记（这个会话已经收干净了，别再收第二次）
//! ```
//!
//! 认不出的行**忽略**（协议要留加字段的余地）；**读错误当成 EOF**（宁可多收一次，
//! 也不要漏 —— 漏掉的那个进程用户再也关不掉）。EOF 之前已写入的字节**不会丢**：
//! 内核先交付缓冲数据、再报 EOF —— 所以"刚登记就被 SIGKILL"那一瞬也是安全的。
//!
//! # 看门狗这一侧的生命周期
//!
//! 它**只**因为 EOF 退出。[`detach`] 让它离开 app 的会话与终端，于是 `^C`、关终端
//! 这些"打给前台进程组"的信号都带不走它 —— 而它的职责恰恰是"app 死了它还在"。
//! 它也不往任何地方输出：醒来的时候 app 已经不在了，没有读者。

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use crate::pty::teardown::kill_session;

/// 看门狗模式的 argv 标志。
///
/// 看门狗**就是本可执行文件**再跑一次（见 [`SessionWatchdog::spawn`]），
/// 所以这个标志只在 `main` 的第一行被检查。
pub const FLAG: &str = "--akasha-session-watchdog";

/// 这次启动是不是"以看门狗身份跑"。
///
/// 看**全部**参数而不是 `argv[1]`：将来给 app 加别的开关时，顺序不该改变语义。
pub fn is_invocation(args: impl IntoIterator<Item = OsString>) -> bool {
    args.into_iter().any(|arg| arg == FLAG)
}

/// 协议里的一条指令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// 登记：`+<首进程 pid>`。
    Watch(u32),
    /// 撤销登记：`-<首进程 pid>`。
    Forget(u32),
}

/// 解析一行。认不出就是 `None` —— **不报错**。
///
/// 单向协议里没有回话通道（要收尾的时候 app 已经不在了），所以"这行看不懂"唯一
/// 合理的处置就是忽略它，而不是让看门狗死掉 —— 那会让**全部**会话失去兜底。
///
/// 数字**逐位**校验，不交给 `parse::<u32>` 自己去宽容：它会接受 `+1` 这种前导正号，
/// 于是 `++1` 也会被当成"登记 pid 1" —— 而 pid 1 是 init，看门狗会去 SIGKILL 它。
pub fn parse(line: &str) -> Option<Verb> {
    let mut chars = line.trim().chars();
    let marker = chars.next()?;
    let rest = chars.as_str();
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let leader: u32 = rest.parse().ok()?;
    match marker {
        '+' => Some(Verb::Watch(leader)),
        '-' => Some(Verb::Forget(leader)),
        _ => None,
    }
}

/// 一行指令的线上表示（[`parse`] 的逆）。
fn encode(verb: Verb) -> String {
    match verb {
        Verb::Watch(leader) => format!("+{leader}\n"),
        Verb::Forget(leader) => format!("-{leader}\n"),
    }
}

/// 一次看门狗收尾的结果。
///
/// 正常路径上**没有读者**（醒来时 app 已经不在了，它也不输出东西），所以它主要是
/// 测试与将来的自检用的返回值。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WatchdogReport {
    /// EOF 时还登记着的会话数。
    pub watched: usize,
    /// 实际发出 SIGKILL 的进程数（跨全部登记过的会话求和）。
    pub killed: usize,
}

/// 与 app 的会话 / 终端脱钩。
///
/// 失败**不致命**（`setsid` 在"自己已经是进程组长"时会失败，而这里没有别的退路）：
/// 那种情况下看门狗照样工作，只是多了一条"被 `^C` 顺手带走"的路径。
///
/// ⚠️ **平台对应的做法不一样，不是同一件事的两种写法**：
///
/// | 平台 | 脱钩靠什么 |
/// |---|---|
/// | unix | 本函数：`setsid` —— 另起一个会话，从此收不到打给前台进程组的那几个信号 |
/// | Windows | **不用做事**：控制台归属在**创建那一刻**就定了，所以由 `SessionWatchdog::spawn` 带上 `CREATE_NO_WINDOW`（没有控制台，`CTRL_C_EVENT` 就送不到它手上） |
#[cfg(unix)]
pub fn detach() -> io::Result<()> {
    rustix::process::setsid().map_err(io::Error::from)?;
    Ok(())
}

/// Windows：脱钩在创建那一侧完成（见上表），这里没有对应动作。
///
/// 保留这个函数而不是在调用点写 `cfg`：调用方（app 的 `watchdog::run_if_watchdog`）
/// 因此不必知道平台差异，只表达"看门狗要脱钩"这一件事。
#[cfg(not(unix))]
pub fn detach() -> io::Result<()> {
    Ok(())
}

/// 看门狗主循环：读协议，**到 EOF 就把还登记着的会话全部收掉**。
///
/// 这里是"最后一道"回收：走到这一步说明 app 已经没机会跑任何代码了，
/// 所以除 EOF 之外没有任何事件能触发收尾 —— 也不该有（多一个触发源就多一种
/// "为什么它这时候动了"的解释成本）。
pub fn run<R: BufRead>(mut input: R) -> WatchdogReport {
    let mut leaders: BTreeSet<u32> = BTreeSet::new();
    let mut line = String::new();

    loop {
        line.clear();
        match input.read_line(&mut line) {
            // EOF：写端的所有持有者都没了 = app 不在了。
            Ok(0) => break,
            Ok(_) => match parse(&line) {
                Some(Verb::Watch(leader)) => {
                    leaders.insert(leader);
                }
                Some(Verb::Forget(leader)) => {
                    leaders.remove(&leader);
                }
                // 看不懂的行：忽略。协议要有加字段的余地。
                None => {}
            },
            // 读错误（不是 EOF，例如非法 UTF-8）：**照收**。宁可多收一次，
            // 也不要让某个进程永远留在用户机器上。
            Err(_) => break,
        }
    }

    let mut report = WatchdogReport {
        watched: leaders.len(),
        killed: 0,
    };
    for leader in leaders {
        report.killed += kill_session(leader);
    }
    report
}

/// app 这一侧的看门狗句柄。
pub struct SessionWatchdog {
    /// 控制端（管道写端）。用 `Mutex` 只是为了在 `&self` 上拿到 `Write`：
    /// 每次写入的都是一整行（十几个字节，远小于 `PIPE_BUF`），不会交错。
    control: Mutex<Box<dyn Write + Send>>,
    /// 看门狗进程的 pid（诊断用；测试里没有进程）。
    pid: Option<u32>,
}

impl SessionWatchdog {
    /// 起一个看门狗：**就是本可执行文件**再跑一次，带 [`FLAG`]。
    ///
    /// 为什么不单独做一个 bin：那要跟构建与打包打交道（`cargo run` 只跑默认 bin，
    /// 装进产物还得配 sidecar），而 `current_exe()` 在 dev / release / 打包后
    /// **给的都是同一个答案**。看门狗模式在起 GUI 之前就被认领（`main` 的第一行），
    /// 所以这一次 exec 不会碰 Tauri。
    ///
    /// stdout / stderr 都接到空设备：看门狗不是 app 输出的一部分，也没有读者；
    /// 更不能让它**攥着一条管道**不放（那会让调用方以为"进程还没退出"）。
    pub fn spawn(exe: &Path) -> io::Result<Self> {
        let mut command = Command::new(exe);
        command
            .arg(FLAG)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Windows 上的"脱钩"（见 `detach` 的平台表）：不带控制台创建，
        // 于是打给控制台的 `CTRL_C_EVENT` 送不到看门狗手上。
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command.spawn()?;

        let pid = child.id();
        let control = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("看门狗的控制管道没拿到（stdin 不是 piped）"))?;

        // 收尸线程：看门狗只会比 app 活得久，但"它意外先死"不该在 app 里变成僵尸。
        // 线程都起不来的极端情况下不报错 —— 那时的僵尸会由 init 在 app 退出时收掉，
        // 而真正值得报的问题（看门狗根本没起来）已经由上面的 `spawn()?` 报过了。
        let _ = std::thread::Builder::new()
            .name("akasha-session-watchdog-reaper".into())
            .spawn(move || {
                let _ = child.wait();
            });

        Ok(Self {
            control: Mutex::new(Box::new(control)),
            pid: Some(pid),
        })
    }

    /// 用给定的 writer 造一个句柄（**测试缝**：不真起进程，只记下协议写了什么）。
    ///
    /// `akasha` 的用例靠它验证"登记 / 撤销的确切时机"，而不必去解析一个真进程。
    pub fn with_writer(control: Box<dyn Write + Send>) -> Self {
        Self {
            control: Mutex::new(control),
            pid: None,
        }
    }

    /// 登记一个会话：app 万一再也不跑代码，就由看门狗收掉它。
    pub fn watch(&self, leader: u32) -> io::Result<()> {
        self.send(Verb::Watch(leader))
    }

    /// 撤销登记：这个会话已经显式收干净了，别在 EOF 时再收一次。
    ///
    /// ⚠️ 必须排在**收尾之后**：反过来的话，app 在"撤销了、但还没收"之间被 SIGKILL，
    /// 那个会话就没人管了。而多收一次只是对着已经死掉的 pid 再发一遍 SIGKILL ——
    /// 这是这个方向上唯一可以接受的代价（`kill_session` 靠 sid 匹配，
    /// 首进程已死时那个 sid 还在不在由内核说了算，见 `teardown` 的时机说明）。
    pub fn forget(&self, leader: u32) -> io::Result<()> {
        self.send(Verb::Forget(leader))
    }

    /// 看门狗进程的 pid（诊断用）。测试缝造出来的没有 pid。
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn send(&self, verb: Verb) -> io::Result<()> {
        let line = encode(verb);
        let mut control = self
            .control
            .lock()
            .map_err(|err| io::Error::other(err.to_string()))?;
        control.write_all(line.as_bytes())?;
        control.flush()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::pty::transport::{TerminalSize, Transport};
    use crate::pty::{Batch, BatchPolicy, PtyTransport, ShellLaunch, spawn_batcher};
    use rustix::process::Pid;
    use std::io::Cursor;
    use std::sync::mpsc::Receiver;
    use std::time::{Duration, Instant};

    /// 起一个跑探测命令的 `/bin/sh`：不碰用户登录 shell，结果才可复现。
    fn sh() -> PtyTransport {
        PtyTransport::spawn(&ShellLaunch::new("/bin/sh"), TerminalSize::DEFAULT)
            .expect("spawn /bin/sh 失败")
    }

    /// 在**截止时间**内把批次攒到探针 pid 出现为止（不用固定 sleep 猜）。
    ///
    /// 找的是 `AKPROBE<数字>`，而不是 `AKPROBE=` 这个前缀 —— 终端会把我们**敲进去的
    /// 那行原样回显**，回显里也有 `AKPROBE`（后面跟的是 `%05d`）。判据必须只对
    /// "shell 真的执行了"成立。
    fn wait_for_probe(batches: &Receiver<Batch>, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            match batches.recv_timeout(Duration::from_millis(100)) {
                Ok(batch) => seen.extend_from_slice(&batch.bytes),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let text = String::from_utf8_lossy(&seen);
            // 取**最后**一个 `AKPROBE`：回显在前、输出在后，而回显后面跟的是 `%`。
            if let Some(pid) = text.rfind("AKPROBE").and_then(|at| {
                text[at + "AKPROBE".len()..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .ok()
            }) {
                return Some(pid);
            }
        }
        None
    }

    /// 进程是否还在：`kill(pid, 0)` 存在即 `Ok`、不存在即 `ESRCH`。
    ///
    /// ⚠️ 不读 `/proc/<pid>`：那个目录只在 Linux 上存在，用它当判据会让本模块的用例在 macOS 上
    /// 一律报"探针没起来"（`alive` 恒为假），而失败的并不是被验的行为。语义与 `/proc` 那条一致：
    /// 僵尸也有目录项，`kill(pid, 0)` 对僵尸同样成功。
    fn alive(pid: u32) -> bool {
        Pid::from_raw(pid as i32).is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
    }

    /// 在截止时间内等 `pid` 消失。
    ///
    /// ⚠️ 不能发完信号立刻断言：**SIGKILL 的投递是异步的**，内核只是打上记号，
    /// 真正消失要等被调度（问题 #45）。
    fn waits_until_gone(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// 在一个会话里放一个**明确忽略 SIGHUP** 的探针，返回 (载体, 会话首进程, 探针 pid)。
    ///
    /// 探针是最坏情况：内核在 PTY 挂断时发的那轮 SIGHUP 对它无效，`Child::kill()`
    /// 也够不着它 —— 只有点名 SIGKILL 才收得走（plan 0204 实测）。
    fn session_with_probe() -> (PtyTransport, u32, u32) {
        let mut transport = sh();
        let leader = transport
            .session_leader()
            .expect("PTY 子进程应当有会话首进程 pid");
        let output = transport.output_stream().expect("取输出流失败");
        let batches = spawn_batcher(output, BatchPolicy::DEFAULT);

        // `sh -c '…' &` 是三种 shell 下语义相同的写法（`(…) &` 在 fish 里是命令替换）。
        // `printf "AKPROBE%05d" $$` 后面的 `exec` 让探针**保持同一个 pid**。
        transport
            .write(b"sh -c 'trap \"\" HUP; printf \"AKPROBE%05d\\n\" $$; exec sleep 600' &\n")
            .expect("write 失败");

        let probe = wait_for_probe(&batches, Duration::from_secs(15))
            .unwrap_or_else(|| panic!("没读到探针 pid"));

        (transport, leader, probe)
    }

    #[test]
    fn protocol_round_trips_and_ignores_nonsense() {
        for verb in [Verb::Watch(4242), Verb::Forget(7)] {
            assert_eq!(
                parse(encode(verb).as_str()),
                Some(verb),
                "编码与解析必须互逆"
            );
        }
        // 认不出的行一律 `None`（忽略，不是报错）—— 协议要留加字段的余地。
        // 负例是必须的：只验"能认出来的那些"，就分不清"规则在工作"与"它什么都没匹配"。
        for line in [
            "",
            "\n",
            " ",
            "hello",
            "?",
            "+",
            "+abc",
            "++1",
            "+1x",
            "+ 1",
            "1",
            "=1",
            "+99999999999999999",
        ] {
            assert_eq!(parse(line), None, "行 {line:?} 应当被忽略");
        }
        assert_eq!(parse("+1"), Some(Verb::Watch(1)));
        // 首尾空白与 `\r\n` 不该改变语义（将来跨平台写协议时用得上）。
        assert_eq!(parse("+42\r\n"), Some(Verb::Watch(42)));
        assert_eq!(parse("  -42  "), Some(Verb::Forget(42)));
    }

    #[test]
    fn eof_collects_the_registered_session() {
        let (mut transport, leader, probe) = session_with_probe();
        assert!(alive(probe), "探针起出来了");

        // 看门狗看到的一切：一行登记，然后 EOF（= app 被 SIGKILL）。
        let report = run(Cursor::new(format!("+{leader}\n")));

        assert_eq!(report.watched, 1);
        assert!(report.killed > 0, "会话里至少该有一个进程被收掉");
        assert!(
            waits_until_gone(probe, Duration::from_secs(15)),
            "忽略 SIGHUP 的探针必须随会话被收掉（pid {probe}）"
        );
        transport.shutdown().expect("shutdown 失败");
    }

    #[test]
    fn an_unregistered_session_is_a_bystander_not_a_casualty() {
        // 两个会话：只登记一个。诱饵是必须的 —— 没有它，"把见到的全杀了"与
        // "只杀登记过的"在测试结果上完全一样。
        let (mut victim, leader, victim_probe) = session_with_probe();
        let (mut bystander, _bystander_leader, bystander_probe) = session_with_probe();

        run(Cursor::new(format!("+{leader}\n")));

        assert!(
            waits_until_gone(victim_probe, Duration::from_secs(15)),
            "登记过的会话该被收掉"
        );
        assert!(
            alive(bystander_probe),
            "没登记的会话一个进程都不该被碰（诱饵 pid {bystander_probe}）"
        );

        victim.shutdown().expect("shutdown 失败");
        bystander.shutdown().expect("shutdown 失败");
    }

    #[test]
    fn a_forgotten_session_survives_the_last_breath() {
        // `-` 是"我已经收干净了"的意思。它错了的后果很具体：app 一死就对着
        // 一个**已经被复用**的 pid 发 SIGKILL —— 杀掉的是别人的进程。
        let (mut transport, leader, probe) = session_with_probe();

        let report = run(Cursor::new(format!("+{leader}\n-{leader}\n")));

        assert_eq!(report.watched, 0, "撤销之后就不该再有登记");
        assert_eq!(report.killed, 0, "撤销之后不该发任何信号");
        assert!(alive(probe), "撤销过的会话必须原样活着");

        transport.shutdown().expect("shutdown 失败");
    }

    #[test]
    fn a_line_written_by_a_dying_writer_is_still_read() {
        // 真管道 + 真"写完就死"的写端：内核**先交付缓冲数据、再报 EOF**。
        // 这条性质撑住了"刚登记就被 SIGKILL"那一瞬 —— 顺序反过来的话，
        // 那一瞬起出来的会话就没人管了。
        let (mut transport, leader, probe) = session_with_probe();
        let mut writer = Command::new("sh")
            .arg("-c")
            .arg(format!("printf '+{leader}\\n'"))
            .stdout(Stdio::piped())
            .spawn()
            .expect("起写端失败");
        let pipe = writer.stdout.take().expect("拿到写端的管道");
        let _ = writer.wait();

        let report = run(io::BufReader::new(pipe));

        assert_eq!(report.watched, 1, "写完就死的写端，它写的行也必须被读到");
        assert!(waits_until_gone(probe, Duration::from_secs(15)));
        transport.shutdown().expect("shutdown 失败");
    }

    #[test]
    fn watchdog_reports_what_it_did() {
        // 没有登记就没有动作：app 正常退出时看门狗要**什么都不做**地走掉。
        let report = run(Cursor::new(""));
        assert_eq!(report, WatchdogReport::default());
    }

    /// `SessionWatchdog` 这一侧的协议：`watch` / `forget` 写出来的行必须被 [`parse`]
    /// 认得（跨进程的约定，只能靠一条共享的编解码来守）。
    #[test]
    fn the_writer_speaks_the_protocol_the_reader_parses() {
        #[derive(Clone)]
        struct Recorder(std::sync::Arc<Mutex<Vec<u8>>>);
        impl Write for Recorder {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().expect("锁中毒").extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let log = std::sync::Arc::new(Mutex::new(Vec::new()));
        let watchdog = SessionWatchdog::with_writer(Box::new(Recorder(log.clone())));

        watchdog.watch(4242).expect("watch 失败");
        watchdog.forget(4242).expect("forget 失败");

        let written = String::from_utf8(log.lock().expect("锁中毒").clone()).expect("必须是 ASCII");
        assert_eq!(written, "+4242\n-4242\n");
        assert_eq!(
            written.lines().filter_map(parse).collect::<Vec<_>>(),
            vec![Verb::Watch(4242), Verb::Forget(4242)],
            "写出去的每一行都要能被看门狗读懂"
        );
        assert_eq!(watchdog.pid(), None, "测试缝没有进程");
    }

    #[test]
    fn the_flag_is_recognised_wherever_it_appears() {
        let args = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(is_invocation(args(&["akasha", FLAG])));
        assert!(
            is_invocation(args(&["akasha", "--foo", FLAG])),
            "别的开关不该改变它的语义"
        );
        assert!(!is_invocation(args(&["akasha"])));
        assert!(
            !is_invocation(args(&["akasha", "--akasha-session-watchdog=1"])),
            "只认整份参数相等，不做前缀匹配"
        );
    }
}
