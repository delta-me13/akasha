//! 认证：**顺序是数据，不是散落的 `if`**（ADR-0003 D7）。
//!
//! 顺序 = **agent → 密钥池 → keyboard-interactive → password**。每一档只在前一档失败
//! 之后才执行；服务端说"这一档过了、还要再来一种"（`partial_success`）时，
//! 按它给的 `remaining_methods` **续接**，不从头重来（重来会把已经通过的因子再算一遍，
//! 而服务端是按自己的顺序计失败次数的 —— 那是账号锁定的经典成因）。
//!
//! ## 这一层不做的事
//!
//! * **不重试**：连不上、认证失败、主机密钥不对——三者的重试策略在阶段 6
//!   （ADR-0003 D13）。这里的契约简单：成就是成，不成就把**是哪一类**交出去。
//! * **不落盘**：拿到的口令进的是 [`crate::ssh::CredentialCache`]（进程内存）。
//!   认证结束之后我们手上不再留明文 —— 除了无法避免的两份副本
//!   （`russh` 要的 `String` 与 `ssh_key::PrivateKey`），那两条照实记在 D8。

use std::sync::Arc;

use russh::client::KeyboardInteractiveAuthResponse;
use russh::client::{AuthResult, Handle};
use russh::keys::{Error, PrivateKey, PrivateKeyWithHashAlg, PublicKey, decode_secret_key};
use russh::{MethodKind, MethodSet};

use crate::ssh::credential::{CacheKey, Credential, CredentialKind, CredentialRequest};
use crate::ssh::error::{SshError, connect_failed};
use crate::ssh::handshake::{Handler, SshConnect};

/// keyboard-interactive 的往返次数上限。
///
/// 存在的理由：`InfoRequest` 是一个**服务端可以一直发**的消息，没有上限时
/// 一个坏掉或恶意的服务端就能把我们挂在这里（而这条路是长驻任务）。
/// 8 是"任何真实的多因子流程都够用"的取值 —— 2FA 用一次往返，PAM 堆几层也很少超过三次。
const MAX_KEYBOARD_ROUNDS: usize = 8;

/// 一档认证方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// SSH agent（`SSH_AUTH_SOCK`）。私钥不进我们进程。
    Agent,
    /// 密钥池里的候选私钥。
    KeyPool,
    /// keyboard-interactive（2FA / OTP）。
    KeyboardInteractive,
    /// 登录口令。
    Password,
}

impl Method {
    /// **顺序本身**。判据"agent 优先、keyboard-interactive 在 password 之前"
    /// 就是这一行，而它有一条用例盯着。
    pub const ORDER: [Method; 4] = [
        Method::Agent,
        Method::KeyPool,
        Method::KeyboardInteractive,
        Method::Password,
    ];

    /// 日志用的稳定短名（`docs/logging.md`）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Method::Agent => "agent",
            Method::KeyPool => "publickey",
            Method::KeyboardInteractive => "keyboard-interactive",
            Method::Password => "password",
        }
    }

    /// 对应的 SSH 协议方法名 —— 用来与服务端给的 `remaining_methods` 对账。
    const fn protocol(self) -> MethodKind {
        match self {
            Method::Agent | Method::KeyPool => MethodKind::PublicKey,
            Method::KeyboardInteractive => MethodKind::KeyboardInteractive,
            Method::Password => MethodKind::Password,
        }
    }
}

/// 一档认证的结果。
enum Step {
    /// 服务端接受了。
    Accepted,
    /// 服务端拒绝了 —— 带上它还愿意接受哪些方法、以及"这一档其实过了多少"。
    Rejected { remaining: MethodSet, partial: bool },
    /// **这一档根本没试**：agent 不可用、没有候选私钥。**不是失败**，去下一档。
    Unavailable,
}

/// 按 [`Method::ORDER`] 走完整条链。
pub(crate) async fn authenticate(
    session: &mut Handle<Handler>,
    options: &mut SshConnect,
) -> Result<(), SshError> {
    let mut allowed: Option<MethodSet> = None;

    for method in Method::ORDER {
        if let Some(allowed) = &allowed {
            // 只走服务端还愿意谈的那几档。空集 = 它一条都不愿意谈了。
            if !allowed.contains(&method.protocol()) {
                tracing::debug!(method = method.as_str(), "ssh auth method skipped");
                continue;
            }
        }

        let step = match method {
            Method::Agent => agent_step(session, options).await?,
            Method::KeyPool => key_step(session, options).await?,
            Method::KeyboardInteractive => keyboard_step(session, options).await?,
            Method::Password => password_step(session, options).await?,
        };

        match step {
            Step::Accepted => {
                tracing::info!(
                    host = options.target.host(),
                    port = options.target.port(),
                    user = options.target.user(),
                    method = method.as_str(),
                    "ssh authenticated"
                );
                return Ok(());
            }
            Step::Unavailable => {
                tracing::debug!(method = method.as_str(), "ssh auth method unavailable");
            }
            Step::Rejected { remaining, partial } => {
                tracing::debug!(
                    method = method.as_str(),
                    partial,
                    remaining = %describe(&remaining),
                    "ssh auth method rejected"
                );
                allowed = Some(remaining);
            }
        }
    }

    Err(SshError::AuthenticationFailed {
        target: options.target.clone(),
        remaining: allowed.as_ref().map(describe).unwrap_or_default(),
    })
}

