#!/usr/bin/env python3
"""akasha 的受限执行器：在沙箱之外执行**用户在 policy 里登记过**的项目配方。

为什么存在：macOS 上 DSH 的 workspace-write 沙箱禁写 pty 设备（/dev/ptmx 的 O_RDWR）与
~/.cargo，终端项目在其中既不能构建也不能运行。本进程需要一次提权才能启动，因此必须做到
"提权出去的是一份没有任意执行能力的东西"：

* 命令表只来自 policy 文件（默认 ~/.akasha-agent-runner/policy.json，可用环境变量
  AKASHA_AGENT_RUNNER_POLICY 覆盖）。该文件必须在工作区与 /tmp 之外 —— 这两处都在
  workspace-write 的可写集合里，放进去等于把能力表交给被约束方；--check 会拒绝这种配置。
* 本脚本内部没有任何命令表，只承认 policy 中登记过的动作名。
* 请求只能携带动作名，无法夹带参数、路径或 shell 元字符。
* 子进程一律 execve：固定 cwd、固定最小环境、argv 原样取自 policy，不经过 /bin/sh。
* 脚本自身的 sha256 必须与 policy.script_sha256 相等，否则拒绝启动 —— 改脚本必须由用户重新登记。
* 生命周期不逃逸：不 setsid、不 double-fork，随父进程（DSH 后台作业）存活；父进程消失、
  空闲超时、最大存活期三者任一触发即自毁，并回收它名下的进程组。
* 每个请求自成一份记录（req/<id>/）：并发请求互不覆盖，等待可重连，退出码不丢。
* 请求目录落在沙箱可写区，因此一律以 O_NOFOLLOW + 目录 fd 访问，不跟随符号链接。
  否则被约束方可以用一个符号链接借用执行器的权限，在工作区之外写文件。

请求协议（见 docs/agent-runner.md §4）：客户端先建 req/<id>/wait（FIFO），再放 action，
最后放 ready；执行器收尾时先原子写 result.json、再往 wait 写一行并关闭 —— 客户端阻塞在
select 上由内核唤醒，不轮询文件系统。id 缺失或目录不可达时，result.json 就是唯一的答案。

平台：POSIX（FIFO、进程组、select、dir_fd）。Windows 上只允许 Git Bash / MSYS2 的 POSIX
Python，PowerShell 与 cmd 不受支持；不满足该条件时拒绝启动并说明原因。

用法：
  --print-policy [--out PATH]      生成 policy 样板（含当前脚本哈希）供手工复制
  --check                          校验 policy 与自身哈希，打印将要授予的能力
  --serve                          常驻（DSH 后台作业 + 一次提权）
  --run ACTION [--timeout N]       提交并阻塞到结束；退出码 = 动作退出码
  --submit ACTION                  只提交，打印 request=<id>
  --wait ID [--timeout N]          阻塞等待某请求（已结束则立即返回，可重复等待）
  --result ID                      非阻塞读某请求的结果
  --status                         授权清单、在飞请求与 dev 状态
  --stop / --stop-dev              停止执行器 / 只回收 dev
"""
import errno
import hashlib
import json
import os
import pty
import re
import select
import shutil
import signal
import stat
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SELF = os.path.abspath(__file__)
REPO = os.path.dirname(HERE)

POLICY_ENV = "AKASHA_AGENT_RUNNER_POLICY"
RUNTIME_ENV = "AKASHA_AGENT_RUNNER_HOME"
DEFAULT_POLICY = os.path.join(os.path.expanduser("~"), ".akasha-agent-runner", "policy.json")
DEFAULT_RUNTIME = "/tmp/akasha-agent-runner"

# 单个动作的墙钟上限：挡住"动作无响应 ⇒ 执行器也停不下来"这条路径。
DEFAULT_ACTION_TIMEOUT = 1800
# 同时在飞的动作数上限：超出即回 busy，不排队（排队会让等待方拿到别人的结局）。
DEFAULT_MAX_CONCURRENT = 4
# 已收尾的请求目录保留时长：等待方可能晚一步回来取结果。
DEFAULT_RESULT_TTL = 3600
# 客户端默认等待上限：低于 DSH 前台调用上限（实测 600s），避免等待方先被自己的运行环境终止。
CLIENT_WAIT_DEFAULT = 540
# 监护进程先 setsid 再登记，登记之前的 killpg 只会得到 ESRCH。
DEV_GRACE = 3.0

POLL = 0.5
LOG_MAX = 4 * 1024 * 1024
LOG_TAIL = 12
NAME_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
ID_RE = re.compile(r"^[0-9a-f]{16}$")
CONTROL = ("stop", "stop-dev")
SHELL_META = (";", "|", "&&", ">", "$(", chr(96))
FIXED_PATH = os.environ.get("PATH", "/usr/bin:/bin")
BASE_ENV = {
    "LANG": "en_US.UTF-8",
    "SHELL": "/bin/zsh",
    "TERM": "dumb",
}
# O_NOFOLLOW 只作用于最后一个路径分量；中间分量由 0700 的运行时目录保证。
DIR_FLAGS = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | os.O_NOFOLLOW


