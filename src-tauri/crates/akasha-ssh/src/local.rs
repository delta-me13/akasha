//! **本机端点**（plan 0702）—— `scope.md` §4 那张表里 "local ↔ host" 的那个 local。
//!
//! 它是 [`crate::transfer::Endpoint`] 的第二种实现（第一种是一个 SFTP 会话）。两侧共用
//! 同一个引擎、同一个落盘不变量，差别只在**谁去碰字节**：这里用 `std::fs` / `tokio::fs`。
//!
//! ## 为什么是一个无字段的类型
//!
//! 本机不需要连接、认证，也没有"会话"要持有 —— 路径一律由每一条命令带进来（当前目录
//! 记在 SFTP `Session` 的某一侧，不记在这里）。于是调用方可以**随手构造一个**，
//! 不必为它开一个实体，也就不会有"谁来回收它"这个问题。
//!
//! ## 临时名与原子重命名在这里
//!
//! ADR-0006 D4 把这件事定在**端点**这一层。本机的做法是通行的那一种：同一个目录下写
//! `.name.part`，完成后 `rename` —— 同目录的 `rename` 在 POSIX 与 Windows 上都是原子的
//! （Windows 的 `fs::rename` 覆盖已存在的目标）。
//!
//! ⚠️ **不做 `fsync`**：判据是"用户看到最终名就等于成功"（`scope.md` §4.2），
//! 而断电后文件是否还在磁盘上属于另一件事（远端那一侧同理，`fsync@openssh.com`
//! 缺失时的降级同样没做 —— 见 ADR-0006 §4）。

use std::path::PathBuf;

use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::SshError;
use crate::transfer::{
    BoxFuture, Endpoint, Entry, EntryKind, FileRead, Listing, PendingWrite, file_failed,
    temp_candidates,
};

/// 本机文件系统作为一个端点。
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalEndpoint;

impl LocalEndpoint {
    /// 空的构造（它没有字段，所以这是个常数）。
    pub const fn new() -> Self {
        Self
    }

    /// 本机那一栏的**默认起点**。
    ///
    /// 取 `HOME`（Windows 上是 `USERPROFILE`），都没有就退到当前工作目录，再没有就退到
    /// 根。三层都要有：受限环境（容器、CI）里 `HOME` 可能是空的，而"默认目录取不到"
    /// 不该让整栏用不了。
    pub fn default_dir() -> String {
        let home = if cfg!(windows) {
            std::env::var_os("USERPROFILE")
        } else {
            std::env::var_os("HOME")
        };
        let fallback = || {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" }))
        };
        tidy(home.map_or_else(fallback, PathBuf::from))
    }
}

impl Endpoint for LocalEndpoint {
    fn list<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Listing, SshError>> {
        let path = path.to_owned();
        Box::pin(async move {
            let dir = fs::canonicalize(&path)
                .await
                .map_err(|err| file_failed(&path, err))?;
            let mut names: Vec<Entry> = Vec::new();
            let mut reader = fs::read_dir(&dir)
                .await
                .map_err(|err| file_failed(&path, err))?;
            while let Some(entry) = reader
                .next_entry()
                .await
                .map_err(|err| file_failed(&path, err))?
            {
                // ⚠️ `file_type()`**不跟随符号链接**（那是 `metadata()` 的事）——
                // 界面上要区分的正是"这是一个链接"，跟随之后它就变成普通文件了。
                let kind = match entry
                    .file_type()
                    .await
                    .map_err(|err| file_failed(&path, err))?
                {
                    kind if kind.is_dir() => EntryKind::Directory,
                    kind if kind.is_file() => EntryKind::File,
                    kind if kind.is_symlink() => EntryKind::Symlink,
                    _ => EntryKind::Other,
                };
                names.push(Entry {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    kind,
                });
            }
            // 排序的理由与远端那一侧相同（`SftpClient::list`）：不排序时"同一个目录两次
            // 列出不一样"会变成四处出现的假象，而断言也失去了确定性。
            names.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(Listing {
                path: tidy(dir),
                entries: names,
            })
        })
    }

    fn open_read<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<FileRead, SshError>> {
        let path = path.to_owned();
        Box::pin(async move {
            let file = fs::File::open(&path)
                .await
                .map_err(|err| file_failed(&path, err))?;
            // 大小从**已经打开的那个句柄**上读：换一条路径去 `metadata` 会多一次可能失败
            // 的往返，还会给出"打开的是这个、量的是那个"这种说不清的状态。
            let size = file
                .metadata()
                .await
                .map_err(|err| file_failed(&path, err))?
                .len();
            Ok(FileRead {
                reader: Box::new(file),
                size,
            })
        })
    }

