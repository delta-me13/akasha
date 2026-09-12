// 会话接收层 —— 前端唯一手写的 IPC 包装（`src/ipc/` 是唯一允许碰后端的目录）。
//
// 为什么这一层手写而不是全靠生成器：命令 `open_session` 的参数是**频道句柄**，
// 它在线上是一个字符串（`__CHANNEL__:<id>`），而 Rust 侧真正发出去的是 **raw 字节**。
// 生成器只能照 Rust 的签名写出 `channel: string`，没法表达"往里发 ArrayBuffer"这件事 ——
// 于是"构造频道 + 把 ArrayBuffer 变成字节"这段就留在这里，其余签名仍然来自生成物。
//
// ⚠️ 两条纪律：
//   1. **字节不进 React state**（`AGENTS.md` §4.2）：`onBatch` 是命令式回调，
//      由调用方（0203 的 xterm 写入路径）直接消费；
//   2. 除了这里，前端不许在别处出现裸 `invoke("字符串命令名")`（`AGENTS.md` §0 绝对禁止 #1）。

import { Channel } from "@tauri-apps/api/core";
import { commands, type IpcError } from "./bindings";

/** 后端返回的错误，原样带着 [`IpcError`] 的结构抛出（调用方多半只是显示它）。 */
export class IpcInvokeError extends Error {
  readonly detail: IpcError;

  constructor(detail: IpcError) {
    super(IpcInvokeError.describe(detail));
    this.name = "IpcInvokeError";
    this.detail = detail;
  }

  private static describe(detail: IpcError): string {
    switch (detail.kind) {
      case "notFound":
        return `会话 ${detail.detail.handle} 不存在（已关闭或未打开）`;
      case "transport":
        return `载体失败：${detail.detail.message}`;
      case "channel":
        return `频道句柄无效：${detail.detail.message}`;
      case "internal":
        return `内部状态不可用：${detail.detail.message}`;
    }
  }
}

/** 打开会话的结果：一个可控的终端会话。 */
export interface TerminalSession {
  /** 过 IPC 的会话句柄 —— 后续 write / resize / close 都要带它。 */
  readonly handle: number;
  /** 送用户按键进载体。一次按键只有几个字节，走普通 JSON 参数即可。 */
  write(bytes: Uint8Array): Promise<void>;
  /** 调整窗口尺寸（"尽力"语义：载体可以拒绝）。 */
  resize(cols: number, rows: number): Promise<void>;
  /** 关闭会话：后端会显式 kill + wait 收尸。 */
  close(): Promise<void>;
}

/**
 * 打开一个本地终端会话，输出**逐批**交给 `onBatch`。
 *
 * `onBatch` 拿到的是 `Uint8Array`，**没有解码**：字节流可能被切在任意多字节字符中间
 * （`AGENTS.md` §3.2），拼接与解码是解析层的事，不是这一层的事。
 */
export async function openTerminalSession(
  onBatch: (bytes: Uint8Array) => void,
): Promise<TerminalSession> {
  // 频道收的是 **ArrayBuffer**：后端发的是 `InvokeResponseBody::Raw`。
  // 若哪天有人把它改成 `Channel<Vec<u8>>`，这里收到的会变成 number[] ——
  // 那正是 `AGENTS.md` §3.2 禁止的"字节流被序列化成数组"。
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (payload) => onBatch(new Uint8Array(payload));

  const opened = await commands.openSession(channel.toJSON());
  if (opened.status === "error") throw new IpcInvokeError(opened.error);
  const handle = opened.data;

  return {
    handle,
    async write(bytes: Uint8Array) {
      const result = await commands.writeSession(handle, Array.from(bytes));
      if (result.status === "error") throw new IpcInvokeError(result.error);
    },
    async resize(cols: number, rows: number) {
      const result = await commands.resizeSession(handle, cols, rows);
      if (result.status === "error") throw new IpcInvokeError(result.error);
    },
    async close() {
      const result = await commands.closeSession(handle);
      if (result.status === "error") throw new IpcInvokeError(result.error);
    },
  };
}
