# Plan 0408: 可搬迁性的平台口径分档

- **关联**：ROADMAP 阶段 4 ·「可搬迁性的平台口径分档」
- **前置**：plan 0405（可搬迁性验证，三平台同一套判据）；plan 0112（它的 macOS 验收依赖本条不动配置注入）
- **状态**：未开始
- **影响面**：`docs/portable.md`、`docs/scope.md`、`src-tauri/tests/portable.rs`、
  `docs/STATUS.md`、`ROADMAP.md`、`docs/plans/README.md`

## 目标

把「可搬迁性」的平台范围写清楚，并让它与实现一致：

1. **macOS 不做便携**：安装形态是 dmg，数据目录取 OS 标准目录；
2. **Windows / Linux 保留便携**（bin 同目录的 `akasha-data/` 就是「要便携」的标记）；
3. **Linux 的形态暂时待定**：发行版包与 AppImage 两种分发方式下，"bin 所在文件夹"并不是同一件事，
   因此 Linux 那一栏的判据先标注为暂定，等形态定了再收敛。

判据（来自 ROADMAP）：**文档与实现的口径一致，且 macOS 上不再承诺 `.app` 旁的便携目录**。

## 非目标

- **不改 ADR-0002**（已定案，不可改）：它的 D1 / D12 与 §4 只说「便携目录不可写时明确报错」，
  没有指定平台范围 —— 本 plan 动的是 `portable.md` / `scope.md` 这一层落地面。
- **不拆掉 `src-tauri/src/config/ipc.rs` 的便携检测**：`just test-e2e` 前两段的**配置注入**依赖它
  （两段都先在 bin 同目录备好便携数据目录，否则第二段写的 `close_behavior` 读不到，
  关窗语义就没有判据）。产品侧的结论是「macOS 不承诺便携」，不是「删掉这段代码」。
  ⚠️ 触发条件：若产品要求 macOS 上**严格忽略**便携目录，那要先给 `test-e2e` 的两段与 `portable`
  配方找到另一个配置注入点（写进 macOS 的 OS 数据目录并在结束后还原），那是**另一条工作项**。
- **不为 `.app` 出包**：那需要 macOS 主机与签名，属「从未验证」，见 `docs/STATUS.md` 的待验证。

## 前置检查

```bash
grep -n "fn exe_dir" -A 4 src-tauri/src/config/ipc.rs
grep -n "macOS" docs/portable.md docs/scope.md
```

- CI 的 `E2E（macOS）` 作业日志：第三段 `portable` **3/3 通过**（含 `data_survives_the_move`、
  `an_unwritable_portable_dir_refuses_to_start`）—— 这就是「macOS 上 dev 布局的便携目录今天生效」
  的读数，也是 `test-e2e` 前两段配置注入能工作的前提；
- 静态读代码：`exe_dir()` 是 `current_exe().parent()`。打包成 `Foo.app` 之后它等于
  `Foo.app/Contents/MacOS`，而 `docs/portable.md` §3 与 `docs/scope.md` §9 承诺的是
  「`.app` **旁边**」—— 那条路径在实现里**不可达**（`.app` 内部不可写，写进去还会破坏签名）。

## 步骤（每步都能独立验证）

1. **portable.md 的平台范围**：§1 的唯一要求加一句平台范围；§3 的平台表按新口径改写 ——
   macOS 行改成「不适用」并写明理由（安装形态是 `.app` / dmg，可执行文件在 bundle 内部）；
   Windows 行与 Linux 行保留；§4 触发方式与 §5 验证方法各加一句平台限定。
2. **portable.md 新增「待定：Linux 的分发形态」小节**：写清 AppImage 下可执行文件位于只读挂载点内，
   用户看到的那个文件是 `$APPIMAGE`，因此「bin 所在文件夹」的等价物**尚未定**；
   在形态定了之前，Linux 的便携判据以发行版安装为准。这一节只记待定与它的影响，不写推测性方案。
3. **scope.md**：§9 平台矩阵的 macOS 行去掉「数据目录在 `.app` 旁边」；
   §1 的 P2（可数据目录搬迁）条目补一句平台范围。§509 那句「'数据存在 bin 所在文件夹' 在 macOS
   `.app` 上会失败」仍然成立，保留。
4. **tests/portable.rs**：补一条 **macOS 上的正反例**——裸二进制同目录存在 `akasha-data/` 时被采用
   （正例，它是 E2E 配置注入的前提）、不存在时走 OS 标准目录（反例）。
   另外两条（移动后数据仍在、不可写即拒绝启动）在 macOS 上按平台**显式跳过并写明原因**：
   它们验的是产品级便携，而 macOS 不承诺。⚠️ 本步**不得**让 dev 布局失效，否则 plan 0112 的
   macOS 验收会连带失效。
5. **登记与订正**：`docs/STATUS.md` 登记问题 #182（文档曾承诺 `.app` 旁的便携目录、实现落在
   `.app` 内部）与本次口径变更；更新 `portable` 的读数行。
6. **ROADMAP**：阶段 4 的可搬迁性条目加平台限定；本 plan 完成后把 0405 与 0408 的差别写进
   `docs/plans/README.md` 索引一行（0405 = 便携本身，0408 = 它的平台范围）。

## 验收命令

```bash
# macOS：第三段（portable）在 macOS 上按平台跳过并打印原因，退出码 0；
# 新加的 dev 布局正反例在同一段里通过
just runner-run test-e2e

# Windows / Linux：原有的三条判据不变（CI 的对应格子给读数）
#   （E2E（Linux）与（Windows）的第三段）

# 文档与实现一致，且不回归
just runner-run ready        # 期望：六步全绿（含 docs-check 对 portable.md / scope.md / plan 索引的检查）
```

## 回滚

文档改动可 `git revert` 单条提交；`tests/portable.rs` 的新增正反例与两条平台跳过同理。
`config/ipc.rs` 在本 plan 里**不改**，所以代码面无回滚对象。

## 实施记录

（边做边追加：macOS 上两段配置注入的读数、`portable` 第三段在 macOS 与 Linux 上的实际输出。）
