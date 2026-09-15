// dev-web 的**本地模拟后端**（plan 0203）。
//
// 只在浏览器里跑 `just dev-web` 时装上：没有 app、没有 PTY，但终端 UI 仍然要能迭代
// —— 字节从哪来？这里造一个最小的行编辑 shell：回显按键、认四个内置命令，
// `big` 还会灌一大坨输出，用来在**不开 app** 的情况下观察写入队列与内存。
//
// ⚠️ 三条约束：
//   1. **不进生产包**：`main.tsx` 只在 `import.meta.env.DEV && 没有 __TAURI_INTERNALS__`
//      时动态 import 本文件；生产构建里那段是常量假条件，连这个 chunk 一起被摇掉。
//   2. **命令名只写在 `src/ipc/` 里**（`AGENTS.md` §0：这是唯一允许碰后端的目录）。
//      名字与 Rust 侧重复是这一层的固有代价 —— 改名时两边一起改，别在别处再抄一份。
//   3. 它模拟的是**线上格式**：频道句柄是 `__CHANNEL__:<id>`，发出去的是 `ArrayBuffer`
//      （而不是 `number[]`）—— 于是浏览器里的 UI 走的解码路径与真后端完全一致。

import { mockIPC } from "@tauri-apps/api/mocks";

/** 与 `BatchPolicy::DEFAULT` 的 `max_bytes` 对齐：批次这个概念要一起模拟。 */
const BATCH_BYTES = 64 * 1024;

const PROMPT = "akasha-dev:~$ ";

interface Internals {
  runCallback?(channel: number, data: unknown): void;
}

function rawInternals(): Internals {
  return (globalThis as { __TAURI_INTERNALS__?: Internals }).__TAURI_INTERNALS__ ?? {};
}

/**
 * 取命令参数里的一个字段。
 *
 * `invoke` 的 payload 是个联合类型（对象 / 数组 / `ArrayBuffer`），所以取值前必须先收窄 ——
 * 直接下标访问在类型上就是错的（真后端永远传对象，但这一层不该假设）。
 */
function field(payload: unknown, key: string): unknown {
  if (!payload || typeof payload !== "object") return undefined;
  if (Array.isArray(payload) || ArrayBuffer.isView(payload) || payload instanceof ArrayBuffer) {
    return undefined;
  }
  return (payload as Record<string, unknown>)[key];
}

/** `__CHANNEL__:<id>` → `<id>`；形状不对就报错（与 Rust 侧同样不静默丢输出）。 */
function channelId(handle: unknown): number {
  const text = typeof handle === "string" ? handle : "";
  const id = Number.parseInt(text.slice("__CHANNEL__:".length), 10);
  if (!text.startsWith("__CHANNEL__:") || !Number.isInteger(id)) {
    throw new Error(`频道句柄无效：${String(handle)}`);
  }
  return id;
}

/** 一个假 shell：够用就好，它的唯一用途是让 UI 有字节可渲染。 */
class FakeShell {
  private readonly decoder = new TextDecoder();
  private readonly encoder = new TextEncoder();
  private index = 0;
  private line = "";
  private stopped = false;

  constructor(
    private readonly internals: Internals,
    private readonly channel: number,
  ) {}

  start(): void {
    this.emit(
      "akasha dev-web 模拟后端（没有 app、没有真 PTY）\r\n" +
        "内置命令：help / echo <文本> / big [MiB] / clear / exit\r\n",
    );
    this.emit(PROMPT);
  }

  stop(): void {
    this.stopped = true;
  }

  /** 用户按键：回显 + 行编辑。真实终端里回显来自 PTY 的 ECHO；这里是我们自己装的。 */
  write(bytes: Uint8Array): void {
    if (this.stopped) return;
    let echo = "";
    // 按 UTF-8 解码再一次成（多字节字符被按键切开的可能性在这里不重要）。
    for (const char of this.decoder.decode(bytes)) {
      switch (char) {
        case "\r":
        case "\n": {
          echo += "\r\n";
          const line = this.line;
          this.line = "";
          this.emit(echo);
          echo = "";
          this.run(line);
          break;
        }
        case "\x7f":
        case "\b": {
          if (this.line.length > 0) {
            this.line = this.line.slice(0, -1);
            echo += "\b \b";
          }
          break;
        }
        case "\x03": {
          this.line = "";
          echo += "^C\r\n";
          this.emit(echo);
          echo = "";
          this.emit(PROMPT);
          break;
        }
        default: {
          if (char >= " ") {
            this.line += char;
            echo += char;
          }
        }
      }
    }
    this.emit(echo);
  }

  private run(line: string): void {
    const [cmd, ...rest] = line.trim().split(/\s+/);
    const arg = rest.join(" ");
    switch (cmd) {
      case "":
        break;
      case "help":
        this.emit("help / echo <文本> / big [MiB] / clear / exit —— 这就是全部\r\n");
        break;
      case "echo":
        this.emit(`${arg}\r\n`);
        break;
      case "clear":
        this.emit("\x1b[2J\x1b[H");
        break;
      case "big":
        this.flood(Number.parseInt(arg, 10) > 0 ? Number.parseInt(arg, 10) : 8);
        break;
      case "exit":
        this.emit("模拟后端不会退出 —— 关掉这个标签页就行\r\n");
        break;
      default:
        this.emit(`sh: ${cmd}: command not found\r\n`);
    }
    if (cmd !== "clear") this.emit(PROMPT);
  }

