//! 认证材料：**候选私钥**与它们的选择顺序（ADR-0003 D7）。
//!
//! 私钥以 **PEM 字节**的形式进来，住在一页受保护内存里（ADR-0002 D13）——
//! 与密钥池那一页是**同一个原语**（`akasha_store::protected::Protected`），
//! 不是另抄一份。从密钥池取出 PEM 是**调用方**的事（app 持有解锁后的库），
//! 本 crate 只负责"拿这些候选去认证"。
//!
//! ⚠️ 握手时需要一份**普通堆上的明文私钥**（`russh` 要 `ssh_key::PrivateKey` 才能签名），
//! 它无法放进受保护页。缓解 = 只在握手窗口内存在、签完即 drop、**不进缓存**（D8）。

use akasha_store::pools::keys::MAX_PEM_LEN;
use akasha_store::protected::{Exposed, Protected};
use russh::keys::{HashAlg, decode_secret_key};

use crate::error::SshError;

/// 一把候选私钥：**调用方给的稳定标识** + PEM 字节（受保护页）。
pub struct KeyCandidate {
    id: String,
    pem: Protected<MAX_PEM_LEN>,
}

impl KeyCandidate {
    /// 从 PEM 字节造一个候选。成功时源缓冲已被擦零。
    ///
    /// `id` 会被放进凭据缓存的键（[`crate::CredentialKind::KeyPassphrase`]），
    /// 所以它必须**跟着密钥材料变**：换了一把钥匙就得换标识，
    /// 否则缓存里那句口令会拿去解新钥匙（症状是"解密失败"，真实原因却在缓存）。
    /// 密钥池的行 id 配上更新时间戳是够用的形态。
    pub fn new(id: impl Into<String>, pem: Vec<u8>) -> Result<Self, SshError> {
        Ok(Self {
            id: id.into(),
            pem: Protected::new(pem)?,
        })
    }

    /// 稳定标识（缓存键的一部分，也是日志字段）。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 借出 PEM 文本给解析器用。**不留副本**（解析器要 `&str`）。
    pub(crate) fn with_pem<R>(&mut self, f: impl FnOnce(&str) -> R) -> Result<R, SshError> {
        let guard: Exposed<'_, MAX_PEM_LEN> = self.pem.expose()?;
        let text = std::str::from_utf8(&guard).map_err(|_| SshError::CredentialNotUtf8)?;
        Ok(f(text))
    }
}

/// 从一把私钥算出**公钥的 `SHA256:…` 指纹**（plan 0904 的离线自检）。
///
/// 为什么住在这里：解析私钥要 `ssh_key`，而那个 crate 只在本 crate 里（`russh::keys`）——
/// 让 `akasha-bw` 或 app 各自引一份，就会多出第二个版本的 `ssh-key`，而"指纹算的是不是同一个
/// 东西"正是这条判据的全部内容。
///
/// 输入是**从受保护页借出来的那一段**（`&[u8]`）：本函数不留副本。
pub fn fingerprint_of_private_key(pem: &[u8]) -> Result<String, SshError> {
    let text = std::str::from_utf8(pem).map_err(|_| SshError::CredentialNotUtf8)?;
    let key = decode_secret_key(text, None).map_err(|err| SshError::PrivateKeyUnusable {
        reason: err.to_string(),
    })?;
    Ok(key.public_key().fingerprint(HashAlg::Sha256).to_string())
}

/// 认证材料：要不要用 agent、agent 在哪、以及密钥池给出的候选私钥（**按顺序**）。
///
/// 顺序有意义：第一个被服务端接受的就用它，与 OpenSSH 的行为一致。
pub struct SshAuth {
    /// 要不要先试 SSH agent。
    ///
    /// agent 是**最干净**的一档：私钥不进我们进程、也不用问口令（D7）。
    /// 它不可用时（没设环境变量、socket 不在、agent 里一把钥匙都没有）**静默落到下一档**。
    pub use_agent: bool,
    /// agent 的 socket 路径。`None` = 看环境变量 `SSH_AUTH_SOCK`（unix）。
    ///
    /// 为什么要有它显式指定这一档：① 多用户机器 / 容器里 `SSH_AUTH_SOCK` 指向的
    /// 未必是用户想要的那个 agent；② **测试要能确定性地造出"agent 不在"** ——
    /// 而改环境变量在 Rust 2024 里是 `unsafe`，本项目只在 `akasha-store` 里允许 `unsafe`
    /// （AGENTS.md §3.4 与那三条 ast-grep 规则）。
    pub agent_socket: Option<std::path::PathBuf>,
    /// 候选私钥。
    pub keys: Vec<KeyCandidate>,
}

impl SshAuth {
    /// 只用 agent（不提供私钥）。agent 位置交给环境变量。
    pub fn agent_only() -> Self {
        Self {
            use_agent: true,
            agent_socket: None,
            keys: Vec::new(),
        }
    }

    /// 只用指定路径上的 agent。
    pub fn agent_at(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            use_agent: true,
            agent_socket: Some(path.into()),
            keys: Vec::new(),
        }
    }

    /// 不用 agent，只用给定的私钥。
    pub fn keys(keys: Vec<KeyCandidate>) -> Self {
        Self {
            use_agent: false,
            agent_socket: None,
            keys,
        }
    }
}

impl Default for SshAuth {
    fn default() -> Self {
        Self::agent_only()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn a_pem_round_trips_through_the_protected_page() {
        let pem = b"-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\n";
        let mut key = KeyCandidate::new("key#7", pem.to_vec()).unwrap();
        assert_eq!(key.id(), "key#7");
        key.with_pem(|text| {
            assert!(text.contains("OPENSSH PRIVATE KEY"));
            assert_eq!(text.len(), pem.len());
        })
        .unwrap();
    }

    /// 指纹 = 那一对密钥里公钥的 `SHA256:…`。正例与诱饵成对：
    /// 少了正例，"算不出指纹"与"算出来是别的"分不开。
    #[test]
    fn the_fingerprint_of_a_private_key_is_the_one_its_pair_reports() {
        let (pem, fingerprint) = crate::testing::key_pair();
        assert_eq!(
            fingerprint_of_private_key(pem.as_bytes()).unwrap(),
            fingerprint
        );
    }

    #[test]
    fn an_unparsable_key_is_refused_instead_of_yielding_an_empty_fingerprint() {
        assert!(matches!(
            fingerprint_of_private_key(b"-----BEGIN OPENSSH PRIVATE KEY-----\nnope\n"),
            Err(SshError::PrivateKeyUnusable { .. })
        ));
    }

    #[test]
    fn a_non_utf8_pem_is_refused_not_mangled() {
        // 私钥 PEM 一定是 ASCII；真出现坏字节就是有东西坏了，
        // 这时候**报出来**比"悄悄替换成问号再去解析"有用得多。
        let mut key = KeyCandidate::new("bad", vec![0xff, 0xfe]).unwrap();
        assert!(matches!(
            key.with_pem(|_| ()),
            Err(SshError::CredentialNotUtf8)
        ));
    }
}
