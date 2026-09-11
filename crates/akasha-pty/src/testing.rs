//! **假实现**：内存载体。
//!
//! 存在的理由（`docs/scope.md` §2 的"可 mock"）：`Transport` 的契约必须在
//! **不启动任何进程**的前提下可测 —— 否则每个用它的 crate 都得起真 shell，
//! 测试会变慢、变 flaky，还得处理"机器上没有 /bin/sh"这种事。
//!
//! 另一个用途是**验证契约本身**：真实实现（`portable-pty`）能跑的路径有限
//! （比如"不具备 resize 能力的载体该怎么答"），假实现可以把这些分支跑到。
//!
//! ⚠️ 生产代码不要用它 —— 它不会真的跑任何东西。

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::transport::{Capabilities, ExitStatus, TerminalSize, Transport, TransportError};

/// 内存里的 [`Transport`]。
pub struct FakeTransport {
    capabilities: Capabilities,
    size: TerminalSize,
    written: Vec<u8>,
    /// 读端；被 `output_stream()` 取走后为 `None`。
    output: Option<Receiver<Vec<u8>>>,
    /// 生产端。**留着**它读端才会阻塞等待；`shutdown` 时 drop 掉它 = EOF。
    feed: Option<Sender<Vec<u8>>>,
    status: Option<ExitStatus>,
    closed: bool,
}

impl FakeTransport {
    /// 造一个假载体。能力与尺寸都由测试指定 —— "不支持 resize 的载体"这种情形
    /// 在真实后端上要等到 serial 才出现，这里当场就能测。
    pub fn new(capabilities: Capabilities, size: TerminalSize) -> Self {
        let (feed, output) = mpsc::channel();
        Self {
            capabilities,
            size,
            written: Vec::new(),
            output: Some(output),
            feed: Some(feed),
            status: None,
            closed: false,
        }
    }

    /// 调用方写进来的字节。
    pub fn written(&self) -> &[u8] {
        &self.written
    }

    /// 取走写进来的字节（并清空）。
    pub fn take_written(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.written)
    }

    /// 最后一次被 `resize` 设成的尺寸。
    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// 模拟载体产出：这些字节会被 `output_stream()` 读到。
    pub fn feed(&self, bytes: impl Into<Vec<u8>>) {
        if let Some(feed) = &self.feed {
            // 读端可能已经被 drop（测试里常见）—— 那不是错误，丢掉即可。
            let _ = feed.send(bytes.into());
        }
    }

    /// 模拟载体结束（等价于真实实现的"子进程退出"），并给出结局。
    pub fn finish(&mut self, status: ExitStatus) {
        self.status = Some(status);
        self.feed = None; // drop 生产端 → 读端收到 EOF
    }
}

impl Transport for FakeTransport {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        self.written.extend_from_slice(bytes);
        Ok(())
    }

    fn output_stream(&mut self) -> Option<Box<dyn Read + Send>> {
        self.output.take().map(|rx| {
            Box::new(MemoryReader {
                rx,
                pending: VecDeque::new(),
            }) as Box<dyn Read + Send>
        })
    }

    fn resize(&mut self, size: TerminalSize) -> Result<(), TransportError> {
        // 这里刻意**照 capability 走**，而不是"假实现就随便成功"：
        // 契约要求 capability 与实际行为一致，这条路径本身就是被验的对象。
        if !self.capabilities.resize {
            return Err(TransportError::Unsupported("resize"));
        }
        self.size = size;
        Ok(())
    }

    fn exited(&mut self) -> Result<Option<ExitStatus>, TransportError> {
        Ok(self.status.clone())
    }

    fn shutdown(&mut self) -> Result<Option<ExitStatus>, TransportError> {
        self.closed = true;
        self.feed = None;
        // 幂等：已有结局就照原样返回，绝不覆盖成"成功"。
        if self.status.is_none() && self.capabilities.exit_status {
            self.status = Some(ExitStatus::Code(0));
        }
        Ok(self.status.clone())
    }
}

/// 从通道读字节的 `Read` 实现：生产端 drop 即 EOF。
struct MemoryReader {
    rx: Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
}

