use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::event::SessionEvent;
use crate::session::{SessionError, SessionId, SessionKind};

/// `Session` 的登记簿，**兼事件路由表**。
///
/// 为什么这两件事合成一个类型：**"关闭后不再向它投递事件"这条不变量横跨两者**。
/// 若登记与订阅分属两个类型，关闭就得靠调用方记得同时清理订阅 —— 忘一次就是
/// "给已关闭的 `Session` 攒事件"，属于最难发现、也最容易泄漏的那类问题。
/// 合成一个类型后，`close` 是关闭的唯一入口，不变量不可能被绕过。
///
/// 用 `BTreeMap` 而不是 `HashMap`：ID 有序，遍历结果稳定，
/// 测试里可以直接断言顺序，不需要额外排序。
#[derive(Debug)]
pub struct SessionRegistry {
    /// 下一个待分配的原始 ID。从 1 起，于是 `0` 永远是一个"明显不对劲"的值。
    next_raw: u64,
    kinds: BTreeMap<SessionId, SessionKind>,
    sinks: BTreeMap<SessionId, Vec<Sender<SessionEvent>>>,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRegistry {
    /// 空注册表。
    pub const fn new() -> Self {
        Self {
            next_raw: 1,
            kinds: BTreeMap::new(),
            sinks: BTreeMap::new(),
        }
    }

    /// 登记一个新 `Session`。ID 单调递增、**永不复用**（复用会让"已关闭的旧 ID"
    /// 与"新 Session"在日志与事件里撞在一起，是最难查的一类串号）。
    ///
    /// 此刻还没有订阅者（订阅要求已登记），所以这里不广播 `Opened` ——
    /// 按 `SessionId` 路由的模型下，它本来就不该有"生命周期广播"这种东西。
    pub fn open(&mut self, kind: SessionKind) -> Result<SessionId, SessionError> {
        let id = SessionId::from_raw(self.next_raw);
        self.next_raw = self
            .next_raw
            .checked_add(1)
            .ok_or(SessionError::IdExhausted)?;
        self.kinds.insert(id, kind);
        Ok(id)
    }

    /// 关闭一个 `Session`：**先通知它的订阅者，再撤掉订阅，最后从登记簿移除**。
    ///
    /// 关闭是幂等意义上的"一次性"操作：重复关闭返回
    /// [`SessionError::NotFound`]，而不是静默成功 —— 静默成功会让调用方
    /// 以为"我关掉了什么"，其实关掉的是别的 `Session`。
    pub fn close(&mut self, id: SessionId) -> Result<(), SessionError> {
        if self.kinds.remove(&id).is_none() {
            return Err(SessionError::NotFound(id));
        }
        // 订阅者还能收到最后一条事件（"它关了"），这正是它们需要知道的。
        self.emit(SessionEvent::Closed { id });
        // 撤掉订阅：此后对该 ID 的投递一律为 0。这就是上面说的那条不变量。
        self.sinks.remove(&id);
        Ok(())
    }

    /// 查询某个 `Session` 的种类；已关闭或从未存在都是 `None`。
    pub fn kind(&self, id: SessionId) -> Option<SessionKind> {
        self.kinds.get(&id).copied()
    }

    /// 该 `Session` 是否仍然登记在册。
    pub fn is_open(&self, id: SessionId) -> bool {
        self.kinds.contains_key(&id)
    }

    /// 当前存活的 `Session` 数。
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    /// 是否一个 `Session` 都没有。
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// 存活的 `Session` ID（升序）。
    pub fn ids(&self) -> impl Iterator<Item = SessionId> + '_ {
        self.kinds.keys().copied()
    }

    /// 订阅某个 `Session` 的事件。
    ///
    /// **只能订阅已登记的 `Session`**：允许订阅未知 ID 就等于允许
    /// "给一个不存在的 `Session` 攒事件"，而那正是"关闭后不再投递"要防的东西。
    pub fn subscribe(&mut self, id: SessionId) -> Result<Receiver<SessionEvent>, SessionError> {
        if !self.kinds.contains_key(&id) {
            return Err(SessionError::NotFound(id));
        }
        let (sender, receiver) = mpsc::channel();
        self.sinks.entry(id).or_default().push(sender);
        Ok(receiver)
    }

    /// 按事件自带的 `SessionId` 投递，返回**实际投递成功的订阅者数**。
    ///
    /// 已关闭或从未存在的 ID → `0`，事件被丢弃。**没有全局接收方**：
    /// 这正是"按 `SessionId` 路由"与"全局广播后由前端过滤"的区别。
    /// 接收端已 drop 的订阅者会被顺手回收，免得投递列表只增不减。
    pub fn emit(&mut self, event: SessionEvent) -> usize {
        let Some(sinks) = self.sinks.get_mut(&event.session()) else {
            return 0;
        };
        let mut delivered = 0;
        sinks.retain(|sender| match sender.send(event.clone()) {
            Ok(()) => {
                delivered += 1;
                true
            }
            Err(_) => false,
        });
        delivered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::TryRecvError;

    /// 测试里用 `expect` 是有意的：它是断言手段，不属于 `AGENTS.md` §0 说的
    /// "command 边界或长驻任务"。本 crate 的**生产代码零 `unwrap` / `expect`**。
    #[test]
    fn open_returns_unique_ids_and_registers_kind() {
        let mut registry = SessionRegistry::new();
        let mut ids = Vec::new();

        for kind in SessionKind::ALL {
            let id = registry.open(kind).expect("open 不应失败");
            assert_eq!(registry.kind(id), Some(kind), "登记的种类必须能查回来");
            ids.push(id);
        }

        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "分配的 ID 必须两两不同");
        assert_eq!(registry.len(), SessionKind::ALL.len());
        assert!(!registry.is_empty());
    }