def platform_guard():
    """Windows 只允许 Git Bash / MSYS2 的 POSIX Python；PowerShell 与 cmd 不受支持。"""
    if os.name != "posix":
        return ("本机制依赖 POSIX 语义（FIFO、进程组、select、dir_fd）。Windows 上只支持 "
                "Git Bash / MSYS2，PowerShell 与 cmd 不受支持 —— 请在 Git Bash 中执行 just runner-*。")
    if sys.platform in ("msys", "cygwin") and not os.environ.get("MSYSTEM"):
        return ("检测到 MSYS/Cygwin 的 Python 但不在 Git Bash / MSYS2 会话内（MSYSTEM 未设置）；"
                "请在 Git Bash 中执行 just runner-*。")
    for func in (os.open, os.stat, os.mkdir, os.unlink):
        if func not in os.supports_dir_fd:
            return ("本 Python 不支持 dir_fd（%s）：请求目录的符号链接防护无法成立，拒绝运行。"
                    % func.__name__)
    return None


def policy_path():
    return os.environ.get(POLICY_ENV) or DEFAULT_POLICY


def runtime_dir():
    return os.environ.get(RUNTIME_ENV) or DEFAULT_RUNTIME


def req_root():
    return os.path.join(runtime_dir(), "req")


def req_path(rid):
    return os.path.join(req_root(), rid)


def at(name):
    return os.path.join(runtime_dir(), name)


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def log(line):
    try:
        with open(at("out.log"), "a", buffering=1) as handle:
            handle.write("%s %s\n" % (time.strftime("%Y-%m-%dT%H:%M:%S"), line))
    except OSError:
        pass


def write_state(**fields):
    tmp = at("state.json.tmp")
    try:
        with open(tmp, "w") as handle:
            json.dump(fields, handle, ensure_ascii=False, sort_keys=True)
            handle.write("\n")
        os.replace(tmp, at("state.json"))
    except OSError:
        pass


def inside(path, root):
    path = os.path.realpath(path)
    root = os.path.realpath(root)
    return path == root or path.startswith(root + os.sep)


def check_anchor_location(path):
    """policy 落在工作区或临时目录之下等于把能力表交给被约束方，必须拒绝。"""
    if inside(path, REPO):
        return "policy 不能放在工作区内（workspace-write 可写）: %s" % path
    for temp in (runtime_dir(), "/tmp", os.environ.get("TMPDIR") or "/tmp"):
        if inside(path, temp):
            return "policy 不能放在临时目录内（workspace-write 可写）: %s" % path
    return None


def load_policy(path):
    with open(path) as handle:
        policy = json.load(handle)
    actions = policy.get("actions")
    if not isinstance(actions, dict) or not actions:
        raise ValueError("policy.actions 必须是非空对象")
    for name, argv in actions.items():
        if not NAME_RE.match(name):
            raise ValueError("动作名非法: %r" % name)
        if not isinstance(argv, list) or not argv or not all(isinstance(x, str) for x in argv):
            raise ValueError("动作 %s 必须是字符串数组" % name)
        if not argv[0].startswith("/"):
            raise ValueError("动作 %s 的 argv[0] 必须是绝对路径: %r" % (name, argv[0]))
        for flag in SHELL_META:
            if any(flag in item for item in argv):
                raise ValueError("动作 %s 的 argv 含 shell 元字符 %r" % (name, flag))
    workdir = policy.get("workdir")
    if not isinstance(workdir, str) or not os.path.isdir(workdir):
        raise ValueError("policy.workdir 不是已存在的目录: %r" % workdir)
    for key in ("idle_timeout_sec", "max_lifetime_sec", "action_timeout_sec", "result_ttl_sec"):
        value = policy.get(key, 0)
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            raise ValueError("policy.%s 必须是 >= 0 的整数" % key)
    concurrent = policy.get("max_concurrent_actions", DEFAULT_MAX_CONCURRENT)
    if not isinstance(concurrent, int) or isinstance(concurrent, bool) or concurrent < 1:
        raise ValueError("policy.max_concurrent_actions 必须是 >= 1 的整数")
    return policy


def verify():
    path = policy_path()
    if not os.path.exists(path):
        return None, "policy 不存在: %s" % path
    misplaced = check_anchor_location(path)
    if misplaced:
        return None, misplaced
    try:
        policy = load_policy(path)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        return None, "policy 无法解析: %s" % exc
    digest = sha256(SELF)
    if policy.get("script_sha256") != digest:
        return None, ("脚本哈希与 policy 不一致（改过脚本必须重新生成并复制 policy）\n  policy: %s\n  实际  : %s"
                      % (policy.get("script_sha256"), digest))
    return policy, None


def describe(policy):
    print("脚本    : %s" % SELF)
    print("sha256  : %s" % sha256(SELF))
    print("policy  : %s" % policy_path())
    print("运行时  : %s" % runtime_dir())
    print("工作目录: %s" % policy["workdir"])
    print("空闲上限: %ss  最长存活: %ss  单动作上限: %ss  并发上限: %s  结果保留: %ss"
          % (policy.get("idle_timeout_sec", 0), policy.get("max_lifetime_sec", 0),
             policy.get("action_timeout_sec", DEFAULT_ACTION_TIMEOUT),
             policy.get("max_concurrent_actions", DEFAULT_MAX_CONCURRENT),
             policy.get("result_ttl_sec", DEFAULT_RESULT_TTL)))
    print("允许的动作（唯一来源是 policy）:")
    for name in sorted(policy["actions"]):
        print("  %-10s %s" % (name, " ".join(policy["actions"][name])))
    print("控制字（无需登记，只减少能力）: %s" % ", ".join(CONTROL))


