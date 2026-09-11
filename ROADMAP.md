# ROADMAP

> **回答"我们去哪"** —— "下一步做什么"只在这一个文件里，不散落在 `AGENTS.md` 或 `docs/STATUS.md`。
> 这里**只勾复选框**，不写细节：具体步骤在 [`docs/plans/`](./docs/plans/)，
> 当前到哪一步在 [`docs/STATUS.md`](./docs/STATUS.md)。
>
> 每个条目必须有**可执行的验收标准**。写不出验收标准的条目不许进入本文件。

图例：`[x]` 已完成 · `[ ]` 未开始 · `[~]` 进行中 · `[!]` 被阻塞

---

## 阶段 0 — 地基（可编译、可验证、可交接）

目标：任何时刻 `just ready` 是绿的，CI 是绿的，规范/现状/进度各有唯一来源。

- [x] 工具链与依赖就位（`just check` / `just lint` 退出码 0）
- [x] 依赖门禁（`just deny-offline` → bans / licenses / sources all ok）
- [x] 规范与文档体系（`AGENTS.md` + `ROADMAP.md` + `docs/`）
- [~] **修好 CI** —— 原 `victauri.yml` 在仓库根跑 `cargo build`（根目录无 `Cargo.toml`），
      且两个 job 分在两个文件里无法用 `needs` 串联。已合并为 `.github/workflows/ci.yml`：
      `checks` 跑 `just ready` + `just docs-check`，`e2e` 依赖 `checks`
      验收：CI 上两个 job 变绿 —— **待首次推送确认**
      （本地只能校验 YAML 合法性、命令存在性与依赖版本，跑不了 runner）
- [ ] **ADR-0001 定案**（工作区切分 + PTY 抽象，当前状态：提议）
      验收：`docs/adr/0001` 状态改为"已接受"，或写明被哪条取代
- [ ] **落地 ADR-0001 决策一**（根 workspace）
      验收：仓库根 `cargo metadata --no-deps` 列出 ≥2 个包，且 `just ready` 仍全绿
      见 [`docs/plans/0001`](./docs/plans/0001-root-workspace.md)

## 阶段 1 — 端到端最小可用终端

目标：能开一个 shell、敲命令、看到输出、关窗口不残留进程。

- [ ] `crates/akasha-pty` 骨架：`PtyBackend` / `PtySession` trait + `portable-pty` 实现
      验收：`FakePty` 覆盖 spawn / write / shutdown 的单元测试通过
- [ ] PTY 输出合批（≥16ms 或 ≥64KiB）
      验收：单元测试断言 chunk 边界；`criterion` 基准给出吞吐数字
- [ ] IPC 二进制通道（`tauri::ipc::Channel<Vec<u8>>`）
      验收：`ast-grep` 规则 `no-string-pty-channel` 生效；大输出（`yes | head -c 10M`）不掉帧
- [ ] 前端 xterm + WebGL 渲染，字节流不进 React state
      验收：Victauri `dom_snapshot` 确认 canvas 存在；无 per-chunk 组件重渲染
- [ ] 进程生命周期三路径收尸（窗口关闭 / app 重载 / panic）
      验收：Victauri `introspect { action: "processes" }` 关闭后无残留 PTY 子进程

## 阶段 2 — 日常可用

- [ ] 多标签 / 分屏会话模型
- [ ] 搜索（`addon-search`）、复制粘贴、字体与主题
- [ ] 复制粘贴安全：OSC 52 默认拒绝 + 显式确认
- [ ] 会话序列化恢复（`addon-serialize`）

## 阶段 3 — 类型边界与安全收口

- [ ] `tauri-specta` 生成 `src/ipc/bindings.ts`，并纳入 `just ready` 的漂移检查
      验收：`just gen-types && git diff --exit-code` 为空
- [ ] OSC 8 超链接白名单、设备查询白名单
- [ ] `tauri.conf.json` 的 `csp` 从 `null` 收紧为最小策略

## 阶段 4 — 平台与质量

- [ ] CI 加 macOS / Windows 矩阵（PTY 语义差异最大处）
- [ ] `criterion` 性能基线纳入 CI，防止吞吐回归
- [ ] Windows ConPTY 行为验证（ADR-0001 未决项 2）

---

## 能力范围

**本文件只管"去哪"，不管"有什么"。** 能力清单、平台矩阵与明确的非目标在
[`docs/scope.md`](./docs/scope.md) —— 那里是能力的唯一来源，本文件的阶段从它派生。

> 2026 变更：产品范围已从"一个终端"扩展到**多后端（local/ssh/serial）+ SFTP +
> 凭据池 + Bitwarden**，见 `docs/scope.md`。下列阶段 1–4 的切分**正在重构**
> （尤其 CI 平台矩阵必须从阶段 4 提前到阶段 1——平台差异是主体工作量，不是收尾工作）。
> 重构前以 `docs/scope.md` 为准。

### 仍然成立的非目标（详见 `docs/scope.md` §9）

- 不在 Rust 侧实现屏幕模型（除非 `docs/adr/0001` §3 的触发条件满足）
- 不做插件系统
- 不自实现 Bitwarden vault 密码学
- 不做 host↔host"真不中转"（需 A↔B 互信）
