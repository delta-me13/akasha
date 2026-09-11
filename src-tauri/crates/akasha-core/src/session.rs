use std::fmt;

/// `Session` 的**不透明标识**。
///
/// 新类型而不是裸 `u64`：ID 会穿过注册表、事件、日志三处，而裸 `u64` 一旦被顺手
/// 当成"序号"或"计数"用，编译器一句话都不会说。包成新类型后这类误用在类型上就写不出来。
///
/// 唯一的分配入口是 `SessionRegistry::open`：这里刻意**不**实现 `From<u64>`，
/// 也不对外提供 `from_raw` —— 于是"手搓一个 ID"在 crate 外不可能。
///
/// ⚠️ 阶段 3 接 IPC 时才会需要 `serde` 反序列化构造；那时再按 IPC 的边界补，
/// 现在补等于提前把"外部可以构造 ID"这个口子开出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(u64);

impl SessionId {
    /// 内部用：只有注册表的分配逻辑会调它。
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// 取原始值。只用于日志与将来的序列化边界，**不用它做业务判断**。
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session#{}", self.0)
    }
}

/// `Session` 的种类。**平级**，没有主次（`docs/scope.md` §5.1）：
/// SFTP 不需要先开终端，转发也不需要。
///
/// 枚举而不是"终端 + 附件列表"：后者会把"终端 = 应用"的假设写进架构，
/// 之后拆它很贵 —— 这正是本模型要先于任何后端落地的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionKind {
    /// 本地 PTY 或 SSH shell：它拥有一条 `Transport`（plan 0105）。
    Terminal,
    /// 双栏文件传输：拥有两侧连接与传输任务。
    Sftp,
    /// 端口转发：拥有它的一条或多条隧道。
    Tunnel,
    /// 凭据库：无长连接，只有 SQLite（阶段 4）。
    Vault,
}

impl SessionKind {
    /// 全部种类。用于"对所有种类都是同一套语义"的测试与将来的 UI 遍历。
    pub const ALL: [SessionKind; 4] = [
        SessionKind::Terminal,
        SessionKind::Sftp,
        SessionKind::Tunnel,
        SessionKind::Vault,
    ];

    /// 日志用的稳定短名。**不是序列化格式** —— IPC 的表示要由阶段 3 的类型生成定案。
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionKind::Terminal => "terminal",
            SessionKind::Sftp => "sftp",
            SessionKind::Tunnel => "tunnel",
            SessionKind::Vault => "vault",
        }
    }
}

impl fmt::Display for SessionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 注册表操作的失败原因。
///
/// 没有 `panic` 分支：`AGENTS.md` §0 禁止 `unwrap()` / `expect()` 出现在
/// command 边界与长驻任务中，而注册表将来正好由 command 与长驻任务持有。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    /// 这个 ID 没被登记：**已经关闭**，或者从来不存在。两者的处置相同，故不区分。
    NotFound(SessionId),
    /// ID 空间耗尽（`u64` 用尽）。现实中到不了，但不许 panic，也不许绕回复用旧 ID。
    IdExhausted,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::NotFound(id) => write!(f, "{id} 未登记（已关闭或不存在）"),
            SessionError::IdExhausted => f.write_str("SessionId 空间已耗尽"),
        }
    }
}

impl std::error::Error for SessionError {}