def suggested_policy():
    just = shutil.which("just") or "/usr/bin/just"
    return {
        "script_sha256": sha256(SELF),
        "workdir": REPO,
        "idle_timeout_sec": 900,
        "max_lifetime_sec": 14400,
        "action_timeout_sec": DEFAULT_ACTION_TIMEOUT,
        "max_concurrent_actions": DEFAULT_MAX_CONCURRENT,
        "result_ttl_sec": DEFAULT_RESULT_TTL,
        "actions": {
            "check": [just, "check"],
            "test": [just, "test"],
            "dev": [just, "dev"],
        },
    }


def child_env(policy):
    env = dict(BASE_ENV)
    env["PATH"] = FIXED_PATH
    env["HOME"] = os.path.expanduser("~")
    for key, value in (policy.get("env") or {}).items():
        if isinstance(key, str) and isinstance(value, str):
            env[key] = value
    return env


def spawn(policy, argv, tag, own_group=False):
    log("[%s] start %s" % (tag, " ".join(argv)))
    pid = os.fork()
    if pid == 0:
        try:
            # 动作自成进程组：回收时能一次带走整棵树（just → cargo → nextest → 用例），
            # 且不波及执行器自己。dev 不能这么设 —— 它的监护进程随后要 setsid，
            # 而进程组组长调 setsid 会 EPERM。
            if own_group:
                os.setpgid(0, 0)
            out = os.open(at("out.log"), os.O_WRONLY | os.O_APPEND)
            os.dup2(out, 1)
            os.dup2(out, 2)
            null = os.open("/dev/null", os.O_RDONLY)
            os.dup2(null, 0)
            os.chdir(policy["workdir"])
            os.execve(argv[0], argv, child_env(policy))
        except BaseException as exc:
            log("[%s] exec failed: %s" % (tag, exc))
        os._exit(127)
    return pid


def exit_code(status):
    if os.WIFEXITED(status):
        return os.WEXITSTATUS(status)
    if os.WIFSIGNALED(status):
        return -os.WTERMSIG(status)
    return -1


def open_root(create=False):
    root = req_root()
    if create:
        os.makedirs(root, mode=0o700, exist_ok=True)
    fd = os.open(root, DIR_FLAGS)
    if not stat.S_ISDIR(os.fstat(fd).st_mode):
        os.close(fd)
        raise OSError(errno.ENOTDIR, "不是目录", root)
    return fd


def open_req(root_fd, rid):
    """打开一个请求目录：符号链接或不存在的目录一律返回 None。"""
    try:
        fd = os.open(rid, DIR_FLAGS, dir_fd=root_fd)
    except OSError:
        return None
    if not stat.S_ISDIR(os.fstat(fd).st_mode):
        os.close(fd)
        return None
    return fd


def stat_at(dir_fd, name):
    try:
        return os.stat(name, dir_fd=dir_fd, follow_symlinks=False)
    except OSError:
        return None


def read_file_at(dir_fd, name, limit):
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=dir_fd)
    except OSError:
        return None
    try:
        data = os.read(fd, limit)
        if os.read(fd, 1):
            return None
        return data.decode("utf-8", "replace")
    except OSError:
        return None
    finally:
        os.close(fd)


def write_file_at(dir_fd, name, text):
    """先写临时名再 rename：读取方读不到不完整的文件，且 rename 不跟随末段符号链接。"""
    tmp = ".%s.tmp" % name
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600, dir_fd=dir_fd)
    try:
        os.write(fd, text.encode())
    finally:
        os.close(fd)
    os.replace(tmp, name, src_dir_fd=dir_fd, dst_dir_fd=dir_fd)


def unlink_at(dir_fd, name):
    try:
        os.unlink(name, dir_fd=dir_fd)
        return True
    except OSError:
        return False


def wake(dir_fd, text):
    """唤醒等待方：调用方保证结果先落盘，于是被唤醒时结果必定已可读。"""
    info = stat_at(dir_fd, "wait")
    if info is None or not stat.S_ISFIFO(info.st_mode):
        return False
    try:
        fd = os.open("wait", os.O_WRONLY | os.O_NONBLOCK | os.O_NOFOLLOW, dir_fd=dir_fd)
    except OSError:
        # ENXIO = 当前没有读者（等待方已经离开或还没开始等）。结果仍在 result.json 里。
        return False
    try:
        os.write(fd, text.encode())
    except OSError:
        return False
    finally:
        os.close(fd)
    return True


def publish(dir_fd, rid, action, status, rc, started, note=None):
    now = time.time()
    payload = {
        "request": rid,
        "action": action,
        "status": status,
        "rc": rc,
        "started": round(started, 3),
        "seconds": round(now - started, 3),
        "finished": round(now, 3),
    }
    if note:
        payload["note"] = note
    try:
        write_file_at(dir_fd, "result.json", json.dumps(payload, ensure_ascii=False, sort_keys=True) + "\n")
    except OSError as exc:
        log("[%s] result publish failed: %s" % (rid, exc))
        return
    woken = wake(dir_fd, "rc=%s status=%s\n" % (rc, status))
    log("[%s] %s action=%s rc=%s in %.1fs woken=%s" % (rid, status, action, rc, now - started, woken))


def read_dev_record():
    try:
        with open(at("dev.json")) as handle:
            record = json.load(handle)
        int(record["pgid"])
        return record
    except (OSError, ValueError, KeyError, json.JSONDecodeError):
        return None


def read_dev():
    record = read_dev_record()
    if record is None:
        return None
    try:
        pgid = int(record["pgid"])
        os.killpg(pgid, 0)
    except (OSError, ValueError, KeyError):
        return None
    return pgid


