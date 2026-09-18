//! `bw list items --raw` 的解析（plan 0903）：**只留下 `type = 5` 的 SSH key 条目**。
//!
//! ## 为什么是纯函数
//!
//! 输入是字节、输出是几条条目 —— 不读盘、不起进程、不碰库。于是"字段名长什么样、
//! 认不出来的形状怎么办"这些判断能在没有 app、没有 `bw`、没有 vault 的地方被钉死。
//!
//! ## ⚠️ 输入里是**整个 vault 的明文**
//!
//! 上游的接口没有"只列 SSH key"这条路（`bw list items` 的选项只有 folder / collection /
//! organization / search / trash / archived），所以每一处登录口令都会从 [`parse`] 的入参里
//! 经过一趟。这里的三条边界：
//!
//! 1. **不认识的字段不进内存**：只有下面两个 `Raw*` 结构体里列出的字段会被反序列化，
//!    别的一律丢掉（serde 默认行为）—— 我们没有"先解析成 `serde_json::Value` 再挑"那条路。
//! 2. **错误消息不带原文**：形状不对时报的是 serde 的位置信息（第几行第几列），
//!    不是那段字节（[`BwError::Parse`] 的 `message` 会进界面与日志）。
//! 3. 那段字节本身由调用方擦零（[`crate::bw::Cli::items`] 返回 `Zeroizing<Vec<u8>>`）。
//!
//! ## 字段名的两版（实测 + 上游实现）
//!
//! 上游自己的导出模型里那个字段叫 **`keyFingerprint`**（`SshKeyExport` 的
//! `toJson` 形状），而 SDK 与官方文档里叫 **`fingerprint`**（`SshKeyView.fromSdkSshKeyView`
//! 做的正是 `view.keyFingerprint = obj.fingerprint`）。两版都认 —— 认不出来的后果是
//! "指纹是空的"，而 plan 0904 会拿它做离线自检，那种空值看起来像"缓存坏了"。

use serde::Deserialize;

use crate::bw::error::BwError;

/// 上游的条目类型编号：`CipherType.SshKey = 5`（1 登录 / 2 安全笔记 / 3 卡 / 4 身份 / 5 SSH 密钥）。
const TYPE_SSH_KEY: u8 = 5;

/// 一次列举的产物。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    /// 上游一共给了多少条（**全部类型**）—— 报告里要能说"看了 N 条，其中 M 条是 SSH key"。
    pub total: usize,
    /// 其中 `type = 5` 的那些。
    pub ssh_keys: Vec<SshKeyItem>,
}

/// 一条 SSH key 条目。字段都是**上游的原样文本**：我们不解释 UUID、不解析时间戳。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SshKeyItem {
    /// 上游条目的 id（来历表的主键，plan 0904 的刷新判据按它比对）。
    pub id: String,
    pub name: String,
    /// `revisionDate`（ISO 8601 文本，原样）。缺字段时是空串 —— 那一档由调用方拒绝，
    /// 因为"没有时间戳"意味着刷新判据对它无效。
    pub revision_date: String,
    /// `sshKey.privateKey`（OpenSSH 或 PEM 文本）。**可能为空**：上游允许只有公钥的条目，
    /// 而命令行把它交出来时 `privateKey` 就是空串 —— 那一档由调用方跳过并说明。
    pub private_key: String,
    pub public_key: String,
    /// `fingerprint` 或 `keyFingerprint`（见模块文档）。
    pub fingerprint: String,
}

impl SshKeyItem {
    /// 私钥是空的 —— 导不进来（`keys::PrivateKey::new` 会拒绝）。
    ///
    /// 单独一个方法而不是让调用方各写一遍 `is_empty`：这条判断与"为什么跳过它"那句话
    /// 在报告里是同一件事。
    pub fn has_private_key(&self) -> bool {
        !self.private_key.trim().is_empty()
    }

    /// 来历能记下来吗：没有 id 或没有 `revisionDate` 的条目导进来会成为一条
    /// **以后认不出来**的记录（刷新判据找不到它、去重也做不到）。
    pub fn is_recordable(&self) -> bool {
        !self.id.trim().is_empty()
            && !self.revision_date.trim().is_empty()
            && !self.name.trim().is_empty()
    }
}

/// `bw list items --raw` 的字节 → 有条目。
///
/// 形状不对（不是数组、`type` 不是数字……）→ [`BwError::Parse`]，消息里只有 serde 的
/// 位置信息。**空的数组是合法输入**（一个空 vault）。
pub fn parse(bytes: &[u8]) -> Result<Inventory, BwError> {
    let ciphers: Vec<RawCipher> = serde_json::from_slice(bytes).map_err(|err| BwError::Parse {
        what: "`bw list items --raw` 的输出".to_owned(),
        // ⚠️ 只带 serde 的位置信息：这段字节是整个 vault 的明文，绝不能进消息。
        message: err.to_string(),
    })?;

    let total = ciphers.len();
    let ssh_keys = ciphers
        .into_iter()
        .filter(|cipher| cipher.kind == TYPE_SSH_KEY)
        .map(|cipher| {
            let key = cipher.ssh_key.unwrap_or_default();
            SshKeyItem {
                id: cipher.id,
                name: cipher.name,
                revision_date: cipher.revision_date.unwrap_or_default(),
                private_key: key.private_key,
                public_key: key.public_key,
                fingerprint: key.fingerprint,
            }
        })
        .collect();

    Ok(Inventory { total, ssh_keys })
}

