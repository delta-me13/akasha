//! plan 0205 的端到端验收：**app 被 SIGKILL 之后零残留** —— 用一次真的 SIGKILL 证明。
//!
//! 为什么这条路径非要有"另一个进程"兜底：`tauri dev` 的重编译重启走的是
//! `child.kill()` = **SIGKILL**，进程里**没有任何一行代码会执行**，所以 plan 0204
//! 那条"退出前显式回收"的路在这里从原理上就不通。
//!
//! 这条用例跑的是**真的那个可执行文件**（`CARGO_BIN_EXE_akasha`）、真的管道、真的会话：
//!
//! 1. 起一个真 PTY 会话，里面放一个**明确忽略 SIGHUP** 的探针（最坏情况：内核那轮
//!    SIGHUP 对无效、`Child::kill()` 也够不着）；
//! 2. 把 app 二进制以 `--akasha-session-watchdog` 起成看门狗，再让一个"持有管道的
//!    中间人"把登记行转给它 —— 中间人就是被测 app 的替身，**用 SIGKILL 杀它**，
//!    与 `tauri dev` 重载、`kill -9` 是同一回事；
//! 3. 断言探针随会话消失，而**没登记过**的另一个会话一根毫毛都不少（诱饵）。
//!
//! 中间人为什么要先回一句 `RELAYED` 到自己的 stderr：登记行是**经过它**才到看门狗
//! 手里的，不等这个回执就杀它，杀掉的可能是"还攥着那行没转出去"的时刻 —— 那是一条
//! 会随机红的用例，而不是一条能证明什么的用例。
//!
//! 为什么不接进 `just test-e2e`：这条用例**不需要 app 在跑**（没有窗口、没有 IPC、
//! 不碰 Victauri），让它在 E2E 里跑只会白起一套 Vite + app。清单在 `src-tauri/justfile`
//! 的 `E2E_NO_APP` 里显式登记。
//!
//! 平台差异照实说：会话级回收目前只有 Linux 实现（`akasha-pty` 的 `teardown` 模块；
//! Windows 要 Job Object，macOS 要 `proc_listpids` + `getsid`），所以强断言只在 Linux 上
//! 跑，别处**显式跳过并打印原因** —— 不把弱判据说成强判据。
//!
//! 本文件是测试，`unwrap` / `expect` 在这里就是断言手段。

#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use akasha_pty::{
    Batch, BatchPolicy, PtyTransport, ShellLaunch, TerminalSize, Transport, spawn_batcher,
};

/// 看门狗模式的 argv 标志（`akasha_pty::watchdog::FLAG` 的线上表示）。
const FLAG: &str = "--akasha-session-watchdog";