def write_dev_exit(code):
    try:
        with open(at("dev-exit.json"), "w") as handle:
            json.dump({"rc": code, "at": round(time.time(), 3)}, handle)
    except OSError:
        pass


def read_dev_exit():
    try:
        with open(at("dev-exit.json")) as handle:
            return int(json.load(handle)["rc"])
    except (OSError, ValueError, KeyError, json.JSONDecodeError):
        return None


def kill_pgid(pgid, timeout=8.0):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(pgid, sig)
        except OSError:
            return True
        deadline = time.time() + timeout / 2
        while time.time() < deadline:
            time.sleep(0.1)
            try:
                os.killpg(pgid, 0)
            except OSError:
                return True
    return False


def stop_dev():
    pgid = read_dev()
    if pgid is not None:
        kill_pgid(pgid)
    try:
        os.unlink(at("dev.json"))
    except OSError:
        pass
    return pgid


def sweep_orphan():
    """上一次运行被 SIGKILL 时留下的进程组：属主已死则回收，并给等待方一个结局。"""
    record = None
    try:
        with open(at("dev.json")) as handle:
            record = json.load(handle)
        owner = int(record["owner"])
        pgid = int(record["pgid"])
    except (OSError, ValueError, KeyError, json.JSONDecodeError):
        record = None
    if record is not None:
        try:
            os.kill(owner, 0)
        except OSError:
            log("[shd] sweep orphan dev owner=%s pgid=%s" % (owner, pgid))
            kill_pgid(pgid)
            try:
                os.unlink(at("dev.json"))
            except OSError:
                pass
    try:
        root_fd = open_root(create=True)
    except OSError as exc:
        log("[shd] sweep skipped: %s" % exc)
        return
    try:
        for entry in os.scandir(root_fd):
            rid = entry.name
            if not ID_RE.match(rid):
                continue
            dir_fd = open_req(root_fd, rid)
            if dir_fd is None:
                continue
            try:
                raw = read_file_at(dir_fd, "process.json", 512)
                if raw is None:
                    continue
                item = json.loads(raw)
                owner = int(item["owner"])
                pgid = int(item["pgid"])
                action = str(item.get("action") or "?")
                started = float(item.get("started") or time.time())
                try:
                    os.kill(owner, 0)
                    continue
                except OSError:
                    pass
                # 回收不依赖结果是否已经发布：上一次执行器被 SIGKILL 时留下的进程组
                # 仍然占着 app 或用例进程，必须带走。
                log("[shd] sweep orphan %s owner=%s pgid=%s" % (rid, owner, pgid))
                kill_pgid(pgid)
                if stat_at(dir_fd, "result.json") is None:
                    publish(dir_fd, rid, action, "orphaned", -9, started,
                            note="上一次执行器被强制终止")
            except (OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
                log("[%s] sweep skipped: %s" % (rid, exc))
            finally:
                os.close(dir_fd)
    finally:
        os.close(root_fd)


def dev_term(signum, _frame):
    """收到终止信号先留下退出码，再带走整个进程组：等待方拿到的结局不含猜测。"""
    # 先忽略同类信号：killpg 会把自己一并打到，否则本函数会递归重入（实测 990 层后
    # RecursionError）。
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(sig, signal.SIG_IGN)
    write_dev_exit(-int(signum))
    try:
        os.killpg(os.getpgid(0), signal.SIGTERM)
    except OSError:
        pass
    time.sleep(0.3)
    try:
        os.killpg(os.getpgid(0), signal.SIGKILL)
    except OSError:
        pass
    os._exit(0)


def supervise(policy, argv):
    owner = os.getppid()
    os.setsid()
    # 登记由监护进程自己完成（setsid 之后）。交给执行器登记会与其 setsid 抢时序：
    # 执行器在那之前 killpg(pgid, 0) 得到 ESRCH，会误判 dev 已经结束。
    with open(at("dev.json"), "w") as handle:
        json.dump({"pgid": os.getpid(), "owner": owner, "started": time.time()}, handle)
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(sig, dev_term)
    pid = spawn(policy, argv, "dev")
    log("[shd] supervise pid=%d owner=%d" % (pid, owner))
    while True:
        try:
            done, status = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            done, status = pid, 0
        if done == pid:
            code = exit_code(status)
            log("[dev] exit %d" % code)
            write_dev_exit(code)
            try:
                os.killpg(os.getpgid(0), signal.SIGTERM)
            except OSError:
                pass
            return 0
        if os.getppid() != owner:
            log("[shd] supervise owner gone, killing dev group")
            write_dev_exit(-9)
            try:
                os.killpg(os.getpgid(0), signal.SIGTERM)
            except OSError:
                pass
            time.sleep(1)
            try:
                os.killpg(os.getpgid(0), signal.SIGKILL)
            except OSError:
                pass
            return 0
        time.sleep(0.5)


def rotate():
    try:
        if os.path.getsize(at("out.log")) > LOG_MAX:
            os.replace(at("out.log"), at("out.log.1"))
    except OSError:
        pass

class Runner:
    def __init__(self, policy):
        self.policy = policy
        self.stopping = False
        # 按请求 id 记账：并发请求互不覆盖，任一个的结局都能单独取回。
        self.actions = {}
        # waitpid(-1) 可能先抢走结局，抢到就暂存在这里，避免丢失退出码。
        self.reaped = {}
        self.dev_request = None
        self.result_ttl = int(policy.get("result_ttl_sec", DEFAULT_RESULT_TTL) or 0)
        self.next_prune = 0.0

    def max_concurrent(self):
        value = self.policy.get("max_concurrent_actions", DEFAULT_MAX_CONCURRENT)
        return int(value or DEFAULT_MAX_CONCURRENT)

    def refresh_state(self):
        write_state(
            pid=os.getpid(),
            status="running" if (self.actions or self.dev_request) else "idle",
            dev_pgid=read_dev(),
            requests=[
                {"request": rid, "action": item["action"], "pgid": item["pgid"],
                 "seconds": round(time.time() - item["started"], 1)}
                for rid, item in sorted(self.actions.items())
            ],
        )

    def scan(self):
        """收到 ready 标记即接纳；动作名不在 policy 中即回 rejected —— 不接纳任何参数。"""
        try:
            root_fd = open_root(create=True)
        except OSError as exc:
            log("[shd] request dir unavailable: %s" % exc)
            return
        try:
            for entry in os.scandir(root_fd):
                rid = entry.name
                if not ID_RE.match(rid) or rid in self.actions:
                    continue
                if self.dev_request and self.dev_request["request"] == rid:
                    continue
                self.consider(root_fd, rid)
        finally:
            os.close(root_fd)

    def consider(self, root_fd, rid):
        dir_fd = open_req(root_fd, rid)
        if dir_fd is None:
            return
        keep = False
        try:
            if stat_at(dir_fd, "result.json") is not None:
                return
            if stat_at(dir_fd, "process.json") is not None:
                return
            if stat_at(dir_fd, "ready") is None:
                return
            raw = read_file_at(dir_fd, "action", 128)
            action = (raw or "").strip()
            if not NAME_RE.match(action) or action not in self.policy["actions"]:
                publish(dir_fd, rid, action or "?", "rejected", None, time.time(),
                        note="动作未在 policy 中登记")
                return
            if action == "dev":
                self.start_dev(dir_fd, rid, action)
                keep = True
                return
            if len(self.actions) >= self.max_concurrent():
                publish(dir_fd, rid, action, "busy", None, time.time(),
                        note="在飞动作已达上限 %d" % self.max_concurrent())
                return
            started = time.time()
            pid = spawn(self.policy, self.policy["actions"][action], action, own_group=True)
            record = {"pgid": pid, "owner": os.getpid(), "action": action, "started": round(started, 3)}
            write_file_at(dir_fd, "process.json", json.dumps(record))
            self.actions[rid] = {"action": action, "pgid": pid, "started": started, "dir_fd": dir_fd}
            keep = True
        except OSError as exc:
            log("[%s] intake failed: %s" % (rid, exc))
        finally:
            if not keep:
                os.close(dir_fd)

    def start_dev(self, dir_fd, rid, action):
        pgid = read_dev()
        if pgid is not None:
            publish(dir_fd, rid, action, "busy", None, time.time(), note="dev 已在运行 pgid=%s" % pgid)
            return
        if self.dev_request is not None:
            publish(dir_fd, rid, action, "busy", None, time.time(), note="另一个 dev 请求已在飞")
            return
        argv = self.policy["actions"][action]
        watchdog = [sys.executable, SELF, "--supervise", "--"] + argv
        started = time.time()
        pid = spawn(self.policy, watchdog, action)
        # dev.json 由监护进程在 setsid 之后自己写（见 supervise）。
        record = {"pgid": pid, "owner": os.getpid(), "action": action, "kind": "dev",
                  "started": round(started, 3)}
        write_file_at(dir_fd, "process.json", json.dumps(record))
        try:
            os.unlink(at("dev-exit.json"))
        except OSError:
            pass
        self.dev_request = {"request": rid, "action": action, "started": started, "dir_fd": dir_fd}

    def reap(self):
        while True:
            try:
                pid, status = os.waitpid(-1, os.WNOHANG)
            except (ChildProcessError, OSError):
                return
            if pid == 0:
                return
            for item in self.actions.values():
                if item["pgid"] == pid:
                    self.reaped[pid] = status
                    break

    def finish(self, rid, item, status, rc, note=None):
        publish(item["dir_fd"], rid, item["action"], status, rc, item["started"], note)
        os.close(item["dir_fd"])
        del self.actions[rid]

    def poll_actions(self):
        """判定每个动作的结局与墙钟上限 —— 主循环因此从不等它。"""
        for rid, item in list(self.actions.items()):
            status = self.reaped.pop(item["pgid"], None)
            if status is None:
                try:
                    done, raw = os.waitpid(item["pgid"], os.WNOHANG)
                except (ChildProcessError, OSError):
                    done, raw = item["pgid"], 0
                if done == item["pgid"]:
                    status = raw
            if status is not None:
                self.finish(rid, item, "done", exit_code(status))
                continue
            limit = int(self.policy.get("action_timeout_sec") or 0)
            if limit > 0 and time.time() - item["started"] > limit:
                log("[%s] timeout after %ss pgid=%s" % (rid, limit, item["pgid"]))
                kill_pgid(item["pgid"])
                self.finish(rid, item, "timeout", -9, note="超过 action_timeout_sec=%s" % limit)

    def poll_dev(self):
        if self.dev_request is None:
            return
        if read_dev() is not None:
            return
        item = self.dev_request
        rc = read_dev_exit()
        if rc is None and time.time() - item["started"] < DEV_GRACE:
            return
        self.dev_request = None
        if rc is not None:
            publish(item["dir_fd"], item["request"], item["action"], "done", rc, item["started"])
        else:
            why = ("dev 已结束，但监护进程没有留下退出码" if read_dev_record() is not None
                   else "监护进程没有登记，dev 未能启动")
            publish(item["dir_fd"], item["request"], item["action"], "error", -1, item["started"],
                    note=why)
        os.close(item["dir_fd"])
        for name in ("dev.json", "dev-exit.json"):
            try:
                os.unlink(at(name))
            except OSError:
                pass

    def prune(self):
        if self.result_ttl <= 0 or time.time() < self.next_prune:
            return
        self.next_prune = time.time() + 60
        try:
            root_fd = open_root()
        except OSError:
            return
        try:
            for entry in os.scandir(root_fd):
                rid = entry.name
                if not ID_RE.match(rid) or rid in self.actions:
                    continue
                if self.dev_request and self.dev_request["request"] == rid:
                    continue
                dir_fd = open_req(root_fd, rid)
                if dir_fd is None:
                    continue
                drop = False
                try:
                    info = stat_at(dir_fd, "result.json")
                    # 没有结果的目录同样要清理：客户端可能在中途消失，提交因而不完整。
                    # 在飞的动作与 dev 请求在更上面已经跳过，剩下的都没有归属。
                    stamp = info.st_mtime if info is not None else os.fstat(dir_fd).st_mtime
                    if time.time() - stamp >= self.result_ttl:
                        drop = True
                        for name in ("action", "ready", "wait", "process.json", "result.json",
                                     ".result.json.tmp", ".process.json.tmp"):
                            unlink_at(dir_fd, name)
                finally:
                    os.close(dir_fd)
                if drop:
                    try:
                        os.rmdir(rid, dir_fd=root_fd)
                    except OSError:
                        pass
        finally:
            os.close(root_fd)

    def shutdown(self, why):
        if self.stopping:
            return
        self.stopping = True
        log("[shd] shutdown: %s" % why)
        # 先给等待方一个结局，再回收进程组：否则等待方要等到自己的超时才拿到答案。
        for rid, item in list(self.actions.items()):
            log("[%s] kill action group pgid=%s" % (rid, item["pgid"]))
            kill_pgid(item["pgid"])
            try:
                os.waitpid(item["pgid"], os.WNOHANG)
            except (ChildProcessError, OSError):
                pass
            self.finish(rid, item, "aborted", -9, note="执行器停止: %s" % why)
        if self.dev_request is not None:
            item = self.dev_request
            self.dev_request = None
            stop_dev()
            publish(item["dir_fd"], item["request"], item["action"], "aborted", -9, item["started"],
                    note="执行器停止: %s" % why)
            os.close(item["dir_fd"])
        else:
            stop_dev()
        for name in ("dev-exit.json", "runner.pid"):
            try:
                os.unlink(at(name))
            except OSError:
                pass
        write_state(status="stopped", pid=os.getpid(), control="shutdown", note=why)

    def control(self, token):
        if token == "stop":
            self.shutdown("requested stop")
            os._exit(0)
        if token == "stop-dev":
            log("[shd] stop-dev requested")
            stop_dev()
            self.poll_dev()
            return
        if token in self.policy["actions"]:
            log("[shd] reject bare action %r: 动作必须经 req/<id> 投递" % token)
            write_state(status="rejected", control=token, note="动作必须经 req/<id> 投递")
            return
        log("[shd] reject unknown control %r" % token)
        write_state(status="rejected", control=token)

    def serve(self):
        owner = os.getppid()
        try:
            master, slave = pty.openpty()
            os.close(master)
            os.close(slave)
        except OSError as exc:
            print("PTY 不可用（%s）：拒绝启动 —— 本进程的存在理由就是在沙箱外提供 pty" % exc)
            return 1
        # 等待方提前离开时 write 只报 EPIPE，不终止本进程。
        signal.signal(signal.SIGPIPE, signal.SIG_IGN)
        # 先清过期请求，再回收孤儿：过期的进程组记录不进入扫描范围（pgid 可能已被复用）。
        self.prune()
        sweep_orphan()
        with open(at("runner.pid"), "w") as handle:
            handle.write(str(os.getpid()))
        with open(at("request"), "w") as handle:
            handle.write("none")
        seen = ("none", os.stat(at("request")).st_mtime_ns, os.path.getsize(at("request")))
        log("[shd] ready pid=%d owner=%d actions=%s protocol=v2"
            % (os.getpid(), owner, ",".join(sorted(self.policy["actions"]))))
        self.refresh_state()
        idle_limit = int(self.policy.get("idle_timeout_sec") or 0)
        life_limit = int(self.policy.get("max_lifetime_sec") or 0)
        born = time.time()
        last = born
        for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            signal.signal(sig, lambda *_: self.shutdown("signal"))
        while True:
            self.reap()
            self.poll_actions()
            self.scan()
            self.poll_dev()
            self.prune()
            # 停止过程中不再覆盖 state.json：停机结论由 shutdown 写。
            if not self.stopping:
                self.refresh_state()
            rotate()
            if self.stopping:
                return 0
            if os.getppid() != owner:
                self.shutdown("owner gone")
                return 0
            if life_limit > 0 and time.time() - born > life_limit:
                self.shutdown("max lifetime")
                return 0
            if (idle_limit > 0 and not self.actions and self.dev_request is None
                    and read_dev() is None and time.time() - last > idle_limit):
                self.shutdown("idle timeout")
                return 0
            try:
                token = open(at("request")).read().strip()
                stamp = (token, os.stat(at("request")).st_mtime_ns, os.path.getsize(at("request")))
            except OSError:
                stamp = None
            if stamp is not None and stamp != seen:
                seen = stamp
                last = time.time()
                try:
                    self.control(token)
                except BaseException as exc:
                    log("[shd] control failed: %s" % exc)
            time.sleep(POLL)


def prepare_runtime():
    path = runtime_dir()
    if os.path.islink(path):
        print("运行时目录是符号链接，拒绝使用: %s" % path)
        return False
    os.makedirs(path, mode=0o700, exist_ok=True)
    if os.stat(path).st_uid != os.getuid():
        print("运行时目录不属于当前用户: %s" % path)
        return False
    os.chmod(path, 0o700)
    return True


def runner_pid():
    try:
        pid = int(open(at("runner.pid")).read().strip())
        os.kill(pid, 0)
        return pid
    except (OSError, ValueError):
        return None


def read_result(path):
    try:
        with open(os.path.join(path, "result.json")) as handle:
            result = json.load(handle)
    except (OSError, ValueError, json.JSONDecodeError):
        return None
    return result if isinstance(result, dict) else None


def list_requests():
    """列出请求目录：在飞（无结果）与已收尾（有结果）各一份清单。"""
    inflight = []
    finished = []
    try:
        names = sorted(os.listdir(req_root()))
    except OSError:
        return inflight, finished
    for rid in names:
        if not ID_RE.match(rid):
            continue
        path = req_path(rid)
        if os.path.islink(path):
            continue
        result = read_result(path)
        if result is not None:
            finished.append(result)
            continue
        action = "?"
        try:
            with open(os.path.join(path, "action")) as handle:
                action = handle.read().strip() or "?"
        except OSError:
            pass
        try:
            started = os.stat(os.path.join(path, "ready")).st_mtime
        except OSError:
            started = time.time()
        inflight.append({"request": rid, "action": action,
                         "seconds": round(time.time() - started, 1)})
    finished.sort(key=lambda item: item.get("finished") or 0)
    return inflight, finished


def tail_log(count):
    if count <= 0:
        return []
    try:
        size = os.path.getsize(at("out.log"))
        with open(at("out.log"), "rb") as handle:
            handle.seek(max(0, size - 65536))
            data = handle.read()
    except OSError:
        return []
    return data.decode("utf-8", "replace").splitlines()[-count:]


def report(result, tail):
    print("request=%s action=%s status=%s rc=%s 用时=%ss"
          % (result.get("request"), result.get("action"), result.get("status"),
             result.get("rc"), result.get("seconds")))
    if result.get("note"):
        print("note=%s" % result["note"])
    for line in tail_log(tail):
        print("| %s" % line)


def exit_code_of(result):
    status = result.get("status")
    if status in ("done", "timeout"):
        rc = result.get("rc")
        if isinstance(rc, int):
            return rc if rc >= 0 else 128 + min(-rc, 127)
        return 1
    return 3


def submit(action):
    rid = os.urandom(8).hex()
    os.makedirs(req_root(), mode=0o700, exist_ok=True)
    path = req_path(rid)
    os.mkdir(path, 0o700)
    with open(os.path.join(path, "action"), "w") as handle:
        handle.write(action + "\n")
    # 顺序固定：等待通道先就位，ready 才意味着"可以接纳了"。
    os.mkfifo(os.path.join(path, "wait"), 0o600)
    with open(os.path.join(path, "ready"), "w") as handle:
        handle.write("%d\n" % os.getpid())
    return rid


def await_request(rid, timeout):
    if not ID_RE.match(rid):
        return None, "请求 id 非法: %s" % rid
    path = req_path(rid)
    if not os.path.isdir(path):
        return None, "未知请求 id: %s（目录不存在或已被清理）" % rid
    fifo = os.path.join(path, "wait")
    try:
        # 先把读端打开，再查结果：此后任何完成都能看到本读者（写入端 O_NONBLOCK 成功），
        # 于是"结果先落盘、读端后打开"与"读端已就位、结果才落盘"两种次序都不会漏。
        fd = os.open(fifo, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    except OSError as exc:
        return None, "无法打开等待通道 %s: %s" % (fifo, exc)
    try:
        result = read_result(path)
        if result is not None:
            return result, None
        if runner_pid() is None:
            return None, "执行器未运行，该请求也没有结果"
        deadline = time.time() + max(0.0, float(timeout))
        while True:
            remaining = deadline - time.time()
            if remaining <= 0:
                return None, "pending"
            # select 是内核等待，不是文件系统轮询：只在有写入或 EOF 时才返回。
            if select.select([fd], [], [], min(1.0, remaining))[0]:
                try:
                    os.read(fd, 256)
                except OSError:
                    pass
                result = read_result(path)
                if result is not None:
                    return result, None
    finally:
        os.close(fd)


def parse_options(args):
    options = {"timeout": CLIENT_WAIT_DEFAULT, "tail": LOG_TAIL}
    rest = []
    index = 0
    while index < len(args):
        item = args[index]
        if item in ("--timeout", "--tail"):
            index += 1
            if index >= len(args):
                raise ValueError("%s 缺少取值" % item)
            try:
                options[item.lstrip("-")] = int(args[index])
            except ValueError:
                raise ValueError("%s 的取值不是整数: %r" % (item, args[index]))
        else:
            rest.append(item)
        index += 1
    return options, rest


def client_wait(rid, options):
    result, reason = await_request(rid, options["timeout"])
    if result is not None:
        report(result, options["tail"])
        return exit_code_of(result)
    if reason == "pending":
        print("request=%s status=running 等待超时（%ss）：动作仍在执行，结果不会丢"
              % (rid, options["timeout"]))
        print("重新等待：just runner-wait %s    看一眼：just runner-result %s" % (rid, rid))
        return 4
    print(reason)
    return 3


def client_result(rid):
    if not ID_RE.match(rid):
        print("请求 id 非法: %s" % rid)
        return 3
    path = req_path(rid)
    if not os.path.isdir(path):
        print("未知请求 id: %s（目录不存在或已被清理）" % rid)
        return 3
    result = read_result(path)
    if result is None:
        print("request=%s status=running（尚未结束）" % rid)
        return 4
    report(result, 0)
    # 与 --wait 同一套退出码：查询本身成功，但仍把动作的结局报告给调用方。
    return exit_code_of(result)


def client_status():
    policy, why = verify()
    if policy is not None:
        describe(policy)
    else:
        print("policy 未通过：%s" % why)
    blocked = platform_guard()
    if blocked:
        print("平台：%s" % blocked)
    pid = runner_pid()
    if pid is None:
        print("执行器：stopped")
    else:
        print("执行器：running pid=%d dev_pgid=%s" % (pid, read_dev()))
    inflight, finished = list_requests()
    for item in inflight:
        print("  在飞 request=%s action=%s 已等待=%ss"
              % (item["request"], item["action"], item["seconds"]))
    for item in finished[-5:]:
        print("  已完成 request=%s action=%s status=%s rc=%s 用时=%ss"
              % (item.get("request"), item.get("action"), item.get("status"),
                 item.get("rc"), item.get("seconds")))
    return 0


def main():
    args = sys.argv[1:]
    command = args[0] if args else "--check"
    os.umask(0o077)
    if os.geteuid() == 0:
        print("拒绝以 root 运行")
        return 1
    blocked = platform_guard()
    if blocked and command != "--print-policy":
        print("未通过：%s" % blocked)
        return 1
    if command == "--print-policy":
        out = args[args.index("--out") + 1] if "--out" in args else None
        text = json.dumps(suggested_policy(), indent=2, ensure_ascii=False) + "\n"
        if out:
            with open(out, "w") as handle:
                handle.write(text)
            print("已写出样板: %s" % out)
        sys.stdout.write(text)
        print("把上面内容复制为 %s（必须在工作区与临时目录之外，否则 --check 会拒绝）" % policy_path(),
              file=sys.stderr)
        return 0
    if command == "--supervise":
        policy, why = verify()
        if policy is None:
            print("拒绝启动：%s" % why)
            return 1
        return supervise(policy, args[args.index("--") + 1:])
    if not prepare_runtime():
        return 1
    if command == "--check":
        policy, why = verify()
        if policy is not None:
            describe(policy)
            print("校验通过")
            return 0
        print("未通过：%s" % why)
        print("生成样板：just runner-policy（把输出复制到 %s）" % policy_path())
        return 1
    if command == "--status":
        return client_status()
    if command == "--stop":
        pid = runner_pid()
        if pid is None:
            print("执行器：stopped")
            return 0
        os.kill(pid, signal.SIGTERM)
        print("已请求停止 pid=%d" % pid)
        return 0
    if command == "--stop-dev":
        if runner_pid() is None:
            print("执行器未运行：dev 不归它管，先执行 just runner-start")
            return 3
        with open(at("request"), "w") as handle:
            handle.write("stop-dev")
        print("已请求回收 dev（执行器在一个轮询周期内处理）")
        return 0
    try:
        options, rest = parse_options(args[1:])
    except ValueError as exc:
        print("参数错误：%s" % exc)
        return 2
    if command in ("--submit", "--run"):
        if len(rest) != 1:
            print("用法: agent-runner.py %s ACTION [--timeout N]" % command)
            return 2
        if not NAME_RE.match(rest[0]):
            print("动作名非法: %r" % rest[0])
            return 3
        if runner_pid() is None:
            print("执行器未运行：先执行 just runner-start（需要一次提权）")
            return 3
        rid = submit(rest[0])
        print("request=%s action=%s 已提交" % (rid, rest[0]))
        if command == "--submit":
            print("阻塞等待：just runner-wait %s    看一眼：just runner-result %s" % (rid, rid))
            return 0
        return client_wait(rid, options)
    if command == "--wait":
        if len(rest) != 1:
            print("用法: agent-runner.py --wait ID [--timeout N]")
            return 2
        return client_wait(rest[0], options)
    if command == "--result":
        if len(rest) != 1:
            print("用法: agent-runner.py --result ID")
            return 2
        return client_result(rest[0])
    if command != "--serve":
        print("用法: agent-runner.py --print-policy [--out PATH] | --check | --serve",
              "| --status | --stop | --stop-dev | --run ACTION | --submit ACTION",
              "| --wait ID | --result ID")
        return 2
    policy, why = verify()
    if policy is None:
        print("拒绝启动：%s" % why)
        return 1
    existing = runner_pid()
    if existing is not None:
        print("已在运行 pid=%d" % existing)
        return 0
    describe(policy)
    print("常驻启动（随本作业结束而退出）")
    sys.stdout.flush()
    return Runner(policy).serve()


if __name__ == "__main__":
    sys.exit(main())
