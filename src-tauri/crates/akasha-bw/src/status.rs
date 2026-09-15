//! `bw status --raw` 的形状（实测 + 官方文档，`docs/bitwarden.md` §7.1）。
//!
//! 界面上的 `unauthenticated` / `locked` / `unlocked` 三态**只能来自这里**，不由我们内存里
//! 的状态推断（ADR-0007 D10）：外部用 `bw lock` 锁过之后，我们手里的 key 已经失效，
//! 而"我们以为还解锁着"只会让下一条命令以 `You are not logged in.` 失败。
//!
//! ⚠️ 未登录时上游**只给三个键**（实测：`{"serverUrl":null,"lastSync":null,"status":"unauthenticated"}`），
//! `userEmail` / `userId` 要等登录之后才出现（官方文档的完整例子）。两个键因此都是可选。
//!
//! ⚠️ `status` 的取值我们不认识时**报错而不取默认值**：把未来的第四个取值当成 `locked`
//! 会让界面显示一个错的、且看起来正常的状态。

use serde::Deserialize;

use crate::error::BwError;

/// vault 的三态。取值名与上游一致（它出现在 JSON 里）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// 没有登录（此时 `user_email` / `user_id` 必然为空）。
    Unauthenticated,
    /// 登录了，但没有 session key。
    Locked,
    /// 登录了且解锁。
    Unlocked,
}

impl State {
    /// 界面上说的那一句。
    pub fn describe(self) -> &'static str {
        match self {
            Self::Unauthenticated => "未登录",
            Self::Locked => "已登录，未解锁",
            Self::Unlocked => "已解锁",
        }
    }

    /// 这个状态下我们**可以**持有 session key 吗。
    ///
    /// `false` 的两种状态都必须让我们立刻丢掉手上的 key（ADR-0007 D10）。
    pub fn allows_session(self) -> bool {
        matches!(self, Self::Unlocked)
    }
}

/// `bw status --raw` 解析出来的东西。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// 当前配置的服务器地址；上游默认的官方云在未登录时也给得出（不是 `null` 才算数）。
    pub server_url: Option<String>,
    /// 上次同步的时间（ISO 8601）。
    pub last_sync: Option<String>,
    pub user_email: Option<String>,
    pub user_id: Option<String>,
    pub state: State,
}

/// 文件里的样子。与 [`Status`] 分开：这里允许字段缺失，那里永远有值。
#[derive(Debug, Deserialize)]
struct Raw {
    #[serde(rename = "serverUrl")]
    server_url: Option<String>,
    #[serde(rename = "lastSync")]
    last_sync: Option<String>,
    #[serde(rename = "userEmail")]
    user_email: Option<String>,
    #[serde(rename = "userId")]
    user_id: Option<String>,
    status: String,
}

/// 解析 `bw status --raw` 的输出。
pub fn parse(raw: &str) -> Result<Status, BwError> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(BwError::Parse {
            what: "status".to_owned(),
            message: "输出是空的".to_owned(),
        });
    }
    let parsed: Raw = serde_json::from_str(text).map_err(|err| BwError::Parse {
        what: "status".to_owned(),
        message: format!("{err}；原文：{}", excerpt(text)),
    })?;
    let state = match parsed.status.as_str() {
        "unauthenticated" => State::Unauthenticated,
        "locked" => State::Locked,
        "unlocked" => State::Unlocked,
        other => {
            return Err(BwError::Parse {
                what: "status".to_owned(),
                message: format!("不认识的 status 取值 {other:?}"),
            });
        }
    };
    Ok(Status {
        server_url: parsed.server_url,
        last_sync: parsed.last_sync,
        user_email: parsed.user_email,
        user_id: parsed.user_id,
        state,
    })
}

/// 出错时别把整段输出贴出来（它可能有几百字节），但也不能只剩一句"解析失败"。
fn excerpt(text: &str) -> String {
    const MAX: usize = 120;
    if text.len() <= MAX {
        return text.to_owned();
    }
    let mut end = MAX;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    /// 实测：未登录时上游只给三个键。
    #[test]
    fn the_unauthenticated_shape_has_only_three_keys() {
        let status = parse(r#"{"serverUrl":null,"lastSync":null,"status":"unauthenticated"}"#)
            .expect("实测过的形状");
        assert_eq!(status.state, State::Unauthenticated);
        assert_eq!(status.user_email, None);
        assert_eq!(status.user_id, None);
        assert_eq!(status.server_url, None);
    }

    /// 官方文档给出的完整形状（登录之后）。
    #[test]
    fn the_documented_shape_is_read_whole() {
        let status = parse(
            r#"{
              "serverUrl": "https://bitwarden.example.com",
              "lastSync": "2020-06-16T06:33:51.419Z",
              "userEmail": "user@example.com",
              "userId": "00000000-0000-0000-0000-000000000000",
              "status": "unlocked"
            }"#,
        )
        .expect("文档里的形状");
        assert_eq!(status.state, State::Unlocked);
        assert_eq!(
            status.server_url.as_deref(),
            Some("https://bitwarden.example.com")
        );
        assert_eq!(status.user_email.as_deref(), Some("user@example.com"));
        assert_eq!(
            status.last_sync.as_deref(),
            Some("2020-06-16T06:33:51.419Z")
        );
        assert!(status.state.allows_session());
    }

    #[test]
    fn locked_is_its_own_state_and_forbids_a_session() {
        let status = parse(r#"{"serverUrl":"https://vault.example.com","status":"locked"}"#)
            .expect("登录但没解锁");
        assert_eq!(status.state, State::Locked);
        assert!(!status.state.allows_session());
        assert!(!State::Unauthenticated.allows_session());
    }

    /// 诱饵：**第四个取值不许被当成三态之一**。
    #[test]
    fn an_unknown_state_is_refused() {
        let err = parse(r#"{"status":"expired"}"#).expect_err("不认识的取值要报错");
        assert!(matches!(err, BwError::Parse { .. }), "{err:?}");
        assert!(err.to_string().contains("expired"), "{err}");
    }

    #[test]
    fn an_empty_output_is_reported_as_such() {
        assert!(matches!(parse("  \n"), Err(BwError::Parse { .. })));
        assert!(matches!(
            parse("You are not logged in."),
            Err(BwError::Parse { .. })
        ));
    }

    #[test]
    fn a_broken_shape_keeps_an_excerpt_of_the_original() {
        let long = format!("{{\"status\": \"{}", "x".repeat(500));
        let err = parse(&long).expect_err("半截 JSON");
        let text = err.to_string();
        assert!(text.contains('…'), "截断要看得出来：{text}");
        assert!(text.len() < 400, "不许把整段贴出来：{}", text.len());
    }
}
