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
//! 做法是**枚举进程表**、取出每个进程的 **session id**，把 `sid == 会话首进程 pid` 的
//! 进程逐个 SIGKILL：Linux 读 `/proc/<pid>/stat`（每个进程一次读，不 spawn 任何东西），
//! 其他 unix 先由 `/bin/ps` 列出 pid、再逐个向内核问 `getsid`（见 `kill_session_by_listing`）。
//!
//! 为什么这样扫是安全的：sid 在会话存续期内**就是**首进程的 pid，只要首进程还活着，
//! 那个 pid 就不可能被复用成别的会话 —— 所以调用点必须排在"kill 子进程"**之前**
//! （[`crate::PtyTransport::shutdown`] 正是这个顺序）。
//!
//! # 平台差异（照实说，不假装一样）
//!
//! | 平台 | 本模块的行为 |
//! |---|---|
//! | Linux | 读 `/proc` 扫 session，逐个 SIGKILL |
//! | 其他 unix | `ps` 列 pid + `getsid` 判会话，逐个 SIGKILL；`ps` 不可用时退到 `killpg` |
//! | Windows | 什么都不做（没有 POSIX 会话；等价物要上 Job Object） |
//!
//! Windows 那一行的缺口是**已知且被记录**的（plan 0204 的实施记录、`docs/STATUS.md` 的已知问题），
//! 不是"顺手忽略了"。⚠️ **可编译不等于有实现**：本模块在 Windows 上编译得过（plan 0108
//! 给这几处补上了 `cfg` 守卫），但它在 Windows 上**仍然什么都不做** —— 那条缺口留在
//! plan 0108 的「留下的缺口」里，实现它要有 Windows 主机可验收。

// ⚠️ 只在 unix 上：上游把 `rustix::process` 限定在 `#[cfg(not(windows))]`，
// Windows 上没有这个模块（问题 #149，plan 0108）。
#[cfg(unix)]
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
        kill_session_by_listing(leader)
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
#[cfg(unix)]
fn sigkill(pid: i32) -> bool {
    let Some(pid) = Pid::from_raw(pid) else {
        return false;
    };
    rustix::process::kill_process(pid, Signal::KILL).is_ok()
}

/// 非 Linux 的 unix：枚举 pid，把 `getsid(pid) == leader` 的逐个 SIGKILL。
///
/// macOS 没有 `/proc`，而 POSIX 也没有"列出某个会话的成员"的调用，所以这里要把两件事
/// 拼起来：**枚举 pid**（`ps`）与**问内核这个 pid 在哪个会话里**（`getsid`）。
/// 只能用 `ps` 枚举 pid：其他 unix 上唯一的替代是 `libproc` 的 `proc_listpids`，
/// 那要写 `unsafe`，而本仓库只允许 `akasha-store` 出现它（`AGENTS.md` §3.4）。
///
/// ⚠️ **不读 `ps` 的 `sess` / `tsess` 列**：本机实测（macOS 26.6.2）那两个关键字打印的是
/// 会话**指针**，对任何进程都输出 0（连会话首进程也是 0）—— 拿它当会话 id 会一个进程都
/// 匹配不到（那正是"ps 说没有、会话却还在"的来源）。会话 id 只由 `getsid` 给。
/// `ps` 在这里只当**pid 列表**用，那是它最不会变的一列。
///
/// 失败（`ps` 不在、被沙箱拒绝、输出不可解析）不是错误：退到 `killpg`，能收多少收多少。
#[cfg(all(unix, not(target_os = "linux")))]
fn kill_session_by_listing(leader: u32) -> usize {
    let Some(pids) = ps_pids() else {
        return kill_group(leader);
    };
    let mut killed = 0;
    for pid in pids {
        if session_of(pid) == Some(leader) && sigkill(pid) {
            killed += 1;
        }
    }
    killed
}

/// 枚举工具的**绝对路径**。
///
/// 不用相对名字：`PATH` 在打包 / 沙箱 / 服务化启动下不由我们掌控，而它在这些 unix 上
/// 都是系统自带的 `/bin/ps`。
///
/// ⚠️ macOS 的 `ps` 是 **setuid root** 的：进程表里的其他进程只有这个身份读得到，
/// 而去掉 setuid 位的那一份**一行输出都不给**（本机实测）。因此任何拒绝 setuid 执行的
/// 沙箱里枚举都会失败 —— 那正是下面要保留 `killpg` 退路的原因。
#[cfg(all(unix, not(target_os = "linux")))]
const PS: &str = "/bin/ps";

