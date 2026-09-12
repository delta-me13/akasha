# Plan 0401: `rusqlite` + SQLCipher 打开加密库

- **关联**：ROADMAP 阶段 4 ·「`rusqlite` + **SQLCipher**（`bundled-sqlcipher-vendored-openssl`）」
- **前置**：plan 0400（ADR-0002 定案）· plan 0103（core 已立）
- **状态**：未规划（骨架）
- **展开时机**：ADR-0002 定案后、写下第一行存储代码之前

## 目标

用 `bundled-sqlcipher-vendored-openssl` 打开 / 创建加密库。

`bundled-sqlcipher` **单独用不够**：没有 `-vendored-openssl` 就会撞系统 OpenSSL，
四平台构建的交叉编译立刻变成负担（`scope.md` §6）。

## 非目标

- 口令 → KDF → 库密钥（plan 0402）
- 四套池的 CRUD（plan 0403）
- 备份 / 导出（plan 0404）

## 判据（来自 ROADMAP，展开时必须变成可粘贴的验收命令）

- 用**错误口令打不开库**
- `.db` 文件里**搜不到明文密钥**

## 展开时要补

- [ ] 「## 步骤」：每步可独立验证（含 `cargo add` 的实际 feature 写法）
- [ ] 「## 验收命令」：可粘贴 + 预期输出（错误口令的报错形态要写出来）
- [ ] 与 plan 0400 的决策条目逐条对齐；不一致就以 ADR 为准并回来改本文件
- [ ] **先跑 [ADR-0002](../adr/0002-secret-storage.md) §7 的实测清单**（`cipher_settings` 的实际输出、
      错误口令与空口令的报错形态、导出 round-trip、`rekey` 后盐是否变化）—— 那 7 项现在都只是"预期"
