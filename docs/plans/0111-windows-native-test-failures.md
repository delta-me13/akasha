# Plan 0111: Windows 上原生执行的另两条红

- **关联**：ROADMAP 阶段 1 ·「Windows 上原生执行 `just test` 不再有红」
- **前置**：问题 #168；plan 0110（同一次发现里的第一条）
- **状态**：已完成

## 目标 / 非目标

**目标**：`cargo nextest run --workspace` 在 Windows 上不再有红 —— 这两条都是“只有原生执行
才会暴露”的差异，在 Linux 上永远是绿的。

**非目标**：

- 不改 SOCKS5 的协议行为：命令集、`REP` 分类与握手顺序一个字节都不动，改的只是**关闭的顺序**。
- 不改端点返回的路径形态：去掉 verbatim 前缀是 `ssh/local.rs` 的 `tidy` 刻意要做的
  （那是原生路径的转义写法，不是用户认得的路径），红的是对照物写错了。

## 前置检查

两条现象（本机 Windows，完整一遍 `cargo nextest run --workspace --no-fail-fast`）：

| 目标 | 现象 |
|---|---|
| `socks5_forward` | 读 `REP` 时拿到 `Os { code: 10054 }`（`ConnectionReset`），而不是那一个字节 |
| `transfer_atomic` | 列目录返回的路径没有 verbatim 前缀，而判据拿的是 `std::fs::canonicalize` 的原样输出 |

## 步骤（每步都能独立验证）

1. **SOCKS5：拒绝之后把对端剩下的字节读干净再关**。被拒的请求里可能还有没读的字节
   （不认的 `ATYP` 连长度都不知道），而接收缓冲非空的连接在 Windows 上关闭发的是 RST，
   RST 会丢掉已经排队、还没被对端读走的 `REP`。做法：写完 `REP` 关写半边，读到 EOF
   （上限 500 ms）再结束。验证：`cargo nextest run --test socks5_forward` 转绿。
2. **`transfer_atomic`：对照物改成“先规范化、再去前缀”**。端点返回的就是这个形态，
   判据要对照的是同一条规则。验证：`cargo nextest run --test transfer_atomic` 转绿。

## 验收命令

```bash
cd src-tauri && cargo nextest run --test socks5_forward --test transfer_atomic
cd src-tauri && cargo nextest run --workspace --no-fail-fast
```

预期：第一条 `7 tests run: 7 passed`；第二条 `423 tests run: 423 passed, 0 skipped`。

## 回滚

两处改动各自独立：`ssh/relay.rs` 的 `refuse_and_close` 改回直接 `return`（SOCKS5 会重新红），
`tests/transfer_atomic.rs` 的对照物改回 `std::fs::canonicalize`（Windows 上会重新红）。

## 实施记录

- 2026-09-20：`socks5_forward` 失败在 `read_reply` 的 `expect("读 REP 失败")`。
  根因是“不认的 `ATYP` 后面还有两个字节没读”加上 Windows 的 RST 语义。
  加了 `refuse_and_close`（回 `REP` → 关写半边 → 读到 EOF，上限 500 ms）之后转绿。
- 2026-09-20：`transfer_atomic` 的断言把端点的输出与 `std::fs::canonicalize` 直接比，
  而 Windows 的 `canonicalize` 给出 verbatim 形态。对照物改成 `canonical()`（同一条规则）之后转绿。
- 2026-09-20：全量 `cargo nextest run --workspace --no-fail-fast` = **423 条全过**
  （此前 423 条里 4 红；前两条见 plan 0110）。