/// `bw` 交出来的那一条（**只有这里列出的字段会被读进内存**）。
#[derive(Debug, Deserialize)]
struct RawCipher {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    /// ⚠️ 这个字段**没有默认值**：`type` 缺失意味着这不是上游给出的形状（`bw` 每条都有），
    /// 而默认成 0 会让"所有条目都不是 SSH key"变成一件静默的事。
    #[serde(rename = "type")]
    kind: u8,
    #[serde(default, rename = "revisionDate")]
    revision_date: Option<String>,
    /// `type = 5` 之外它是 `null`（上游的模板就是这么给的）。
    #[serde(default, rename = "sshKey")]
    ssh_key: Option<RawSshKey>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSshKey {
    #[serde(default, rename = "privateKey")]
    private_key: String,
    #[serde(default, rename = "publicKey")]
    public_key: String,
    #[serde(default, rename = "fingerprint", alias = "keyFingerprint")]
    fingerprint: String,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    /// 两个条目：一条 SSH key（`fingerprint` 那版字段名）+ 一条登录（**必须一条都不进**）。
    const MIXED: &[u8] = br#"[
      {
        "id": "11111111-1111-1111-1111-111111111111",
        "type": 1,
        "name": "example.com",
        "login": { "username": "me", "password": "hunter2" },
        "revisionDate": "2026-09-01T00:00:00.000Z"
      },
      {
        "id": "22222222-2222-2222-2222-222222222222",
        "type": 5,
        "name": "id_ed25519",
        "notes": null,
        "revisionDate": "2026-09-02T03:04:05.000Z",
        "sshKey": {
          "privateKey": "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n",
          "publicKey": "ssh-ed25519 AAAA me",
          "fingerprint": "SHA256:abcdef"
        }
      }
    ]"#;

    #[test]
    fn only_ssh_key_items_are_kept() {
        let found = parse(MIXED).unwrap();
        assert_eq!(found.total, 2, "报告要能说「看了几条」");
        assert_eq!(found.ssh_keys.len(), 1);
        let key = &found.ssh_keys[0];
        assert_eq!(key.name, "id_ed25519");
        assert_eq!(key.revision_date, "2026-09-02T03:04:05.000Z");
        assert_eq!(key.fingerprint, "SHA256:abcdef");
        assert!(key.has_private_key());
    }

    /// 上游**导出模型**那一版的字段名（`keyFingerprint`）也要认 —— 两版都在真机上出现过。
    #[test]
    fn both_spellings_of_the_fingerprint_are_accepted() {
        let older = br#"[{"id":"a","type":5,"name":"k",
            "revisionDate":"r","sshKey":{"privateKey":"p","publicKey":"u","keyFingerprint":"SHA256:zzz"}}]"#;
        assert_eq!(parse(older).unwrap().ssh_keys[0].fingerprint, "SHA256:zzz");
    }

    /// 只有公钥的条目：形状合法，但私钥是空的 —— 调用方要能看出这一档。
    #[test]
    fn an_item_without_a_private_key_is_visible_as_such() {
        let public_only = br#"[{"id":"a","type":5,"name":"k","revisionDate":"r",
            "sshKey":{"privateKey":"","publicKey":"ssh-ed25519 AAAA","fingerprint":"SHA256:x"}}]"#;
        let found = parse(public_only).unwrap();
        assert_eq!(found.ssh_keys.len(), 1);
        assert!(!found.ssh_keys[0].has_private_key());
    }

    /// `sshKey` 是 `null`（上游模板的形状）**不是解析错误**：它是一条导不进来的条目，
    /// 而不是一份坏掉的输出 —— 整批失败会让另外九条好的一起进不来。
    #[test]
    fn a_null_ssh_key_is_an_item_that_cannot_be_imported_not_a_broken_document() {
        let null_key = br#"[{"id":"a","type":5,"name":"k","revisionDate":"r","sshKey":null}]"#;
        let found = parse(null_key).unwrap();
        assert_eq!(found.ssh_keys.len(), 1);
        assert!(!found.ssh_keys[0].has_private_key());
        assert!(
            found.ssh_keys[0].is_recordable(),
            "id 与日期都在，来历记得下"
        );
    }

    /// 缺 `id` / `revisionDate` 的条目：能读出来，但**来历记不下来**（调用方跳过它）。
    #[test]
    fn an_item_without_provenance_is_not_recordable() {
        let bare = br#"[{"type":5,"name":"k","sshKey":{"privateKey":"p"}}]"#;
        let found = parse(bare).unwrap();
        assert!(!found.ssh_keys[0].is_recordable());
    }

    /// 形状不对：错误里**不许**出现那段字节（它是整个 vault 的明文）。
    #[test]
    fn a_broken_document_reports_a_position_not_the_bytes() {
        let err = parse(br#"{"items": [], "secret": "hunter2"}"#).unwrap_err();
        match err {
            BwError::Parse { what, message } => {
                assert!(what.contains("list items"), "{what}");
                assert!(!message.contains("hunter2"), "错误里漏了明文：{message}");
            }
            other => panic!("应当报形状问题：{other:?}"),
        }
    }

    /// 空 vault 是合法输入（不是错误，也不是"读不出来"）。
    #[test]
    fn an_empty_array_is_a_legal_answer() {
        let found = parse(b"[]").unwrap();
        assert_eq!(found.total, 0);
        assert!(found.ssh_keys.is_empty());
    }

    /// 缺 `type` 的条目直接报错：默认成某个数字会让"全都没有 SSH key"变成静默的事。
    #[test]
    fn a_missing_type_is_refused_rather_than_defaulted() {
        assert!(parse(br#"[{"id":"a","name":"k"}]"#).is_err());
    }
}
