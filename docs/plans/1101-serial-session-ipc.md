# Plan 1101: 串口 `Session` 接入 app

- **关联**：ROADMAP 阶段 11 ·「串口 `Session` 接入 app（命令 + 注册表 + 标签页 + 关闭与回收）」
- **前置**：plan 0801 / 0802（`akasha-serial` 的载体、`ports()` 与参数校验已就位）
- **状态**：未规划（骨架）
- **展开时机**：阶段 11 立项时（紧接阶段 8 的 crate 工作）

## 目标

把 `akasha-serial` 接到 app 上：一个串口 `Session` 由命令打开、字节双向流动、
关闭标签页即丢掉它自己。

## 非目标

- 端口枚举与参数界面（plan 1102）；设备消失时的表现（plan 1103）
- 串口热插拔事件驱动的自动重连（`scope.md` §10，`later`）
- 串口协议的解析（只搬字节）

## 判据（来自 ROADMAP，展开时必须变成可粘贴的验收命令）

真实 app 上打开一个串口会话并双向传字节；关闭标签页后 `live` / `registered` 归零。

## 展开时要补

- [ ] 「## 前置检查」：串口在 `Sessions` 里占哪张表（与 SSH、隧道、SFTP 的关系）；
      没有本地进程时"撤销兜底登记"与 `session_leader()` 走哪条路（与 SSH 同一形状）
- [ ] 「## 步骤」：命令与事件的名字；池行 → `SerialSettings` 的字段搬运落在哪一层
      （`TryFrom<u8>` 已在 plan 0802 就位）
- [ ] 「## 验收命令」：本机没有串口硬件 —— 用 `portable-pty` 的从端当设备（plan 0801 的路子）
- [ ] 「## 验收命令」：`introspect { action: "processes" }` 在真正退出之后确认零残留