  /** 分批发一大坨"看起来像终端输出"的字节 —— 一次 64 KiB，与真后端的批次同量级。 */
  private flood(mebibytes: number): void {
    const line = "akasha dev-web 模拟输出 0123456789 abcdefghijklmnopqrstuvwxyz 0123456789\r\n";
    const unit = this.encoder.encode(line);
    const chunk = new Uint8Array(BATCH_BYTES);
    for (let offset = 0; offset + unit.byteLength <= chunk.byteLength; offset += unit.byteLength) {
      chunk.set(unit, offset);
    }

    const total = mebibytes * 1024 * 1024;
    let sent = 0;
    const pump = () => {
      if (this.stopped || sent >= total) return;
      this.emitBytes(chunk);
      sent += chunk.byteLength;
      // 让出主线程：真后端由读线程按时间/容量合批，这里用宏任务模拟"不是一次灌完"。
      setTimeout(pump, 0);
    };
    pump();
  }

  private emit(text: string): void {
    this.emitBytes(this.encoder.encode(text));
  }

  /** 一批 = 一条频道消息；`index` 是 Channel 用来保序的（与真后端一致）。 */
  private emitBytes(bytes: Uint8Array): void {
    if (this.stopped || bytes.byteLength === 0) return;
    for (let offset = 0; offset < bytes.byteLength; offset += BATCH_BYTES) {
      const slice = bytes.slice(offset, offset + BATCH_BYTES);
      this.internals.runCallback?.(this.channel, { index: this.index, message: slice.buffer });
      this.index += 1;
    }
  }
}

/**
 * 装上模拟后端。**必须在渲染之前调用** —— 一旦有命令先跑了，
 * `window.__TAURI_INTERNALS__.invoke` 还是 tauri 缺失的那个，报错会很难懂。
 */
export function installDevBackend(): void {
  const internals = rawInternals();
  const shells = new Map<number, FakeShell>();
  let nextHandle = 1;

  mockIPC((cmd, payload) => {
    switch (cmd) {
      case "greet":
        return `dev-web 模拟后端：你好，${String(field(payload, "name"))}`;
      case "open_session": {
        const handle = nextHandle;
        nextHandle += 1;
        const shell = new FakeShell(internals, channelId(field(payload, "channel")));
        shells.set(handle, shell);
        shell.start();
        return handle;
      }
      case "write_session": {
        const handle = Number(field(payload, "handle"));
        const shell = shells.get(handle);
        if (!shell) throw new Error(`会话 ${handle} 不存在（已关闭或未打开）`);
        const data = field(payload, "data");
        shell.write(Uint8Array.from(Array.isArray(data) ? (data as number[]) : []));
        return null;
      }
      case "resize_session":
        // 模拟后端不需要尺寸；真载体在这里 SETSIZE（"尽力"语义）。
        return null;
      // SSH 那一路（plan 0504）在 dev-web 里**没有模拟后端**：没有 SSH 客户端、也没有库。
      // 两条命令都**明确报错**，而不是返回一个空列表 —— 后者会让"主机池是空的"与
      // "这个模拟后端不支持 SSH"混成一件事（前端会照着第一句去显示）。
      case "vault_hosts":
        throw { kind: "locked" };
      case "open_ssh_session":
        throw {
          kind: "failed",
          detail: { kind: "other", message: "dev-web 的模拟后端没有 SSH 客户端（用 just dev）" },
        };
      // SFTP 那一路（plan 0701 / 0702）同样**没有模拟后端**：没有 SFTP 客户端、也没有库。
      // 每一条都明确报错，而不是返回一个空列表 ——"目录是空的"与"这个模拟后端不支持 SFTP"
      // 是两件事，混在一起会让人以为连接成功了（同上一段的理由）。
      case "sftp_open":
      case "sftp_connect":
      case "sftp_list":
      case "sftp_transfer":
      case "sftp_transfer_cancel":
      case "sftp_transfers":
      case "sftp_sides":
      case "sftp_sessions":
      case "sftp_close":
        throw {
          kind: "failed",
          detail: { kind: "other", message: "dev-web 的模拟后端没有 SFTP 客户端（用 just dev）" },
        };
      case "close_session": {
        const handle = Number(field(payload, "handle"));
        shells.get(handle)?.stop();
        if (!shells.delete(handle)) throw new Error(`会话 ${handle} 不存在（已关闭或未打开）`);
        return null;
      }
      // dev-web 里没有 Rust 的事件系统。会话事件（`session_ended`）**不模拟** ——
      // 模拟后端不会自己退出（见 FakeShell 的 `exit`）。这两个分支只要让
      // `events.sessionEnded.listen(...)` 能成功返回，别让它抛在"打开一个终端"这条路上。
      case "plugin:event|listen":
        return 0;
      case "plugin:event|unlisten":
        return null;
      default:
        throw new Error(`dev-web 模拟后端没有实现命令 ${cmd}`);
    }
  });
}