/// 把 `AuthResult` 折成我们的 [`Step`]。
fn classify(result: AuthResult) -> Step {
    match result {
        AuthResult::Success => Step::Accepted,
        AuthResult::Failure {
            remaining_methods,
            partial_success,
        } => Step::Rejected {
            remaining: remaining_methods,
            partial: partial_success,
        },
    }
}

/// 把服务端给的剩余方法拼成人读的一串（错误消息与日志字段都用它）。
fn describe(methods: &MethodSet) -> String {
    if methods.is_empty() {
        return "无".to_owned();
    }
    methods
        .iter()
        .map(<&'static str>::from)
        .collect::<Vec<_>>()
        .join(",")
}

/// RSA 的签名哈希：能问就问服务端（`server-sig-algs`），问不到就交给上游的默认。
///
/// 为什么值得多这一步：`PrivateKeyWithHashAlg::new` 在 `None` 时对 RSA 用**遗留的
/// `ssh-rsa`（SHA-1）**，而如今的 OpenSSH 默认拒绝它 —— 不问就会得到
/// "明明有这把钥匙却认证失败"，而那是最难查的一类。
async fn rsa_hash(session: &Handle<Handler>, key: &PublicKey) -> Option<russh::keys::HashAlg> {
    if !key.algorithm().is_rsa() {
        return None;
    }
    session
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten()
}

// ── agent ────────────────────────────────────────────────────────────────────

/// 第一档：SSH agent。
///
/// 它不可用的每一种情形（没设 `SSH_AUTH_SOCK`、socket 不在、agent 里一把钥匙都没有）
/// 都只是 [`Step::Unavailable`] —— **静默落到下一档**，不是错误。
#[cfg(unix)]
async fn agent_step(
    session: &mut Handle<Handler>,
    options: &mut SshConnect,
) -> Result<Step, SshError> {
    use russh::keys::agent::client::AgentClient;

    if !options.auth.use_agent {
        return Ok(Step::Unavailable);
    }

    let connected = match &options.auth.agent_socket {
        Some(path) => AgentClient::connect_uds(path).await,
        None => AgentClient::connect_env().await,
    };
    let mut agent = match connected {
        Ok(agent) => agent,
        Err(err) => {
            tracing::debug!(%err, "ssh agent unavailable");
            return Ok(Step::Unavailable);
        }
    };
    let identities = match agent.request_identities().await {
        Ok(identities) => identities,
        Err(err) => {
            tracing::debug!(%err, "ssh agent failed");
            return Ok(Step::Unavailable);
        }
    };
    if identities.is_empty() {
        tracing::debug!("ssh agent has no identity");
        return Ok(Step::Unavailable);
    }

    let mut last = Step::Unavailable;
    for identity in &identities {
        let public = identity.public_key().into_owned();
        let hash = rsa_hash(session, &public).await;
        let result = session
            .authenticate_publickey_with(
                options.target.user(),
                public,
                hash,
                // agent 自己签名：私钥**不进我们进程**（D7 选它当第一档的理由）。
                &mut agent,
            )
            .await
            .map_err(|err| connect_failed(&options.target, err))?;
        match classify(result) {
            Step::Accepted => return Ok(Step::Accepted),
            other => last = other,
        }
    }
    Ok(last)
}

/// Windows：agent 是 Pageant 的命名管道，**本 plan 不做**（如实记为未实现，
/// 而不是留一个"看起来支持"的空实现）。`fallthrough` 到密钥池与口令。
#[cfg(not(unix))]
async fn agent_step(
    _session: &mut Handle<Handler>,
    _options: &mut SshConnect,
) -> Result<Step, SshError> {
    Ok(Step::Unavailable)
}

// ── 密钥池 ───────────────────────────────────────────────────────────────────

/// 第二档：密钥池给的候选私钥，**按调用方给的顺序**。
///
/// 解析不出、解不开、或服务端不认的钥匙都只是"这一把不行"：继续下一把，
/// 全都不行就走后面的档。**只有基础设施的失败（受保护页、要不到口令）才返回 `Err`** ——
/// 一把坏钥匙不该把"其实还能用口令登录"这件事挡掉。
async fn key_step(
    session: &mut Handle<Handler>,
    options: &mut SshConnect,
) -> Result<Step, SshError> {
    if options.auth.keys.is_empty() {
        return Ok(Step::Unavailable);
    }

    let mut last = Step::Unavailable;
    // 先取用户名，避免在循环里同时借 `options.target` 与 `options.auth`。
    let user = options.target.user().to_owned();
    for index in 0..options.auth.keys.len() {
        let Some(key) = decrypt_candidate(options, index).await? else {
            continue;
        };
        let public = key.public_key().clone();
        let hash = rsa_hash(session, &public).await;
        let result = session
            .authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
            .map_err(|err| connect_failed(&options.target, err))?;
        match classify(result) {
            Step::Accepted => return Ok(Step::Accepted),
            other => last = other,
        }
    }
    Ok(last)
}

/// 把第 `index` 把候选私钥变成一把可用的 `PrivateKey`。
///
/// 未加密的直接解析；加密的问口令（命中缓存就不问），**解不开就把缓存里那条忘掉
/// 再问一次**（D8 失效条件 ②的密钥口令版：留着错口令只会每次都失败）。
/// 两次都解不开 = 这把钥匙用不了 → `Ok(None)`，由调用方继续下一把。
async fn decrypt_candidate(
    options: &mut SshConnect,
    index: usize,
) -> Result<Option<PrivateKey>, SshError> {
    let key_id = options.auth.keys[index].id().to_owned();
    let cache_key = CacheKey::new(
        options.target.clone(),
        CredentialKind::KeyPassphrase {
            key: key_id.clone(),
        },
    );

    // 第一轮：先试"没有口令"这条路。未加密的私钥在这里就成功了，**不必问任何人**。
    // 只有"这把钥匙被口令保护着"才值得去问 —— 一个格式坏掉的钥匙问口令也没用。
    let (parsed, needs_passphrase) = options.auth.keys[index].with_pem(|pem| {
        let parsed = decode_secret_key(pem, None);
        let needs = match &parsed {
            Ok(_) => false,
            Err(err) => is_passphrase_protected(pem, err),
        };
        (parsed, needs)
    })?;

    match parsed {
        Ok(key) => return Ok(Some(key)),
        Err(err) if !needs_passphrase => {
            tracing::warn!(key = key_id.as_str(), %err, "ssh private key unusable");
            return Ok(None);
        }
        Err(_) => {}
    }

    // 第二轮：要口令（命中缓存就不问），并**只解一次**：
    // 解不开说明缓存里那句是错的（或者用户敲错了）—— 忘掉它，下一次连接重新问。
    let credential = options
        .cache
        .resolve(cache_key.clone(), options.provider.as_ref())?;
    let mut guard = lock(&credential);
    let decrypted = options.auth.keys[index]
        .with_pem(|pem| guard.with_str(|passphrase| decode_secret_key(pem, Some(passphrase))))??;

    match decrypted {
        Ok(key) => Ok(Some(key)),
        Err(err) => {
            options.cache.forget(&cache_key);
            tracing::warn!(key = key_id.as_str(), %err, "ssh private key passphrase rejected");
            Ok(None)
        }
    }
}

/// 这把私钥需要口令吗。**两条判据都是格式事实**，不是从错误消息里猜：
///
/// 1. PKCS#8 的加密有**专用头**（`-----BEGIN ENCRYPTED PRIVATE KEY-----`）——
///    没有密码时上游会把它当成一个解不开的 DER 报错，从错误类型分不出来；
/// 2. OpenSSH 新格式把加密写在 base64 body 的 `ciphername` 字段里，那一段看不了明文，
///    但上游解析器知道：它对"加密且没给口令"返回的正是 [`Error::KeyIsEncrypted`]。
fn is_passphrase_protected(pem: &str, err: &Error) -> bool {
    pem.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----") || matches!(err, Error::KeyIsEncrypted)
}

// ── keyboard-interactive ─────────────────────────────────────────────────────

/// 第三档：keyboard-interactive（2FA / OTP 的**唯一**入口，D7 把它排在 password 之前）。
///
/// ⚠️ 只在一个服务端回合**只问一句**时用缓存：多个 prompt 是"用户名 + 口令"这类组合，
/// 拿同一句回答去填所有格子必然是错的。
async fn keyboard_step(
    session: &mut Handle<Handler>,
    options: &mut SshConnect,
) -> Result<Step, SshError> {
    let user = options.target.user().to_owned();
    let mut response = session
        .authenticate_keyboard_interactive_start(&user, None::<String>)
        .await
        .map_err(|err| connect_failed(&options.target, err))?;

    for _ in 0..MAX_KEYBOARD_ROUNDS {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(Step::Accepted),
            KeyboardInteractiveAuthResponse::Failure {
                remaining_methods,
                partial_success,
            } => {
                return Ok(Step::Rejected {
                    remaining: remaining_methods,
                    partial: partial_success,
                });
            }
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let cacheable = prompts.len() == 1;
                let cache_key =
                    CacheKey::new(options.target.clone(), CredentialKind::KeyboardInteractive);
                let mut answers = Vec::with_capacity(prompts.len());
                for _ in &prompts {
                    let credential = if cacheable {
                        options
                            .cache
                            .resolve(cache_key.clone(), options.provider.as_ref())?
                    } else {
                        let request = CredentialRequest::new(
                            &options.target,
                            CredentialKind::KeyboardInteractive,
                        );
                        Arc::new(std::sync::Mutex::new(options.provider.request(&request)?))
                    };
                    answers.push(lock(&credential).expose_string()?);
                }
                response = session
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(|err| connect_failed(&options.target, err))?;
            }
        }
    }

    // 服务端一直要答案：**不无限陪着**（这条路径是长驻任务，挂住就等于会话永远不结束）。
    Err(SshError::CredentialUnavailable(format!(
        "keyboard-interactive 超过 {MAX_KEYBOARD_ROUNDS} 个来回"
    )))
}

