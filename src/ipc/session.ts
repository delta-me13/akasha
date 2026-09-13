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
import { commands, events, type IpcError, type SshIpcError } from "./bindings";

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
 * 一个终端标签页要开的**载体**。
 *
 * 本地与 SSH 走同一条尾巴（`adoptSession` + 同一批命令），差的是"哪条命令去开"：
 * 所以这里只是**选择**，不是两套会话模型 —— 后端那边同样只是两种 `Transport`
 * 装进同一张表（ADR-0003 D3）。
 */
export type SessionTarget =
  | { readonly kind: "local" }
  | { readonly kind: "ssh"; readonly hostId: number };

/**
 * 把"后端已经开好了"这件事包成一个可控的会话。
 *
 * 本地终端与 SSH 共用它：句柄、`session_ended` 的订阅、结束后一律不再发命令 —— 这三件事
 * 抄第二份的下场是两条路对"会话已经走了"有不同判断（一条静默、一条报错）。
 */
export function adoptSession(handle: number, onEnded: () => void): TerminalSession {
  // 会话结束之后**一律不再发命令**：后端已经把它摘牌收掉了，再发只会收到 `NotFound` ——
  // 那不是错误，是"它已经走了"。所以这里记住这件事，让 write / resize / close 变成空操作。
  let ended = false;
  const unsubscribe = subscribeSessionEnded(handle, () => {
    if (ended) return;
    ended = true;
    onEnded();
  });

  return {
    handle,
    async write(bytes: Uint8Array) {
      if (ended) return;
      const result = await commands.writeSession(handle, Array.from(bytes));
      if (result.status === "error") throw new IpcInvokeError(result.error);
    },
    async resize(cols: number, rows: number) {
      if (ended) return;
      const result = await commands.resizeSession(handle, cols, rows);
      if (result.status === "error") throw new IpcInvokeError(result.error);
    },
    async close() {
      unsubscribe();
      if (ended) return; // 后端已经收过尾了：再发 close_session 只会收到 NotFound
      const result = await commands.closeSession(handle);
      if (result.status === "error") throw new IpcInvokeError(result.error);
    },
  };
}

// ── 会话**自己**结束（plan 0306）──────────────────────────────────────────────
//
// 用户在终端里敲 `exit`、shell 崩了、PTY 被关掉 —— 这些都是"会话自己走了"，
// 与"用户关掉标签页"（plan 0305）方向相反。后端收掉它之后发一个事件，前端据此关掉
// 那个标签页（标签页与会话**同生命期**，`docs/scope.md` §5.6）。
//
// 为什么这件事必须由**事件**来说：官方 `Channel` 收到收尾帧（`{index, end:true}`）时
// 只把回调注销掉（`cleanupCallback`），**不会**通知 `onmessage` —— 前端光看字节通道
// 是看不出"流结束了"的。

/** 已经结束、但还没有人订阅的会话。见 [`subscribeSessionEnded`] 里的窄窗口说明。 */
const endedBeforeSubscribe = new Set<number>();

/** `handle → 关心它结束的回调`。 */
const endedHandlers = new Map<number, Set<() => void>>();

let listenerStarted = false;

/**
 * 起**一个**全局监听，按 `handle` 分发给订阅者。
 *
 * 为什么不让每个面板各 `listen` 一次：事件的寻址是"哪个**会话**"，不是"哪个面板"——
 * 分发规则只该有一处，否则将来加事件时每个面板都要重新对一遍。
 */
function ensureListening(): void {
  if (listenerStarted) return;
  listenerStarted = true;
  void events.sessionEnded.listen((event) => {
    const { handle } = event.payload;
    const handlers = endedHandlers.get(handle);
    if (!handlers || handlers.size === 0) {
      // 窄窗口：会话一开起来就立刻结束（shell 起不来就会这样），事件可能**早于**订阅到达。
      // 先记下来，订阅时补发 —— 否则那个标签页会留在界面上，里面是一个死终端。
      endedBeforeSubscribe.add(handle);
      return;
    }
    for (const handler of [...handlers]) handler();
  });
}

/**
 * 订阅"这个会话自己结束了"，返回退订函数。
 *
 * 事件若在订阅**之前**就到了（见 [`ensureListening`]），这里立刻回调一次。
 */