    #[test]
    fn closing_one_session_leaves_others_untouched() {
        let mut registry = SessionRegistry::new();
        let first = registry.open(SessionKind::Terminal).expect("open");
        let second = registry.open(SessionKind::Tunnel).expect("open");

        registry.close(first).expect("close");

        assert!(!registry.is_open(first), "被关的那个必须不再存活");
        assert_eq!(
            registry.kind(second),
            Some(SessionKind::Tunnel),
            "关一个 Session 不得影响另一个"
        );
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn closing_an_unregistered_session_is_an_error_not_a_panic() {
        let mut registry = SessionRegistry::new();
        let id = registry.open(SessionKind::Vault).expect("open");

        registry.close(id).expect("第一次 close 应成功");

        assert_eq!(
            registry.close(id),
            Err(SessionError::NotFound(id)),
            "重复 close 必须是 Err —— 静默成功会让调用方以为关掉了别的东西"
        );
    }

    #[test]
    fn events_are_routed_by_session_id_without_crosstalk() {
        let mut registry = SessionRegistry::new();
        let terminal = registry.open(SessionKind::Terminal).expect("open");
        let sftp = registry.open(SessionKind::Sftp).expect("open");
        let terminal_rx = registry.subscribe(terminal).expect("subscribe");
        let sftp_rx = registry.subscribe(sftp).expect("subscribe");

        let delivered = registry.emit(SessionEvent::Closed { id: terminal });

        assert_eq!(delivered, 1, "只应投给 terminal 的订阅者");
        assert!(matches!(
            terminal_rx.try_recv(),
            Ok(SessionEvent::Closed { id }) if id == terminal
        ));
        assert_eq!(
            sftp_rx.try_recv(),
            Err(TryRecvError::Empty),
            "sftp 不该收到 terminal 的事件（串号正是全局广播的病）"
        );
    }

    #[test]
    fn nothing_is_delivered_after_close() {
        let mut registry = SessionRegistry::new();
        let id = registry.open(SessionKind::Tunnel).expect("open");
        let rx = registry.subscribe(id).expect("subscribe");

        registry.close(id).expect("close");

        assert!(
            matches!(rx.try_recv(), Ok(SessionEvent::Closed { id: closed }) if closed == id),
            "关闭时订阅者应收到最后一条 Closed"
        );
        assert_eq!(
            registry.emit(SessionEvent::Closed { id }),
            0,
            "已关闭的 Session 不再收到任何事件"
        );
        assert_eq!(
            rx.try_recv(),
            Err(TryRecvError::Disconnected),
            "订阅通道应已被撤掉，而不是留着一个永远收不到东西的订阅"
        );
    }

    #[test]
    fn subscribing_to_an_unregistered_session_is_rejected() {
        let mut registry = SessionRegistry::new();
        let id = registry.open(SessionKind::Terminal).expect("open");
        registry.close(id).expect("close");

        assert_eq!(
            registry.subscribe(id).err(),
            Some(SessionError::NotFound(id)),
            "不得给已关闭的 Session 订阅"
        );
    }

    #[test]
    fn many_sessions_stay_distinct_under_interleaved_close() {
        let mut registry = SessionRegistry::new();
        let ids: Vec<SessionId> = (0..8)
            .map(|i| {
                let kind = SessionKind::ALL[i % SessionKind::ALL.len()];
                registry.open(kind).expect("open")
            })
            .collect();

        for (i, id) in ids.iter().enumerate() {
            if i % 2 == 0 {
                registry.close(*id).expect("close");
            }
        }

        let survivors: Vec<SessionId> = ids
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 2 == 1)
            .map(|(_, id)| *id)
            .collect();

        assert_eq!(registry.ids().collect::<Vec<_>>(), survivors);
        assert_eq!(registry.len(), survivors.len());
        for id in survivors {
            assert!(registry.is_open(id), "存活的 Session 不该被误关");
        }
    }

    #[test]
    fn tunnel_state_events_are_routed_by_session_id() {
        use crate::tunnel::TunnelState;

        let mut registry = SessionRegistry::new();
        let first = registry.open(SessionKind::Tunnel).expect("open");
        let second = registry.open(SessionKind::Tunnel).expect("open");
        let first_rx = registry.subscribe(first).expect("subscribe");
        let second_rx = registry.subscribe(second).expect("subscribe");

        let delivered = registry.emit(SessionEvent::TunnelStateChanged {
            id: first,
            state: TunnelState::Connected,
        });

        assert_eq!(delivered, 1, "只应投给那一条隧道的订阅者");
        assert!(matches!(
            first_rx.try_recv(),
            Ok(SessionEvent::TunnelStateChanged { id, state })
                if id == first && state == TunnelState::Connected
        ));
        assert_eq!(
            second_rx.try_recv(),
            Err(TryRecvError::Empty),
            "另一条隧道不该收到状态事件（串号正是全局广播的病）"
        );
    }

    #[test]
    fn every_session_kind_has_a_distinct_short_name() {
        let names: Vec<&str> = SessionKind::ALL.iter().map(|k| k.as_str()).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "短名必须互不相同");
    }
}