// ── password ─────────────────────────────────────────────────────────────────

/// 第四档：登录口令。
async fn password_step(
    session: &mut Handle<Handler>,
    options: &mut SshConnect,
) -> Result<Step, SshError> {
    let cache_key = CacheKey::new(options.target.clone(), CredentialKind::LoginPassword);
    let credential = options
        .cache
        .resolve(cache_key.clone(), options.provider.as_ref())?;
    // ⚠️ 这一份 `String` 是普通堆上的明文，drop 时**不擦**：`russh` 的接口只收
    // `Into<String>`（D8 照实记的边界）。
    let password = lock(&credential).expose_string()?;

    let result = session
        .authenticate_password(options.target.user(), password)
        .await
        .map_err(|err| connect_failed(&options.target, err))?;

    match classify(result) {
        Step::Accepted => Ok(Step::Accepted),
        // D8 失效条件 ②：**服务端拒绝就删掉那一条**，否则下次连接会拿错口令反复
        // 重试 —— 那正是账号锁定的成因。⚠️ 但"这一档过了、还要再来一种"不算拒绝：
        // 那时口令是对的，删了就会在 2FA 场景里重新弹问。
        Step::Rejected {
            remaining,
            partial: false,
        } => {
            options.cache.forget(&cache_key);
            Ok(Step::Rejected {
                remaining,
                partial: false,
            })
        }
        other => Ok(other),
    }
}

