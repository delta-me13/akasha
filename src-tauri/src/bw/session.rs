//! session key：**只在内存里，且住在受保护页**（ADR-0007 D7）。
//!
//! ## 它与库口令是两种机密
//!
//! | | 库口令（[`crate::store::Passphrase`]） | session key（本模块） |
//! |---|---|---|
//! | 要交出去吗 | 否 —— 只送进 SQLCipher 的 C API | **是** —— 每次带 session 的命令都要原样交给 `bw` 子进程 |
//! | 出口 | `pub(crate) expose` | [`Session::expose`]：给一个提权窗口，用完即关 |
//!
//! 因此这里用的是 `crate::store::protected` 的**同一个原语**（`Protected<N>`，ADR-0002 D13），
//! 而不是自己再包一层 `mlock` —— 两处各写一份必然漂移（该模块的文档写了理由）。
//!
//! ## 落盘这件事
//!
//! **不落盘**。`bw` 自己会把 access token 写进它的 `data.json`（那是上游的状态，ADR-0007 D6），
//! 但本模块手里的这个 key 不进文件、不进日志、不进事件载荷 —— 它只活在进程内存里，
//! [`crate::bw::Cli::lock`] / `logout` 与进程退出都会让它消失。
//!
//! ## 长度上限
//!
//! [`TOKEN_MAX`] = 256 字节（一页受保护内存装得下，实际取值远小于它：文档例子里的
//! session key 是 44 个字符）。**超长是错误而不是截断**：截断会让"token 无效"
//! 变成一件没人能解释的事，与 `protected.rs` 对私钥、口令的口径一致。

use crate::store::protected::{Exposed, PageError, Protected};

use crate::bw::error::BwError;

/// session key 的上限（字节）。
pub const TOKEN_MAX: usize = 256;

/// 一个活着的 session key。
///
/// **刻意不实现 `Debug` / `Display`**：前者会让 `{:?}` 变成一处泄漏点，
/// 后者会被日志的 `{}` 直接吃掉（`memsafe::Secret` 本身也是这个口径）。
pub struct Session {
    token: Protected<TOKEN_MAX>,
}

impl Session {
    /// 从 `bw --raw` 的输出搬进来。
    ///
    /// 参数按**值**收 `Vec<u8>`：`Protected::new` 成功时会把源缓冲擦零，
    /// 于是那份普通堆内存不会留着明文（先转成 `String` 就会多留一份 —— `cli.rs` 的
    /// 模块文档写了为什么输出保持字节）。
    pub fn from_output(mut bytes: Vec<u8>) -> Result<Self, BwError> {
        // `--raw` 的输出带一个换行；尾部空白不是 key 的一部分。
        while bytes.last().is_some_and(u8::is_ascii_whitespace) {
            bytes.pop();
        }
        if bytes.is_empty() {
            return Err(BwError::EmptyToken);
        }
        let found = bytes.len();
        let token = Protected::new(bytes).map_err(|err| match err {
            // 长度在构造之前就知道，所以这里带得上 `found`（`PageError` 只回上限）。
            PageError::TooLong { max } => BwError::TokenTooLong { found, max },
            PageError::Memory(inner) => BwError::ProtectedPage {
                message: inner.to_string(),
            },
        })?;
        Ok(Self { token })
    }

    /// 这个 key 有几字节（不是那一页的大小）。**测试与诊断用**，不是机密。
    pub fn byte_len(&self) -> usize {
        self.token.byte_len()
    }

    /// 取一个**提权窗口**。返回的守卫一 drop 权限就交还 OS，所以不要把它存起来。
    pub fn expose(&mut self) -> Result<Exposed<'_, TOKEN_MAX>, BwError> {
        // 长度先读出来：`expose` 借的是 `&mut self`，闭包里再读一次 `self` 会同时要两个借用。
        let found = self.token.byte_len();
        self.token.expose().map_err(|err| match err {
            PageError::TooLong { max } => BwError::TokenTooLong { found, max },
            PageError::Memory(inner) => BwError::ProtectedPage {
                message: inner.to_string(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    #[test]
    fn the_key_comes_back_byte_for_byte_and_the_trailing_newline_is_dropped() {
        let raw = b"5PBYGU+5yt3RHcCjoeJKx/wByU34vokGRZjXpSH7Ylo8w==\n";
        let mut session = Session::from_output(raw.to_vec()).unwrap();
        assert_eq!(
            &*session.expose().unwrap(),
            b"5PBYGU+5yt3RHcCjoeJKx/wByU34vokGRZjXpSH7Ylo8w==",
            "换行不是 key 的一部分"
        );
        assert_eq!(session.byte_len(), 47);
    }

    /// 空输出（`bw` 没给 key）必须是错误，不许造出一个空的 session。
    #[test]
    fn an_empty_output_is_refused() {
        assert!(matches!(
            Session::from_output(Vec::new()),
            Err(BwError::EmptyToken)
        ));
        assert!(matches!(
            Session::from_output(b"\n\n".to_vec()),
            Err(BwError::EmptyToken)
        ));
    }

    /// 超长是错误，且错误里带着**实际长度**。
    #[test]
    fn an_over_long_key_is_an_error_with_its_actual_length() {
        let too_long = vec![b'a'; TOKEN_MAX + 1];
        match Session::from_output(too_long) {
            Err(BwError::TokenTooLong { found, max }) => {
                assert_eq!(found, TOKEN_MAX + 1);
                assert_eq!(max, TOKEN_MAX);
            }
            other => panic!("超长要报出来：{:?}", other.err().map(|e| e.to_string())),
        }
    }

    /// 正对照：刚好等于上限是**通过**的 —— 少了它，"超长被拒"与"什么都拒"分不开。
    #[test]
    fn exactly_the_limit_still_passes() {
        let session = Session::from_output(vec![b'a'; TOKEN_MAX]);
        assert!(session.is_ok());
    }
}