function subscribeSessionEnded(handle: number, handler: () => void): () => void {
  if (endedBeforeSubscribe.delete(handle)) {
    handler();
    return () => {};
  }
  let handlers = endedHandlers.get(handle);
  if (!handlers) {
    handlers = new Set();
    endedHandlers.set(handle, handlers);
  }
  handlers.add(handler);
  ensureListening();
  return () => {
    handlers.delete(handler);
    if (handlers.size === 0) endedHandlers.delete(handle);
  };
}

/**
 * 打开一个本地终端会话，输出**逐批**交给 `onBatch`。
 *
 * `onBatch` 拿到的是 `Uint8Array`，**没有解码**：字节流可能被切在任意多字节字符中间
 * （`AGENTS.md` §3.2），拼接与解码是解析层的事，不是这一层的事。
 */
export async function openTerminalSession(
  onBatch: (bytes: Uint8Array) => void,
  /** 这个会话**自己**结束了（敲 `exit` / shell 崩了）—— 壳层据此关掉它的标签页。 */
  onEnded: () => void,
): Promise<TerminalSession> {
  // 频道收的是 **ArrayBuffer**：后端发的是 `InvokeResponseBody::Raw`。
  // 若哪天有人把它改成 `Channel<Vec<u8>>`，这里收到的会变成 number[] ——
  // 那正是 `AGENTS.md` §3.2 禁止的"字节流被序列化成数组"。
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (payload) => onBatch(new Uint8Array(payload));

  const opened = await commands.openSession(channel.toJSON());
  if (opened.status === "error") throw new IpcInvokeError(opened.error);
  return adoptSession(opened.data, onEnded);
}

/**
 * SSH 那条路失败时的错误。
 *
 * 与 [`IpcInvokeError`] 分开：那一族说的是"某个会话坏了"，这一族说的是"连不上 / 不让连"
 * —— 前端能据此做的动作不同（后者里还有**警报**那一档）。
 */
export class SshInvokeError extends Error {
  readonly detail: SshIpcError;

  constructor(detail: SshIpcError) {
    super(SshInvokeError.describe(detail));
    this.name = "SshInvokeError";
    this.detail = detail;
  }

  /** 库没解锁：界面该说的是"先解锁"，而不是"连不上"。 */
  get isLocked(): boolean {
    return this.detail.kind === "locked";
  }

  /** **警报**：服务端的密钥与记下来的不一样（ADR-0003 D11）—— 这一档要显眼。 */
  get isHostKeyChanged(): boolean {
    return this.detail.kind === "failed" && this.detail.detail.kind === "hostKeyChanged";
  }

  private static describe(detail: SshIpcError): string {
    switch (detail.kind) {
      case "locked":
        return "库是锁着的：先解锁再开 SSH 会话（主机密钥记录与密钥池都在库里）";
      case "noSuchHost":
        return `主机池里没有这台主机（id ${detail.detail.id}）`;
      case "failed":
        // ⚠️ **警报那一档单独说**：主机密钥变了是中间人攻击的典型形态，
        // 让它与"连不上"长成一个样子，正是 D11 要避免的那种降级。
        return detail.detail.kind === "hostKeyChanged"
          ? `⚠️ 主机密钥变了，连接已拒绝：${detail.detail.message}`
          : detail.detail.message;
      case "internal":
        return `内部状态不可用：${detail.detail.message}`;
    }
  }
}

/**
 * 打开一个 **SSH** 终端会话，输出逐批交给 `onBatch`。
 *
 * 与上面那条的区别只有一处：后端**照主机池里的一行**去连（`hostId`），而在连的过程中
 * 它可能需要用户回答问题（凭据 / 没见过的主机密钥）—— 那些提示走
 * [`subscribePrompts`] 那条路，**不属于**这个会话（一个会话可能问好几次，也可能一次不问）。
 */
export async function openSshTerminalSession(
  hostId: number,
  onBatch: (bytes: Uint8Array) => void,
  onEnded: () => void,
): Promise<TerminalSession> {
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (payload) => onBatch(new Uint8Array(payload));

  const opened = await commands.openSshSession(channel.toJSON(), hostId);
  if (opened.status === "error") throw new SshInvokeError(opened.error);
  return adoptSession(opened.data, onEnded);
}