/// 取凭据的锁。中毒时继续用：这是一句话的值，不是需要保持一致性的结构。
fn lock(credential: &Arc<std::sync::Mutex<Credential>>) -> std::sync::MutexGuard<'_, Credential> {
    credential
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn the_order_is_the_decision() {
        // D7 的顺序本身就是判据的一部分（"agent 优先"、"keyboard-interactive 在
        // password 之前"）。写成常量 + 这条断言，比散在 `if` 里可核对得多。
        assert_eq!(
            Method::ORDER,
            [
                Method::Agent,
                Method::KeyPool,
                Method::KeyboardInteractive,
                Method::Password
            ]
        );
    }

    #[test]
    fn the_pool_speaks_for_publickey_on_the_wire() {
        // agent 与密钥池在**线协议上都是 `publickey`**：服务端给的 `remaining_methods`
        // 只说得出后者，所以对账必须用 `protocol()` 而不是 `as_str()`。
        assert_eq!(Method::Agent.protocol(), MethodKind::PublicKey);
        assert_eq!(Method::KeyPool.protocol(), MethodKind::PublicKey);
        assert_ne!(Method::KeyboardInteractive.protocol(), MethodKind::Password);
    }

    #[test]
    fn describe_is_readable_and_never_empty() {
        assert_eq!(describe(&MethodSet::empty()), "无");
        let mut set = MethodSet::empty();
        set.push(MethodKind::Password);
        set.push(MethodKind::KeyboardInteractive);
        assert_eq!(describe(&set), "password,keyboard-interactive");
    }
}