    fn begin_write<'a>(
        &'a self,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn PendingWrite>, SshError>> {
        let path = path.to_owned();
        Box::pin(async move {
            let target = PathBuf::from(&path);
            let name = target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .ok_or_else(|| file_failed(&path, "这个路径没有文件名，写不进一个临时名"))?;
            let dir = target.parent().map_or_else(PathBuf::new, PathBuf::from);
            let temp = pick_temp(&dir, &name, &path).await?;
            let file = fs::File::create(&temp)
                .await
                .map_err(|err| file_failed(&path, err))?;
            Ok(Box::new(LocalWrite {
                file: Some(file),
                temp,
                target,
                path,
            }) as Box<dyn PendingWrite>)
        })
    }
}

/// 在 `dir` 里挑一个还没被占用的临时名（`name` 是最终名那一部分）。
async fn pick_temp(dir: &std::path::Path, name: &str, path: &str) -> Result<PathBuf, SshError> {
    // 候选是无限的（`.name.part`、`.name.2.part`、…），但真正的冲突只会是少数几个 ——
    // 无条件遍历下去等于把"挑不出名字"变成一次死循环。
    for candidate in temp_candidates(name).take(16) {
        let full = dir.join(candidate);
        match fs::try_exists(&full).await {
            Ok(false) => return Ok(full),
            Ok(true) => {}
            Err(err) => return Err(file_failed(path, err)),
        }
    }
    Err(file_failed(path, "目标目录里连续的 16 个临时名都被占用了"))
}

/// 本机这一侧的待落盘写入。
struct LocalWrite {
    /// `Option` 是为了**在重命名 / 删除之前把句柄关掉**：Windows 上还开着的文件
    /// 既不能改名也不能删（POSIX 上无所谓，但两边走同一条路更省心）。
    file: Option<fs::File>,
    /// 临时名的完整路径。
    temp: PathBuf,
    /// 最终名的完整路径（`commit` 的把子）。
    target: PathBuf,
    /// 出错消息里那个"用户眼中的路径"（就是 `begin_write` 收到的那个）。
    path: String,
}

impl PendingWrite for LocalWrite {
    fn write_chunk<'a>(&'a mut self, chunk: &'a [u8]) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(async move {
            let file = self
                .file
                .as_mut()
                .ok_or_else(|| file_failed(&self.path, "临时文件已经关掉了"))?;
            // 失败一律归到**目标那个路径**名下：用户要去看的是目标目录（空间、权限），
            // 报一个 `.name.part` 只会让人去找一个正常情况下看不见的文件。
            file.write_all(chunk)
                .await
                .map_err(|err| file_failed(&self.path, err))
        })
    }

    fn commit<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(async move {
            close(&mut self.file, &self.path).await?;
            fs::rename(&self.temp, &self.target)
                .await
                .map_err(|err| file_failed(&self.path, err))
        })
    }

    fn abort<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SshError>> {
        Box::pin(async move {
            close(&mut self.file, &self.path).await?;
            match fs::remove_file(&self.temp).await {
                Ok(()) => Ok(()),
                // ⚠️ **幂等**：临时名已经不在（`commit` 成功之后再叫一次）不是失败。
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(file_failed(&self.path, err)),
            }
        })
    }
}

/// 关掉句柄（已经关过就是成功 —— `abort` 会被叫不止一次）。
async fn close(file: &mut Option<fs::File>, path: &str) -> Result<(), SshError> {
    if let Some(mut file) = file.take() {
        // 用户的字节必须落在盘上，而不是留在我们这边的缓冲区里：
        // `tokio::fs::File` 的写入走的是阻塞池里的 `write(2)`，所以这里要的是"排空"
        // 而不是"提交到磁盘"（判据不要求 `fsync`，见模块文档）。
        file.flush().await.map_err(|err| file_failed(path, err))?;
    }
    Ok(())
}

/// 规范化之后的路径字面量。
///
/// Windows 上 `canonicalize` 会给一个 `\\?\` 前缀 —— 那是原生路径的转义写法，
/// 不是用户认得的路径。去掉它，否则界面上那一栏的当前目录会以 `\\?\C:\Users\…` 开头，
/// 而"返回上一级"拼出来的字符串还带着它。
fn tidy(path: PathBuf) -> String {
    let text = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return rest.to_owned();
        }
    }
    text
}