/// 全部 pid（`ps -Ao pid=`）。跑不起来就是 `None`。
#[cfg(all(unix, not(target_os = "linux")))]
fn ps_pids() -> Option<Vec<i32>> {
    let output = std::process::Command::new(PS)
        .args(["-Ao", "pid="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_pid_list(&String::from_utf8_lossy(&output.stdout)))
}

/// 从一列 pid 的输出里取出 pid。认不出的行**跳过**：带 `=` 时 `ps` 不出表头，
/// 不带时第一行是 `PID` —— 让解析对两种形态都不敏感。
#[cfg(any(all(unix, not(target_os = "linux")), test))]
fn parse_pid_list(text: &str) -> Vec<i32> {
    text.lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

/// 某个进程的会话 id（= 该会话首进程的 pid）。读不到（进程刚没、权限不足）就是 `None`。
#[cfg(all(unix, not(target_os = "linux")))]
fn session_of(pid: i32) -> Option<u32> {
    let pid = Pid::from_raw(pid)?;
    rustix::process::getsid(Some(pid))
        .ok()
        .map(|sid| sid.as_raw_pid() as u32)
}

/// 给 `leader` 所在的**进程组**发 SIGKILL（枚举不可用时的退化路径）。
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

    /// 起一个真 PTY（`/bin/sh`）。用它而不是 `std::process::Command`：会话首进程的 pid
    /// 必须是**确定的**，而 `Command::spawn` 拿到的 pid 与 `setsid` 之后的会话首进程
    /// 未必是同一个（`setsid` 在"自己已经是组长"时会 fork）。
    fn session() -> PtyTransport {
        PtyTransport::spawn(&ShellLaunch::new("/bin/sh"), TerminalSize::DEFAULT)
            .expect("起 /bin/sh 失败")
    }

    /// 进程是否**还在**：`kill(pid, 0)` 存在即 `Ok`、不存在即 `ESRCH`。
    ///
    /// ⚠️ 不读 `/proc/<pid>`：那个目录只在 Linux 上存在，拿它当诱饵的判据会让本用例因为
    /// "路径不存在"而失败 —— 失败的并不是被验的行为。
    fn still_alive(pid: u32) -> bool {
        Pid::from_raw(pid as i32).is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
    }

    /// `ps -Ao pid=` 那一列：能解析的留下，表头与畸形行跳过。
    ///
    /// 非 Linux 的 unix 上这里是**唯一**的解析点，所以正例与反例一起钉住：不带 `=` 时
    /// 出现的表头行、空行、带别的内容的行都必须被跳过。
    #[test]
    fn the_pid_list_keeps_what_it_can_parse() {
        let table = "  PID\n  100\n  101\n\n  oops\n  300\n  1 x\n";
        assert_eq!(parse_pid_list(table), vec![100, 101, 300]);
        assert!(parse_pid_list("").is_empty(), "空输出不是 pid 列表");
    }

    /// 这套枚举在真机器上端到端跑一次：pid 列表看得到会话首进程，`getsid` 说得出它的会话。
    ///
    /// ⚠️ 失败时把 `ps` 的条数带进消息：这条用例是"枚举坏了"的唯一探针，只说"没找到"
    /// 会把排查引向信号投递，而坏掉的通常是枚举。
    #[cfg(all(unix, not(target_os = "linux")))]
    #[test]
    fn the_listing_sees_a_live_session() {
        let mut victim = session();
        let leader = victim
            .session_leader()
            .expect("PTY 子进程应当有会话首进程 pid");

        let pids = ps_pids().unwrap_or_else(|| panic!("{PS} 跑不起来"));
        assert!(
            pids.contains(&(leader as i32)),
            "pid 列表（{} 条）里必须有会话首进程 {leader}",
            pids.len()
        );
        assert_eq!(
            session_of(leader as i32),
            Some(leader),
            "会话首进程的会话 id 就是它自己"
        );
        assert_ne!(
            session_of(std::process::id() as i32),
            Some(leader),
            "本进程在另一个会话里（诱饵：枚举不得把谁都算进来）"
        );

        victim.shutdown().expect("shutdown 失败");
    }

    #[test]
    fn a_whole_session_is_killed() {
        let mut victim = session();
        let victim_pid = victim.process_id().expect("PTY 子进程应当有 pid");

        let killed = kill_session(victim_pid);

        assert!(killed > 0, "本会话里至少有一个进程该被收掉");

        // ⚠️ 这里**不**断言"首进程立刻消失"：macOS 上它收到 SIGKILL 之后可能停在
        // `ps` 的 `?E`（正在退出）状态里，直到**主端被关掉**才真正结束（见
        // `PtyTransport::shutdown` 的第 3 步）。被验的是产品路径的终点，不是中间态。
        victim.shutdown().expect("shutdown 失败");
        assert!(
            !still_alive(victim_pid),
            "会话首进程 {victim_pid} 必须被收掉"
        );
    }

    #[test]
    fn another_session_is_a_bystander_not_a_casualty() {
        let mut victim = session();
        let victim_pid = victim.process_id().expect("PTY 子进程应当有 pid");
        let mut bystander = session();
        let bystander_pid = bystander.process_id().expect("PTY 子进程应当有 pid");

        kill_session(victim_pid);

        // 诱饵是必须的：没有它，"扫到的全杀了"与"只杀了该杀的"在测试结果上完全一样。
        assert_eq!(
            bystander.exited().expect("exited 失败"),
            None,
            "另一个会话的进程不得被误杀（诱饵）"
        );
        assert!(still_alive(bystander_pid), "诱饵进程还应当活着");

        // 收掉目标会话的**全过程**里都不能碰到诱饵 —— 只验"杀的那一刻"会漏掉收尾路径上的
        // 误伤（`shutdown` 比 `kill_session` 多做三件事）。
        victim.shutdown().expect("shutdown 失败");
        assert_eq!(
            bystander.exited().expect("exited 失败"),
            None,
            "收尾路径也不得碰到另一个会话（诱饵）"
        );
        assert!(still_alive(bystander_pid), "诱饵进程还应当活着");

        bystander.shutdown().expect("shutdown 失败");
    }
}
