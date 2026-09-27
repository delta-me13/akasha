# ROADMAP

> 回答"我们去哪"："下一步做什么"只在这一个文件里，不散落在 `AGENTS.md` 或 `docs/STATUS.md`。
> 这里只勾复选框，不写细节：具体步骤在 [`docs/plans/`](./docs/plans/)，
> 当前到哪一步在 [`docs/STATUS.md`](./docs/STATUS.md)。
>
> 每个条目必须有可执行的验收标准。写不出验收标准的条目不许进入本文件。
>
> 能力清单在 [`docs/scope.md`](./docs/scope.md)：本文件的阶段从它派生。
> 每个条目末尾挂一个 plan 指针：怎么做、可粘贴的验收命令都在那里；
> plan 用阶段块号（`TTxx`），完整索引见 [`docs/plans/README.md`](./docs/plans/README.md)。

图例：`[ ]` 未开始 · `[~]` 进行中 · `[!]` 被阻塞。
**已完成的条目不在本文件里** —— 它们移入 [归档](./docs/archive/roadmap-completed.md)（`AGENTS.md` §8.3）。

---

## 阶段 0 — 地基（可编译、可验证、可交接）

目标：任何时刻 `just ready` 通过，规范、状态、进度各有唯一来源。

阶段 0 的条目按约定不回填 plan 指针，记录在提交历史与 `STATUS.md`。

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。
- [~] CI 通过（每平台一条流水线：检查 → E2E 两段串行，六个 job）
      验收：CI 上六个 job 通过

---

## 阶段 1 — 分层与平台矩阵

目标：后端分层落地，且跨平台差异从一开始就被验证。

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。
- [~] CI 平台矩阵（Linux + Windows + macOS，GitHub Actions 一份）
      验收：三平台都能通过类型检查；Linux 另执行完整门禁与 E2E
      → [plan 0102](./docs/plans/0102-ci-platform-matrix.md)
- [~] E2E 入口可实际运行：`just test-e2e` 自包含（启动 app → 运行全部 E2E 目标 → 收尾）
      验收：没有 app 在运行时它也退出码 0；新增的 E2E 目标不接入即失败；CI 上三平台执行
      → [plan 0107](./docs/plans/0107-e2e-entry.md)
- [~] Windows 目标的类型检查通过（问题 #149、#160）
      验收：Windows 目标上的类型检查退出码 0
      → [plan 0108](./docs/plans/0108-windows-type-check.md)

---

## 阶段 2 — 端到端最小终端

目标：可打开一个 shell、执行命令、看到输出，且真正退出后不残留进程。

---

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。
## 阶段 3 — 托盘与应用生命周期

目标：关闭窗口不退出；隧道与终端在窗口隐藏期间存活。

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。

---

## 阶段 4 — 存储与凭据池

目标：四类池可增删改查；库加密；可导出。

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。

---

## 阶段 5 — SSH 栈（`russh`）

目标：纯 Rust SSH 实现，不调用系统 `ssh`。

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。

---

## 阶段 6 — SSH 端口转发

目标：本地/远程/动态三类转发，独立 Session，失败可见。

---

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。
## 阶段 7 — SFTP

目标：双栏传输可用，且**任何时刻都不留下看似完整、实为不完整的文件**。

ADR-0006（SFTP 栈与传输引擎）的落地计划到 0704 为止；本阶段收尾时转为「已定案」。

---

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。
## 阶段 8 — serial

目标：串口能枚举、能按参数打开，且不把 libudev 带进别的平台。

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。
- [ ] Windows 上的枚举带得出描述（条件编译；上游在那边把一切都报成"未知"）
      验收：Windows 上枚举出来的端口各自带得出描述；没有端口时给出空表而不是错误
      → [plan 0803](./docs/plans/0803-windows-serial-tests.md)

---

## 阶段 9 — Bitwarden 导入

目标：CLI 的获取与前置条件明确，登录（含自托管）在前端可用，只读导入可用，离线缓存强度与本地池同级。

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。
- [ ] `bw` 对 `sshKey` 条目的非交互行为有结论（需要真实 vault）
      验收：登录之后的 `bw status` 形状、未解锁时的报错、条目的 JSON 形状与离线可见性各有结论，写回 [`docs/bitwarden.md`](./docs/bitwarden.md)
      → [plan 0901](./docs/plans/0901-bw-noninteractive-probe.md)

---

## 阶段 10 — Android（延迟）

目标：显式决定 Android 的形态，并把它写进 `scope.md`。

- [ ] 评估 Android 上的托盘与端口转发形态
      验收：结论写进 `scope.md` §9 / §10，且有可核对的出处
      → [plan 1001](./docs/plans/1001-android-form-factor.md)
- [ ] 只做 ssh/sftp，还是接 Termux（local）/ USB Host（serial）
      验收：这个决定必须显式做出并记录，不能是意外结果
      → [plan 1002](./docs/plans/1002-android-scope-decision.md)

---

## 阶段 11 — 串口接入 app

目标：`scope.md` §2 的第三个后端在 app 上可用（阶段 8 的续作）。

---

> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。
## 明确的非目标

理由不在这里（按 §8.1，理由属于 `scope.md`），只列条目：

- 不在 Rust 侧实现屏幕模型
- 不做插件系统
- 不自实现 Bitwarden vault 密码学；**不打包 `bw`**
- 不调用系统 `ssh` 二进制
- 不为 webview 依赖栈做环境重定向
- 不做 host↔host"真不中转"
- 不做串口热插拔事件驱动的自动重连

每条的理由与完整清单见 [`docs/scope.md`](./docs/scope.md) §10。
