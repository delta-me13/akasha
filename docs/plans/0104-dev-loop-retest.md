# Plan 0104: 迁移后复测开发循环

- **关联**：ROADMAP 阶段 1 ·「迁移后复测开发循环」
- **前置**：plan 0101（迁移完成）+ plan 0103（**必须先有 `crates/` 成员**，否则监听范围无从测）
- **状态**：未开始
- **影响面**：无（纯验证）。若监听范围失效，改动落在 `src-tauri/Cargo.toml` 或 `.taurignore`

## 目标

确认根 workspace 布局下，`just dev` 的重编译 / 重启 / **监听范围**与迁移前基线一致。

**最大的隐性风险是监听范围**：CLI 打印的监听路径原本是 `src-tauri`，
迁移后 `crates/*` 落在它之外。若 crate 改动不再触发重启，开发循环会**静默失效** ——
`cargo check` / `just ready` 全绿，但你改了代码看不到效果，而没有任何门禁会告诉你。

## 非目标

- 不改业务代码（本 plan 只复测，必要时只动 watcher 配置）
- 不重复 plan 0101 的验收（结构是否正确已在那里验过）

## 前置检查

```bash
pgrep -af 'tauri dev|target/debug/akasha' || echo "无遗留 just dev（预期）"
```

⚠️ 若有遗留进程**先停掉**：它的监听范围是旧布局，会让复测结果失真（坑 #15 ——
那种状态下"端口在听、HTTP 200，但 app 没在运行"极具迷惑性）。

## 步骤

1. 从仓库根起 `just dev`，记录：二进制落点、冷编译耗时、dev server 端口、CLI 打印的监听路径。
2. 按下表逐项对比迁移前基线。
3. **监听范围实测**：改 `crates/akasha-core/src/lib.rs` 里的一行注释，
   观察 CLI 是否重编译并重启 app。
4. 若失效：在 `src-tauri/Cargo.toml` / `.taurignore` 层面处理（把 `crates/` 纳入 watch 范围），
   **不要**接受"静默失效"。处置写进「实施记录」。
5. 顺带走一遍 IPC 与 Victauri：`just doctor` 确认连的是本项目；打一次已有 command。

## 基线对照表（迁移前实测，出处 `docs/STATUS.md`）

| 项 | 迁移前 | 迁移后应为 |
|---|---|---|
| 二进制落点 | `src-tauri/target/debug/akasha` | `<root>/target/debug/akasha` |
| 冷编译 / 增量 | 47.53s / 6.09s | 同量级（不应变慢） |
| dev server | Vite 1420、HTTP 200 | 同上 |
| **CLI 监听范围** | `Watching .../src-tauri for changes` | **改 `crates/` 下的文件仍触发重编译 + 重启** |
| IPC 端到端 | 已有 command 返回预期结果 | 同样返回 |
| Victauri | 连上、identifier 为 `fans.cyrene.akasha-terminal` | 同样连上 |

## 验收命令

```bash
just dev                       # 一个终端常驻；期望能起窗口
# 另开一个终端触发一次 crate 改动：
printf '\n// watch-probe\n' >> crates/akasha-core/src/lib.rs
#   期望：just dev 的 CLI 输出立刻出现重编译 + 重启
git checkout -- crates/akasha-core/src/lib.rs     # 已提交文件，checkout 安全
just doctor                    # 期望识别到本项目
```

> `git checkout` 只对**已提交**的文件安全；未提交的改动用它还原会静默丢失（坑 #12）。

## 回滚

纯验证，无改动。若为此改了 watcher 配置，`git revert <commit>`。

## 实施记录

（边做边追加：**必须粘贴 CLI 的实际输出**，尤其是监听范围那一行。）
