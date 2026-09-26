//! **进程内的假上游**（仅用于测试与 E2E）。
//!
//! 本模块是 `pub` 的，理由与 `crate::ssh::testing` 相同：app 的 E2E 也要用它 ——
//! 端到端那条路要验证的是"下载 → 解包 → 落盘 → 调起来"这一串**真的走通**，
//! 而不是"我们的代码以为它会走通"。指向真实 GitHub 会让判据依赖网络与上游，
//! 所以这里给一个最小的 HTTP 服务器：两个路由，一个 release 列表，一个资产。
//!
//! ⚠️ 它**不是**上游的替身：只实现我们用到的那两个 GET，别的一律 404。
//! 真实路径（真·GitHub）在 `docs/STATUS.md` 里另有一次人工实测记录。

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::bw::acquire::{Sources, asset_name, host_target};
use crate::bw::location::EXECUTABLE;

/// 假上游。`Drop` 时停掉那条接受循环。
pub struct Stub {
    base: String,
    zip: Arc<Vec<u8>>,
    binary: Vec<u8>,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Stub {
    /// 一份 release 列表（都当正式版）+ 一个资产包，包里只有一个可执行文件。
    ///
    /// `asset_version` 决定**资产名**里的版本号 —— 与 tag 列表分开是为了能造出
    /// "列表里有、资产没有"这一类反例。
    pub fn start(tags: &[&str], asset_version: &str) -> Self {
        let meta: Vec<(&str, bool, bool)> = tags.iter().map(|tag| (*tag, false, false)).collect();
        Self::start_with_meta(&meta, asset_version)
    }

    /// 只有一条 release，资产包里的条目由调用方给。
    pub fn start_with(asset_version: &str, entries: &[(&str, Vec<u8>)]) -> Self {
        let tag = format!("cli-v{asset_version}");
        Self::build(
            meta_json(&[(&tag, false, false)]),
            vec![asset_for(asset_version)],
            entries
                .iter()
                .map(|(name, bytes)| ((*name).to_owned(), bytes.clone()))
                .collect(),
            Vec::new(),
        )
    }

    /// 带 draft / prerelease 标记的列表。
    pub fn start_with_meta(meta: &[(&str, bool, bool)], asset_version: &str) -> Self {
        let binary = format!("akasha-fake-bw {asset_version}").into_bytes();
        Self::build(
            meta_json(meta),
            vec![asset_for(asset_version)],
            entries_with_binary(&binary),
            binary,
        )
    }

    /// 每个版本都有自己的资产（用来测"装了一个版本之后，别的版本目录被清掉"）。
    pub fn start_all(versions: &[&str]) -> Self {
        let meta: Vec<(&str, bool, bool)> = versions
            .iter()
            .map(|version| (format!("cli-v{version}").leak() as &str, false, false))
            .collect();
        let newest = versions.last().copied().unwrap_or_default();
        let binary = format!("akasha-fake-bw {newest}").into_bytes();
        Self::build(
            meta_json(&meta),
            versions.iter().map(|version| asset_for(version)).collect(),
            entries_with_binary(&binary),
            binary,
        )
    }

    fn build(
        json: String,
        assets: Vec<String>,
        entries: Vec<(String, Vec<u8>)>,
        binary: Vec<u8>,
    ) -> Self {
        // `Arc`：接受循环在另一条线程上，资产要能被它借去用（`Vec` 一 move 进闭包，
        // 下面就没法再交给 `asset_bytes()` 了）。
        let zip = Arc::new(zip_bytes(&entries));
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑一个回环端口");
        let port = listener.local_addr().expect("本地地址").port();
        listener.set_nonblocking(true).expect("接受循环要能轮询");

        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let assets = Arc::new(assets);

        let handle = {
            let requests = Arc::clone(&requests);
            let stop = Arc::clone(&stop);
            let zip = Arc::clone(&zip);
            let assets = Arc::clone(&assets);
            let json = json.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            requests.fetch_add(1, Ordering::Relaxed);
                            // ⚠️ Windows 上**接受到的那条连接会继承监听套接字的非阻塞模式**
                            // （Linux 上不会）—— 不还原成阻塞模式，`serve` 里第一句
                            // `read_line` 就有可能在客户端的请求还没到时返回 `WouldBlock`
                            // （os error 10035），这条连接随即被丢掉，而客户端读到的是
                            // "连接被中止"（os error 10053 / `Peer disconnected`，问题 #165）。
                            let _ = stream.set_nonblocking(false);
                            let _ = serve(stream, &json, &zip, &assets);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };

        Self {
            base: format!("http://127.0.0.1:{port}"),
            zip,
            binary,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    /// 把它当成 [`Sources`]（指向这个假上游）。
    pub fn sources(&self) -> Sources {
        Sources {
            releases: format!("{}/releases", self.base),
            download: format!("{}/download", self.base),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// 那个资产包的字节（用来核对报出来的 SHA-256）。
    pub fn asset_bytes(&self) -> &[u8] {
        &self.zip
    }

    /// 包里那个可执行文件的内容。
    pub fn binary_bytes(&self) -> &[u8] {
        &self.binary
    }

    /// 一共收到过几次请求（用来证明"版本解析 + 下载"确实各走了一次）。
    pub fn request_count(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn meta_json(meta: &[(&str, bool, bool)]) -> String {
    let items: Vec<String> = meta
        .iter()
        .map(|(tag, draft, prerelease)| {
            format!(r#"{{"tag_name":"{tag}","draft":{draft},"prerelease":{prerelease}}}"#)
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// 一个只含 stored 条目的 zip（不压缩：资产只有几百字节，压不压都一样）。
fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    use std::io::Cursor;

    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        writer.start_file(name.clone(), options).expect("写条目");
        writer.write_all(bytes).expect("写内容");
    }
    writer.finish().expect("收尾").into_inner()
}

/// 资产名：给一个版本，算出这个平台上的那一个。
fn asset_for(version: &str) -> String {
    let (platform, arch) = host_target().expect("假上游只在支持的平台上跑");
    asset_name(version, platform, arch).expect("资产名")
}

fn entries_with_binary(binary: &[u8]) -> Vec<(String, Vec<u8>)> {
    vec![(EXECUTABLE.to_owned(), binary.to_vec())]
}

/// 读一条请求，按路径回一个响应。
fn serve(
    mut stream: TcpStream,
    releases: &str,
    zip: &[u8],
    assets: &[String],
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // 头读完（我们不看它们，但要读掉，否则客户端可能等）。
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    let path = request_line.split_whitespace().nth(1).unwrap_or_default();
    if path.starts_with("/releases") {
        return respond(&mut stream, 200, "application/json", releases.as_bytes());
    }
    if path.starts_with("/download/") && assets.iter().any(|asset| path.ends_with(asset)) {
        return respond(&mut stream, 200, "application/zip", zip);
    }
    respond(&mut stream, 404, "text/plain", b"not found")
}

fn respond(
    stream: &mut TcpStream,
    code: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = if code == 200 { "OK" } else { "Not Found" };
    let header = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

    use super::*;

    /// 自检：这个假上游自己得先说得通（否则 `acquire` 的用例红了会怪错地方）。
    #[test]
    fn the_stub_serves_the_list_and_the_asset() {
        let stub = Stub::start(&["cli-v2026.8.0"], "2026.8.0");
        let http = crate::bw::Http::new(Duration::from_secs(5));
        let sources = stub.sources();

        let list = http.get(&sources.releases, 1024).expect("列表读得到");
        assert!(String::from_utf8_lossy(&list).contains("cli-v2026.8.0"));

        let (platform, arch) = host_target().unwrap();
        let name = asset_name("2026.8.0", platform, arch).unwrap();
        let asset = http
            .get(
                &format!("{}/cli-v2026.8.0/{name}", sources.download),
                1024 * 1024,
            )
            .expect("资产读得到");
        assert_eq!(asset, stub.asset_bytes());
        assert_eq!(stub.request_count(), 2);
    }

    /// 自检：别的路径必须是 404（否则"下载失败"那条判据会被一个万能的 200 顶过去）。
    #[test]
    fn anything_else_is_a_404() {
        let stub = Stub::start(&["cli-v2026.8.0"], "2026.8.0");
        let http = crate::bw::Http::new(Duration::from_secs(5));
        let err = http
            .get(&format!("{}/download/nope.zip", stub.base()), 1024)
            .expect_err("不存在的资产要报错");
        assert!(matches!(err, crate::bw::BwError::Network { .. }), "{err:?}");
    }
}
