//! 存储层在 app 侧的编组：解锁 / 锁定 / 四套池的命令与状态。
//!
//! 纯逻辑在父模块（`passphrase` / `protected` / `schema` / `pools` / `export` …）；
//! 这里是把它接到 IPC 与 app 状态上的那一半（`AGENTS.md` §3.1）。

pub mod pools;
pub mod vault;
