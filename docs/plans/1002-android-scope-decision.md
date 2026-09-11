# Plan 1002: 只做 ssh/sftp 还是接 Termux / USB Host

- **关联**：ROADMAP 阶段 10 ·「只做 ssh/sftp，还是接 Termux（local）/ USB Host（serial）」
- **前置**：plan 1001（形态评估）
- **状态**：未规划（骨架）
- **展开时机**：plan 1001 完成之后

## 目标

**显式做出并记录**这个决定：Android 上只做 ssh/sftp，还是再接 Termux（local）与 USB Host（serial）。

背景事实（决定了取舍的方向）：Android 上普通 app **拿不到 `/dev/ptmx`**（SELinux 沙箱）——
Termux 能用 PTY 是因为它自带 userland bootstrap，不是普通 app 的路径（`scope.md` §2）。

## 非目标

- 实现任何一种形态（本 plan 只出决定）
- 移动端除 Android 的 ssh/sftp 之外的范围（`later`）

## 判据（来自 ROADMAP，展开时必须变成可粘贴的验收命令）

这个决定必须**显式做出并记录**，**不能是意外结果** —— 落点是 [`../scope.md`](../scope.md) §2 / §9。

## 展开时要补

- [ ] 「## 步骤」：备选方案 × 代价表（Termux 依赖 = 引入外部前置，与 P1 的关系要写清）
- [ ] 「## 验收命令」：决定写入 `scope.md` 的具体章节 + docs-check 绿
- [ ] 若结论是"降级为只做 ssh/sftp"，把它登记为非目标/降级形态（而不是悄悄少做）