/// 进程是否还在。
fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// 在截止时间内等 `pid` 从 `/proc` 里消失。
///
/// ⚠️ 不能发完信号立刻断言：**SIGKILL 的投递是异步的**，内核只是打上标记，真正消失
/// 要等被调度（问题 #45）。
///
/// ⚠️ 只对**父进程不是本用例**的那些进程成立：本用例起的会话首进程被收掉之后是
/// **僵尸**（没人 `wait` 它），`/proc/<pid>` 会一直在。判它得走
/// [`waits_until_exited`] —— "还在的过程项"不等于"还活着的进程"。
fn waits_until_gone(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// 在截止时间内等某个子进程被 `wait` 掉（它一退出就是僵尸，`/proc` 判不出来）。
fn waits_until_reaped(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// 在截止时间内等这个载体收工（`exited()` 会顺手 `wait` 掉僵尸）。
fn waits_until_exited(transport: &mut PtyTransport, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if matches!(transport.exited(), Ok(Some(_))) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// 起一个会话，并在里面放一个忽略 SIGHUP 的探针；返回 (载体, 会话首进程 pid, 探针 pid)。
fn session_with_probe(label: &str) -> (PtyTransport, u32, u32) {
    let mut transport = PtyTransport::spawn(&ShellLaunch::new("/bin/sh"), TerminalSize::DEFAULT)
        .unwrap_or_else(|err| panic!("{label}：起 /bin/sh 失败：{err}"));
    let leader = transport
        .session_leader()
        .unwrap_or_else(|| panic!("{label}：PTY 子进程应当有会话首进程 pid"));
    let output = transport
        .output_stream()
        .unwrap_or_else(|| panic!("{label}：取输出流失败"));
    let batches = spawn_batcher(output, BatchPolicy::DEFAULT);

    // `sh -c '…' &` 是三种 shell 下语义相同的写法（`(…) &` 在 fish 里是命令替换）。
    // 探针打印自己的 pid 再 `exec`，于是 pid 不变、而进程忽略 SIGHUP。
    transport
        .write(b"sh -c 'trap \"\" HUP; printf \"AKPROBE%05d\\n\" $$; exec sleep 600' &\n")
        .unwrap_or_else(|err| panic!("{label}：write 失败：{err}"));

    let probe = wait_for_probe(&batches, Duration::from_secs(20))
        .unwrap_or_else(|| panic!("{label}：没读到探针 pid"));
    (transport, leader, probe)
}

/// 在截止时间内把批次攒到探针 pid 出现为止。
///
/// 找的是 `AKPROBE<数字>`，不是 `AKPROBE` 这个前缀 —— 终端会把敲进去的那行原样回显，
/// 回显后面跟的是 `%05d`。判据必须只对"shell 真的执行了"成立。
fn wait_for_probe(batches: &std::sync::mpsc::Receiver<Batch>, timeout: Duration) -> Option<u32> {
    let deadline = Instant::now() + timeout;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        match batches.recv_timeout(Duration::from_millis(100)) {
            Ok(batch) => seen.extend_from_slice(&batch.bytes),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let text = String::from_utf8_lossy(&seen);
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

/// 起一个"持有管道的人"：它把一行登记转给看门狗，回一句 `RELAYED` 到自己的 stderr，
/// 然后 `exec cat` 一直攥着写端 —— 直到被 SIGKILL（= app 被 SIGKILL 的样子）。
fn spawn_pipe_holder(leader: u32, watchdog_stdin: Stdio) -> (Child, std::process::ChildStdin) {
    let mut holder = Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"printf "+%s\n" "$1" >&1; printf "RELAYED\n" >&2; exec cat"#)
        .arg("sh")
        .arg(leader.to_string())
        .stdin(Stdio::piped())
        .stdout(watchdog_stdin)
        .stderr(Stdio::piped())
        .spawn()
        .expect("起管道持有者失败");
    let stderr = holder.stderr.take().expect("持有者的 stderr");
    let line = BufReader::new(stderr)
        .lines()
        .next()
        .expect("读持有者的回执失败")
        .expect("持有者的回执不是文本");
    assert_eq!(line, "RELAYED", "登记行必须先真的转出去，再允许杀它");
    let stdin = holder.stdin.take().expect("持有者的 stdin");
    (holder, stdin)
}

#[test]
fn a_sigkill_of_the_app_leaves_no_child_behind() {
    if !cfg!(target_os = "linux") {
        eprintln!(
            "Skipping: 会话级回收只有 Linux 实现（Windows 要 Job Object、macOS 要 proc_listpids）"
        );
        return;
    }

    // 诱饵：一个**从没登记过**的会话。没有它，"把见到的全杀了"与"只收自己登记的"
    // 在测试结果上完全一样。
    let (mut decoy, decoy_leader, decoy_probe) = session_with_probe("诱饵");
    let (mut victim, leader, victim_probe) = session_with_probe("目标");

    // 真的那个可执行文件，以看门狗身份再跑一次。
    let mut watchdog = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .arg(FLAG)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("起看门狗（= 再跑一次 app 二进制）失败");
    let watchdog_pid = watchdog.id();
    let watchdog_stdin = watchdog.stdin.take().expect("看门狗的控制管道");

    let (mut holder, _holder_stdin) = spawn_pipe_holder(leader, watchdog_stdin.into());

    // ★ 这就是被测的那一次：SIGKILL —— 目标进程没有机会执行任何代码。
    holder.kill().expect("SIGKILL 持有者失败");
    let _ = holder.wait();

    assert!(
        waits_until_gone(victim_probe, Duration::from_secs(20)),
        "忽略 SIGHUP 的探针（pid {victim_probe}）必须随会话被看门狗收掉"
    );
    assert!(
        waits_until_exited(&mut victim, Duration::from_secs(20)),
        "会话首进程（pid {leader}）也该一起走"
    );

    // 诱饵必须原样活着：看门狗收的是**登记过的会话**，不是"所有像样的进程"。
    assert!(alive(decoy_probe), "没登记过的探针不得被误杀");
    assert!(alive(decoy_leader), "没登记过的会话首进程不得被误杀");
    assert_eq!(
        decoy.exited().expect("exited 失败"),
        None,
        "诱饵会话必须还在跑"
    );

    // 看门狗自己收工（EOF 之后它没有理由继续活着）。
    assert!(
        waits_until_reaped(&mut watchdog, Duration::from_secs(20)),
        "看门狗（pid {watchdog_pid}）该在收完尾之后自己退出"
    );

    victim.shutdown().expect("shutdown 失败");
    decoy.shutdown().expect("shutdown 失败");
}
