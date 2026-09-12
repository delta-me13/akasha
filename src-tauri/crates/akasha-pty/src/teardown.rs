//! 收掉**整个终端会话**，而不只是那个 shell。
//!
//! # 为什么光 kill 子进程不够
//!
//! PTY 上的 shell 是**会话首进程**（portable-pty 在 `pre_exec` 里 `setsid()`），用户在它
//! 里面起的每一个作业都在同一个 session 里，但**各自的进程组**（交互式 shell 会打开作业控制）。
//! 于是现成的回收路径都会漏：
//!
//! | 路径 | 它盖住谁 | 它漏掉谁 |
//! |---|---|---|
//! | `killpg(shell_pid)` | 与 shell 同进程组的那些 | 作业控制下**自己一个进程组**的作业 |
//! | 内核在 master 关闭时发 SIGHUP | 会话里**全部**进程组 | **明确忽略 SIGHUP** 的：`nohup`、`trap "" HUP`、setsid 出去的守护进程 |
//! | `Child::kill()`（portable-pty） | 只有 shell 本身 | 上面两类全部 |
//!
//! 实测（plan 0204 的实施记录）：`sh -c 'trap "" HUP; exec sleep 600' &` 在
//! 「关窗口 / SIGKILL / SIGTERM」三条路径上**都活下来**，而且反复起停会**累积**。
//! 终端应用不能这样：用户关掉了窗口，终端里起的东西就该真的没了。
//!
//! # 手段
//!
//! **SIGKILL 是唯一收得走「忽略 SIGHUP」的信号**，而它必须被点名送到每个进程上 ——
//! POSIX 没有"杀掉一个 session"的调用（`killpg` 是进程组，不是会话）。
//! Linux 上的做法是扫 `/proc/<pid>/stat` 里的 **session id**，把 `sid == 会话首进程 pid`
//! 的进程逐个 SIGKILL。
//!
//! 为什么这样扫是安全的：sid 在会话存续期内**就是**首进程的 pid，只要首进程还活着，
//! 那个 pid 就不可能被复用成别的会话 —— 所以调用点必须排在"kill 子进程"**之前**
//! （[`crate::PtyTransport::shutdown`] 正是这个顺序）。
//!
//! # 平台差异（照实说，不假装一样）
//!
//! | 平台 | 本模块的行为 |
//! |---|---|
//! | Linux | 扫 session，逐个 SIGKILL |
//! | 其他 unix | 只 `killpg`（同一个进程组；作业控制下的作业收不到） |
//! | Windows | 什么都不做（没有 POSIX 会话；等价物要上 Job Object） |
//!
//! 后两行的缺口是**已知且被记录**的（plan 0204 的实施记录、`docs/STATUS.md` 的坑），
//! 不是"顺手忽略了"。

use rustix::process::{Pid, Signal};

/// 收掉 `leader` 这个会话里的全部进程，返回**成功发出信号的个数**。
///
/// `leader` 是会话首进程（PTY 子进程）的 pid。收不掉（权限、竞态、平台不支持）不报错：
/// 这是退出路径，能收多少收多少，剩下的由调用方的 `wait` 与内核的 hangup 兜底。
pub(crate) fn kill_session(leader: u32) -> usize {
    #[cfg(target_os = "linux")]
    {
        kill_session_by_scan(leader)
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // 没有可移植的"会话"枚举（macOS 要 `proc_listpids` + `getsid`），退到进程组。
        kill_group(leader)
    }

    #[cfg(not(unix))]
    {
        let _ = leader;
        0
    }
}

/// 扫 `/proc` 找 `sid == leader` 的进程，逐个 SIGKILL。
#[cfg(target_os = "linux")]
fn kill_session_by_scan(leader: u32) -> usize {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 0; // 没有 /proc（极端容器）—— 退化成"只有子进程被收"，不 panic
    };

    let mut killed = 0;
    for entry in entries.flatten() {
        // /proc 下只有 pid 目录是纯数字，其余（self、sys、…）在这里被筛掉。
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if session_of(pid) == Some(leader) && sigkill(pid) {
            killed += 1;
        }
    }
    killed
}