impl Read for MemoryReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        while self.pending.is_empty() {
            match self.rx.recv() {
                Ok(chunk) => self.pending.extend(chunk),
                Err(_) => return Ok(0), // 生产端已 drop = EOF
            }
        }
        let take = buf.len().min(self.pending.len());
        for slot in buf.iter_mut().take(take) {
            *slot = self.pending.pop_front().unwrap_or_default();
        }
        Ok(take)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> FakeTransport {
        FakeTransport::new(Capabilities::PTY, TerminalSize::DEFAULT)
    }

    #[test]
    fn write_is_recorded_verbatim() {
        let mut transport = fake();

        transport.write(b"ls -la\n").expect("write 失败");
        transport.write(&[0xff, 0x00, 0x1b]).expect("write 失败");

        // 逐字节相等：中间任何"顺手解码成字符串"都会在这里露出来。
        assert_eq!(transport.written(), b"ls -la\n\xff\x00\x1b");
        assert_eq!(transport.take_written(), b"ls -la\n\xff\x00\x1b");
        assert_eq!(transport.written(), b"");
    }

    #[test]
    fn output_stream_can_only_be_taken_once() {
        let mut transport = fake();
        let _first = transport.output_stream().expect("第一次应当成功");

        assert!(transport.output_stream().is_none(), "第二次必须是 None");
    }

    #[test]
    fn fed_bytes_come_out_of_the_output_stream() {
        let mut transport = fake();
        let mut output = transport.output_stream().expect("取输出流失败");

        transport.feed(vec![1u8, 2, 3]);

        let mut buf = [0u8; 8];
        let n = output.read(&mut buf).expect("read 失败");
        assert_eq!(&buf[..n], &[1, 2, 3]);
    }

    #[test]
    fn shutdown_ends_the_output_stream() {
        let mut transport = fake();
        let mut output = transport.output_stream().expect("取输出流失败");

        transport.shutdown().expect("shutdown 失败");

        let mut buf = [0u8; 8];
        assert_eq!(output.read(&mut buf).expect("read 失败"), 0, "应当读到 EOF");
    }

    #[test]
    fn resize_follows_the_capability_flag() {
        let mut plain = FakeTransport::new(Capabilities::NONE, TerminalSize::DEFAULT);
        assert!(
            matches!(
                plain.resize(TerminalSize::new(120, 40)),
                Err(TransportError::Unsupported("resize"))
            ),
            "不具备能力的载体必须报 Unsupported，而不是假装成功"
        );

        let mut resizable = fake();
        resizable
            .resize(TerminalSize::new(120, 40))
            .expect("具备能力的载体应当成功");
        assert_eq!(resizable.size(), TerminalSize::new(120, 40));
    }

    #[test]
    fn write_after_shutdown_is_closed() {
        let mut transport = fake();
        transport.shutdown().expect("shutdown 失败");

        assert!(matches!(transport.write(b"x"), Err(TransportError::Closed)));
    }

    #[test]
    fn shutdown_is_idempotent_and_keeps_the_first_outcome() {
        let mut transport = fake();

        let first = transport.shutdown().expect("shutdown 失败");
        let second = transport.shutdown().expect("重复 shutdown 必须成功");
        assert_eq!(first, second);

        let mut exited = fake();
        exited.finish(ExitStatus::Code(3));
        assert_eq!(
            exited.shutdown().expect("shutdown 失败"),
            Some(ExitStatus::Code(3)),
            "已有结局不得被 shutdown 覆盖成默认成功"
        );
    }

    #[test]
    fn without_exit_status_capability_shutdown_reports_nothing() {
        let mut transport = FakeTransport::new(Capabilities::NONE, TerminalSize::DEFAULT);

        assert_eq!(
            transport.shutdown().expect("shutdown 失败"),
            None,
            "没有 exit_status 能力的载体没有结局可报 —— 这不是失败"
        );
    }

    #[test]
    fn finish_is_visible_through_exited() {
        let mut transport = fake();
        assert_eq!(transport.exited().expect("exited 失败"), None);

        transport.finish(ExitStatus::Signal("SIGHUP".into()));

        assert_eq!(
            transport.exited().expect("exited 失败"),
            Some(ExitStatus::Signal("SIGHUP".into()))
        );
        assert!(!ExitStatus::Signal("SIGHUP".into()).success());
        assert!(ExitStatus::Code(0).success());
        assert!(!ExitStatus::Code(1).success());
    }
}
