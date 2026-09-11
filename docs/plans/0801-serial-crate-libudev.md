# Plan 0801: `src-tauri/crates/akasha-serial`（`libudev` 走 Linux-only feature）

- **关联**：ROADMAP 阶段 8 ·「`src-tauri/crates/akasha-serial`，`libudev` 走 **Linux-only cargo feature**」
- **前置**：plan 0105（`Transport` trait 已定形态）
- **状态**：未规划（骨架）
- **展开时机**：阶段 8 开工时

## 目标

serial 后端 crate：`Transport` 的串口实现，**零 Tauri 依赖**。

**`libudev` 只做 cargo feature，且只在 `target_os = "linux"` 时编译** ——
于是 Windows / macOS / Android 的构建完全不碰它（`scope.md` §8 风险 5，已定案）。

副作用要一并处理：Linux 上串口热插拔依赖 libudev，某些发行版缺失时，
串口列表应退化为"手动指定路径"，而不是硬失败。

## 非目标

- 端口枚举与参数界面（plan 0802）
- Android 的 USB Host + JNI（`later`，阶段 10 评估）

## 判据（来自 ROADMAP，展开时必须变成可粘贴的验收命令）

Windows / macOS 构建**不链接 libudev**（构建产物与 `cargo tree` 都能证）。

## 展开时要补

- [ ] 「## 步骤」：feature 名与 `cfg` 写法；`serialport` 的实际依赖形态（先验证再定）
- [ ] 「## 验收命令」：Linux 上带 feature 构建 + 交叉 `cargo check --target` 证明另一侧不带
- [ ] `capability` flag：serial 没有窗口尺寸、没有信号、没有退出码（`scope.md` §2）
