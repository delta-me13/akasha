//! 变体判定：手里的这一份 `bw` 是 OSS 还是专有（ADR-0007 D5，`docs/bitwarden.md` §2.2）。
//!
//! ## 为什么需要它
//!
//! 专有变体的许可证（2.1）把用途限制在**内部开发与内部测试、非生产环境**。因此
//! `host` 那一轴探到专有变体时，界面必须说一句 —— 这是 `scope.md` §7 里
//! "不得让用户在生产环境中使用专有变体而无任何提示"的落点。
//!
//! ## 判据是实测出来的
//!
//! 两份 `cli-v2026.8.0` 资产的对比（`docs/bitwarden.md` §2.2）：
//! `bw --version` **都是** `2026.8.0`，`bw --help` 的命令表里**只有专有那一份**有
//! `device-approval` 一行（官方文档也写明 device approval 是 OSS 版缺的功能）。
//!
//! ⚠️ 它是**启发式**，且判不出来时必须说"判不出来"（[`Variant::Unknown`]）：
//! 默认当 OSS 就等于把许可证提示悄悄关掉。

/// 手里的 CLI 是哪一份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Variant {
    /// `bw-oss-*`：GPL-3.0-only，无用途限制。
    Oss,
    /// `bw`：Bitwarden License Agreement，2.1 限制用途、2.3(i) 禁止分发。
    Proprietary,
    /// 命令表里判不出来（上游改了命令名，或者这份输出根本不是 `--help`）。
    Unknown,
}

impl Variant {
    /// 界面上说的那一句。
    pub fn describe(self) -> &'static str {
        match self {
            Self::Oss => "OSS 变体（GPL-3.0-only）",
            Self::Proprietary => "专有变体（许可限制在生产环境使用）",
            Self::Unknown => "判不出变体",
        }
    }

    /// 要不要提示许可证（只有专有那一份要）。
    pub fn needs_license_notice(self) -> bool {
        matches!(self, Self::Proprietary)
    }
}

/// 命令表里专有变体独有的一行。
const PROPRIETARY_MARKER: &str = "device-approval";

/// 从 `bw --help` 的输出判定变体。**纯函数**，因此判据本身可以单测。
pub fn from_help(help: &str) -> Variant {
    let mut lines = help.lines();
    let Some(_) = lines.find(|line| line.trim() == "Commands:") else {
        return Variant::Unknown;
    };
    let mut commands = 0_usize;
    for line in lines {
        let trimmed = line.trim();
        // 命令表到空行 / 下一段为止。缩进后的示例在 `Examples:` 里，不在这一段。
        if trimmed.is_empty() {
            break;
        }
        commands += 1;
        if trimmed.split_whitespace().next() == Some(PROPRIETARY_MARKER) {
            return Variant::Proprietary;
        }
    }
    // 命令表一行都没有 = 这份输出不是我们认得的命令表（空的 `--help` 输出会被上游的
    // 包装脚本吞掉）—— 报"判不出"，不报"OSS"。
    if commands == 0 {
        return Variant::Unknown;
    }
    Variant::Oss
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    /// 实测输出（`cli-v2026.8.0`，OSS 那一份）—— 只留命令表的头与尾。
    const OSS_HELP: &str = "\
Usage: bw [options] [command]

Options:
  --pretty                                    Format output. JSON is tabbed with two spaces.

Commands:
  sdk-version                                 Print the SDK version.
  login [options] [email] [password]          Log into a user account.
  lock                                        Lock the vault and destroy active session keys.
  status                                      Show server, last sync, user information, and vault status.
  serve [options]                             Start a RESTful API webserver.
  help [command]                              display help for command

  Tip: Managing and retrieving secrets for dev environments is easier with Bitwarden Secrets Manager.
";

    /// 实测输出（`cli-v2026.8.0`，专有那一份）—— 与上面只差 `device-approval` 一行。
    const PROPRIETARY_HELP: &str = "\
Usage: bw [options] [command]

Options:
  --pretty                                    Format output. JSON is tabbed with two spaces.

Commands:
  sdk-version                                 Print the SDK version.
  login [options] [email] [password]          Log into a user account.
  status                                      Show server, last sync, user information, and vault status.
  device-approval                             Manage device approval requests sent to organizations that use SSO with trusted devices.
  serve [options]                             Start a RESTful API webserver.
";

    #[test]
    fn the_two_variants_are_told_apart() {
        assert_eq!(from_help(OSS_HELP), Variant::Oss);
        assert_eq!(from_help(PROPRIETARY_HELP), Variant::Proprietary);
    }

    /// 诱饵：`device-approval` 只出现在**其它段落**里时不算专有。
    /// 少了这一条，"命中"与"读错了段落"分不开。
    #[test]
    fn the_marker_outside_the_command_table_does_not_count() {
        let help = "\
Usage: bw [options] [command]

Options:
  --device-approval    not a command

Commands:
  login [options] [email] [password]          Log into a user account.

  Examples:

    bw device-approval list
";
        assert_eq!(from_help(help), Variant::Oss);
    }

    #[test]
    fn nothing_recognisable_is_unknown_not_oss() {
        assert_eq!(from_help(""), Variant::Unknown);
        assert_eq!(from_help("command not found\n"), Variant::Unknown);
        // 命令表在，但一行都没有 —— 这仍是"判不出"，不是 OSS。
        assert_eq!(from_help("Commands:\n"), Variant::Unknown);
    }

    #[test]
    fn only_the_proprietary_variant_asks_for_a_license_notice() {
        assert!(Variant::Proprietary.needs_license_notice());
        assert!(!Variant::Oss.needs_license_notice());
        assert!(!Variant::Unknown.needs_license_notice());
    }
}
