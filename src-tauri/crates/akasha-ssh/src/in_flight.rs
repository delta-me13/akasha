//! **并发 in-flight 的上限**（ADR-0006 D6）—— 一次最多几个文件同时在搬。
//!
//! 上游按请求 id 支持并发（§2 的事实表），但"同时开多少个**文件**"是我们自己的策略，
//! 而这个策略只有一条判据：**别让往返延迟白白占着**。小文件的墙钟时间几乎全是一次次
//! 往返的叠加（`scope.md` §4.1：拓扑再好也不能弥补串行请求），所以让第 2 个文件的第一次
//! 往返排在第一个文件的第一块之前发出去，就是把等待折叠起来。
//!
//! ## 为什么是一个能跨任务共享的东西，而不是一个参数
//!
//! 引擎的入口是**一个文件**（[`crate::transfer::transfer`]），而"同时几个"这件事只有把
//! 多条传输放在一起才谈得上。app 侧的形状是"一个文件一条命令、一个独立任务"，所以上限
//! 不能挂在某一次调用上 —— 它挂在一个 [`InFlight`] 上，由**一个 SFTP 会话**持有
//! （`src-tauri/src/sftp.rs`），于是它约束的是这个会话的**全部**传输，不区分它们来自几条命令。
//! plan 0704 之前这里没有上限：多开一条命令就多占一份 32 KiB 缓冲与一次在途请求。
//!
//! ## 两个读数
//!
//! [`InFlight::live`] 与 [`InFlight::peak`] 是**上限真的在起作用**的证据，而不是装饰：
//! 上限那一条判据的形态是"第 N+1 个文件在有人让出空位之前一次都没开始"，而这件事在面板上
//! 只能靠这两个数看见（探针 `sftp` 与前端都读它们）。
//!
//! ⚠️ **排队本身不是一个状态**：一条已经登记、还在等空位的传输，在传输记录里与"正在搬"
//! 那一条长得一样（app 侧的状态枚举只有"还没结束"这一档）。分辨它们的是 `live`：
//! `live` 比"还没结束的条数"少，就说明有文件在排队。
//!
//! ## 排队是可以取消的
//!
//! 关会话与用户取消推的是**同一个**信号（ADR-0006 D4），而排队中的传输还没碰过任何端点。
//! 所以等空位这件事本身要能被取消（[`InFlight::transfer`] 用 `select!` 等）—— 否则一条
//! 已经取消、却排在队里的传输会先建出一个临时文件、再把它删掉，而"关会话"要等它走完这两步。

use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::{Semaphore, SemaphorePermit};

use crate::error::SshError;
use crate::transfer::{CancelWaiter, Endpoint, Progress, TransferRequest, transfer};

/// 并发上限：**同时有几个文件在搬**。
///
/// 它不持连接、不持端点，只持空位 —— 谁把端点交给它，谁就按它的上限使用对端。
pub struct InFlight {
    /// 上限本身（[`Self::new`] 已经把 `0` 拨到 `1`，所以这里永远不会是 0）。
    limit: u32,
    /// 空位。
    permits: Semaphore,
    /// 此刻握着空位的文件数。
    live: AtomicUsize,
    /// 这个上限被用到的最大值（**会话生命期内**，不随传输结束回落）。
    peak: AtomicUsize,
}

impl InFlight {
    /// 建一个上限。`limit = 0` 当作 `1`：**"零个文件同时在搬"不是一种能工作的配置**
    /// （每一条传输都会永远等在空位上），而它是配置文件里一个可能被写出来的值 ——
    /// 与其在等不到的时候失败，不如在这里兜住。
    pub fn new(limit: u32) -> Self {
        let limit = limit.max(1);
        Self {
            limit,
            permits: Semaphore::new(limit as usize),
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    /// 上限是多少。
    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// 此刻有几个文件在搬（**含**它们正在打开源、正在提交重命名的那段时间）。
    pub fn live(&self) -> u32 {
        self.live.load(Ordering::Relaxed) as u32
    }

    /// 这个会话见过的最多同时几个。
    pub fn peak(&self) -> u32 {
        self.peak.load(Ordering::Relaxed) as u32
    }

    /// **有界入口**：可取消地等一个空位，然后把这一次搬运交给
    /// [`transfer`]。
    ///
    /// 与直接调用 [`transfer`] 的差别只有"排队"这一步：排队中被取消时**一个端点都没碰过**
    /// （返回 [`SshError::Cancelled`]，目标目录里不会出现这次的临时名）。
    pub async fn transfer(
        &self,
        source: &dyn Endpoint,
        target: &dyn Endpoint,
        request: &TransferRequest<'_>,
        progress: &Progress,
        mut cancel: CancelWaiter,
    ) -> Result<u64, SshError> {
        let Some(_slot) = self.acquire(&mut cancel).await else {
            return Err(SshError::Cancelled);
        };
        transfer(source, target, request, progress, cancel).await
    }

    /// 等一个空位；`None` = 等的时候被取消了。
    async fn acquire(&self, cancel: &mut CancelWaiter) -> Option<InFlightSlot<'_>> {
        let permit = tokio::select! {
            // `biased`：取消那一支**先看**。不加它时 `select!` 随机挑就绪的分支 ——
            // "已经取消、而空位又正好空着"会有一半的机会先拿到空位，于是那条传输照样
            // 会去建一个临时文件、再自己删掉（`transfer` 的第一个取消点在任何 I/O 之后）。
            biased;
            () = cancel.wait() => return None,
            permit = self.permits.acquire() => match permit {
                Ok(permit) => permit,
                // 只有"信号量被关掉"会到这里，而我们从不 close 它 ——
                // 真到了那里也只能当作"等不到空位"。
                Err(_) => return None,
            },
        };
        let live = self.live.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak.fetch_max(live, Ordering::Relaxed);
        Some(InFlightSlot {
            _permit: permit,
            live: &self.live,
        })
    }
}

/// 握着一个空位。**drop = 归还**（不写显式的归还方法：漏掉一次归还的表现是上限慢慢变成 1，
/// 而那正是 Drop 能免疫的错误）。
struct InFlightSlot<'a> {
    _permit: SemaphorePermit<'a>,
    /// 空位归还时把活着的计数减回去。
    live: &'a AtomicUsize,
}

impl Drop for InFlightSlot<'_> {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `0` 不是"不限制"，是"永远等不到" —— 配置文件里写得出来的值不能让它变成死等。
    #[test]
    fn a_zero_limit_becomes_one() {
        assert_eq!(InFlight::new(0).limit(), 1);
        assert_eq!(InFlight::new(1).limit(), 1);
        assert_eq!(InFlight::new(8).limit(), 8);
    }

    /// 三个读数一开始都是干净的（`peak` 不虚构：谁都没搬过时它是 0）。
    #[test]
    fn a_new_limit_reports_nothing_in_flight() {
        let in_flight = InFlight::new(4);
        assert_eq!(in_flight.live(), 0);
        assert_eq!(in_flight.peak(), 0);
    }
}
