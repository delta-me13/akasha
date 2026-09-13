# CLAUDE.md

@AGENTS.md

> 上面这行是 Claude Code 的导入语法，等价于把 `AGENTS.md` 内联到这里 ——
> 目的是**两边读同一份规则，不复制内容**（复制出来的第二份必然漂移）。
> 其他工具直接读本文件或 [`AGENTS.md`](./AGENTS.md) 均可。

**本项目的完整规范在 [`AGENTS.md`](./AGENTS.md)。开始任何工作前先读它。**

`AGENTS.md` 是唯一规范入口（项目定位、开发循环与热重载真相、Rust/前端硬约束、
IPC 类型边界、ast-grep 结构护栏、测试与 DoD、依赖清单归属）。
本文件只保留指针与 Victauri 自动生成块，不再重复规范内容 —— 避免两处漂移。

现在到哪一步、有哪些已知问题 → `docs/STATUS.md`；下一步做什么 → `ROADMAP.md`。

需要立刻记住的少数几条：

- 常驻一个 `just dev`，不要每次手动 `cargo tauri dev`（Tauri 无 Rust 热重载）。
- 前端禁止裸 `invoke("...")`；Rust 侧业务逻辑放 `src-tauri/crates/`，`src-tauri` 只做 IPC 薄壳。
- 异步后端操作用 Victauri `wait_for` 等待，禁止 `sleep` 猜测。
- **提交前运行 `just ready`**（可执行的 DoD），外加 `AGENTS.md` §7 里机器查不了的两件事。

---

<!-- VICTAURI:BEGIN (added by `victauri init` — delete this block to opt out) -->
## Victauri (App Inspection & Testing)

This app has **Victauri** integrated — an MCP server embedded inside the Tauri process
that gives full-stack access to the webview DOM, IPC layer, Rust backend, and native
windows. Available when the app is running in debug mode.

**Use Victauri MCP tools for all app inspection and testing tasks.** Victauri runs inside
the app process with sub-ms response times and direct AppHandle access — it sees
everything, not just the webview.

Key Victauri tools (the read-only backend/DB introspection below has no CDP/Playwright
equivalent — and on macOS/Linux CDP can't attach to a Tauri webview at all):
- `invoke_command` — call any registered Tauri command directly (also records timing and
  honours fault injection; an eval-capable tool can also reach `__TAURI_INTERNALS__.invoke`)
- `verify_state` — cross-boundary frontend/backend state verification
- `detect_ghost_commands` — find frontend calls absent from the introspection registry (read the `reliability` field: only a real "no backend handler" bug when the registry mirrors the app's full command set; with no/partial `#[inspectable]` it lists real, uninstrumented commands)
- `check_ipc_integrity` — verify IPC pipeline health
- `introspect` — command timings, IPC contract testing, coverage, startup timing, capabilities
- `fault` — inject IPC faults (delay, error, drop, corrupt) for chaos engineering
- `explain` — natural-language narration of what happened in the app
- `get_memory_stats` — real OS process memory stats
- `audit_accessibility` — WCAG accessibility checks
- `get_performance` — navigation timing, JS heap, resource loading

### Connecting reliably (read this before reaching for CDP)

`.mcp.json` connects through `victauri bridge` — a stdio proxy that **discovers the running
app's port at connect time and re-discovers on restart**, so you are never pinned to a stale
or wrong port. Do **not** replace it with a fixed `"url": "http://127.0.0.1:7373/mcp"`: a
hardcoded port can point at a *different* Victauri app (e.g. a leftover demo) and every call
then fails with `422`/`404`. The bridge avoids that by design.

- **The server connects even before the app is running.** The bridge answers the MCP
  handshake itself, so `victauri` shows up connected in a fresh terminal whether or not the
  app is up. While the app is down the tool list is a static fallback and a tool *call*
  returns a clear "backend not reachable — start the app" error (not a hang). Start the app
  (`npm run tauri dev` / `pnpm tauri dev`) and the bridge connects automatically — tools go
  live with **no `/mcp` reconnect**. So if a call reports the backend unreachable, the fix is
  to start/await the app, not to switch to CDP.
- **First contact:** call `get_plugin_info` once and check `app.identifier` — confirm you
  reached the intended app, not another Victauri instance.
- **Multiple apps running?** Pin the bridge with `--app <bundle-identifier>` in `.mcp.json`
  (`"args": ["bridge", "--app", "com.your.app"]`), or set `VICTAURI_APP`. `init` bakes this in
  automatically when it can read your identifier.
- **If a tool call fails after an app rebuild/restart:** the bridge re-resolves the backend
  automatically — just retry (the transport is stateless by default, so there is usually no
  session to lose). If the MCP path is genuinely wedged, the **sessionless
  REST API is the fallback, NOT CDP**: `POST http://127.0.0.1:<port>/api/tools/<tool>` with
  the Bearer token from `<temp>/victauri/<pid>/token` (same capabilities, no session).

### Awaiting async backend work (don't guess with sleeps)

Many Tauri commands are **fire-and-forget**: they spawn background work and return
immediately (often `null`) while the real work runs. Don't poll by hand or sprinkle fixed
`sleep`s — use `wait_for` to await true completion:

- **Pollable status (no app changes):** `wait_for` with `condition: "expression"` evaluates a
  JS expression every poll until it is truthy (or equals `expected`). It may `await`, so you
  can await a status command directly:
  `wait_for { condition: "expression", value: "(await window.__TAURI_INTERNALS__.invoke('get_status')).running === false" }`.
  Level-triggered and race-free.
- **Completion event:** `wait_for` with `condition: "event"` blocks until a named Tauri event
  fires, with a `since_ms` look-back so an event emitted in the gap after your `invoke_command`
  is still caught: `wait_for { condition: "event", value: "analysis-complete" }`. (Custom events
  must be registered via `VictauriBuilder::listen_events(&["…"])`.)

The robust pattern is `invoke_command(...)` then `wait_for(expression|event, ...)` — never a bare sleep.

### Reading app-specific backend state

If the app registers state probes (`VictauriBuilder::probe("name", || json!({...}))`), call
`app_state` to read domain state directly from the Rust process — no IPC round-trip, no log
grepping. `app_state` with no args lists probe names; `app_state { probe: "name" }` returns its
snapshot. Use this for pipeline/queue/cache internals (version, depth, stats) instead of
reverse-engineering them from `query_db` + logs.

### Driving specific code paths & mutating test state

- To exercise a specific backend code path, call the relevant command via `invoke_command`
  (with args). If the path you need isn't reachable from any command, that's an app gap — add a
  small debug command and drive it.
- `query_db` is intentionally **read-only**. To mutate state for a test, go through the app's
  own commands with `invoke_command` (which respects app invariants) rather than writing the DB.

Prefer Victauri over Playwright or CDP for any Tauri-app task it handles; fall back to
Playwright only for browser-only work unrelated to this app.
<!-- VICTAURI:END -->