/// 某个进程的 session id。读不到（进程刚没、权限不足、格式意外）就是 `None`。
#[cfg(target_os = "linux")]
fn session_of(pid: i32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // 格式：`pid (comm) state ppid pgrp session …` —— comm 里**可以**有空格和括号
    // （`(sd-pam)` 这种），所以只能从**最后一个** `)` 之后开始切，不能从头按空格切。
    let rest = stat.rsplit_once(')')?.1;
    rest.split_whitespace().nth(3)?.parse().ok()
}

/// 给 `pid` 发 SIGKILL。返回是否真的发出去了。
fn sigkill(pid: i32) -> bool {
    let Some(pid) = Pid::from_raw(pid) else {
        return false;
    };
    rustix::process::kill_process(pid, Signal::KILL).is_ok()
}

/// 给 `leader` 所在的**进程组**发 SIGKILL（非 Linux 的 unix 退化路径）。
#[cfg(all(unix, not(target_os = "linux")))]
fn kill_group(leader: u32) -> usize {
    let Some(pid) = Pid::from_raw(leader as i32) else {
        return 0;
    };
    usize::from(rustix::process::kill_process_group(pid, Signal::KILL).is_ok())
}

// 真实 PTY 的用例只在 unix 上跑；Windows 没有会话这个概念，等有 Job Object 时再补。
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::transport::{TerminalSize, Transport};
    use crate::{PtyTransport, ShellLaunch};
    use std::time::{Duration, Instant};

    /// 起一个真 PTY（`/bin/sh`）。用它而不是 `std::process::Command`：会话首进程的 pid
    /// 必须是**确定的**，而 `Command::spawn` 拿到的 pid 与 `setsid` 之后的会话首进程
    /// 未必是同一个（`setsid` 在"自己已经是组长"时会 fork）。
    fn session() -> PtyTransport {
        PtyTransport::spawn(&ShellLaunch::new("/bin/sh"), TerminalSize::DEFAULT)
            .expect("起 /bin/sh 失败")
    }

    /// 在**截止时间**内等这个载体收工。
    ///
    /// ⚠️ 不能 `kill_session` 完就立刻断言：**SIGKILL 的投递是异步的** —— 内核只是给它
    /// 打上记号，真正变成僵尸要等被调度。紧接着读状态会读到 "R"，那**不是**"没收掉"。
    fn waits_until_exited(transport: &mut PtyTransport) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(transport.exited(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_whole_session_is_killed() {
        let mut victim = session();
        let victim_pid = victim.process_id().expect("PTY 子进程应当有 pid");

        let killed = kill_session(victim_pid);

        assert!(killed > 0, "本会话里至少有一个进程该被收掉");
        assert!(waits_until_exited(&mut victim), "会话首进程应当已经被收掉");
        victim.shutdown().expect("shutdown 失败");
    }

    #[test]
    fn another_session_is_a_bystander_not_a_casualty() {
        let mut victim = session();
        let victim_pid = victim.process_id().expect("PTY 子进程应当有 pid");
        let mut bystander = session();
        let bystander_pid = bystander.process_id().expect("PTY 子进程应当有 pid");

        kill_session(victim_pid);

        // 诱饵是必须的：没有它，"扫到的全杀了"与"只杀了该杀的"在测试结果上完全一样。
        assert!(waits_until_exited(&mut victim), "目标会话应当被收掉");
        assert_eq!(
            bystander.exited().expect("exited 失败"),
            None,
            "另一个会话的进程不得被误杀（诱饵）"
        );
        assert!(
            std::path::Path::new(&format!("/proc/{bystander_pid}")).exists(),
            "诱饵进程还应当活着"
        );

        victim.shutdown().expect("shutdown 失败");
        bystander.shutdown().expect("shutdown 失败");
    }
}
