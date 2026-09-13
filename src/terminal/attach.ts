// 把 IPC 会话接到终端渲染面上 —— **命令式**接线，React 只拿状态回调。
//
// 字节流在这里从"批次"直接进 `surface.write`，**从不经过 React state**
// （`AGENTS.md` §4.2）。这个文件是"不进 state"这条纪律的落点：它在 React 之外，
// 用闭包持有会话与渲染面，`TerminalPane` 只负责挂载/卸载它。

import {
  openSshTerminalSession,
  openTerminalSession,
  type SessionTarget,
  type TerminalSession,
} from "../ipc/session";
import {
  describeError,
  mountTerminalSurface,
  type RendererKind,
  type TerminalSurface,
} from "./surface";

export type TerminalStatus = "connecting" | "open" | "error";

export interface TerminalHandlers {
  onStatus(status: TerminalStatus, detail?: string): void;
  onRenderer(kind: RendererKind): void;
  /**
   * 会话**自己**结束了（终端里敲了 `exit` / shell 崩了）：壳层据此关掉这个标签页。
   *
   * ⚠️ 它**不是**"用户关了标签页"那条路（0305）—— 那条路走的是下面的清理函数。
   */
  onEnded(): void;
}

/**
 * 挂载终端并打开一个会话，返回 **detach**（正是 React effect 的清理函数）。
 *
 * 为什么是"返回函数"而不是"返回对象"：React 19 的 effect 清理就是 `() => void`，
 * 这里不做多余的包装，调用方也不必再写一层闭包。
 *
 * `target` 决定**哪条命令去开会话**（本地 shell / 主机池里的一台）：两条路的字节进出、
 * 尺寸、收尾完全一样（后端那边也只是两种 `Transport` 装进同一张表）。
 */
export function attachTerminal(
  host: HTMLElement,
  target: SessionTarget,
  handlers: TerminalHandlers,
): () => void {
  let disposed = false;
  let session: TerminalSession | null = null;
  let frame = 0;
  let forceResize = false;
  let lastCols = 0;
  let lastRows = 0;
  const encoder = new TextEncoder();

  const fail = (err: unknown) => {
    if (disposed) return;
    handlers.onStatus("error", describeError(err));
  };

  // 挂载失败**不能**把异常放出去：它抛在 React effect 里，React 会卸载整棵树 ——
  // 用户看到的是白屏，而不是"哪里坏了"。所以这里折成一句可读的报错交给壳层显示。
  let surface: TerminalSurface;
  try {
    surface = mountTerminalSurface(host, {
      onInput(data) {
        // 会话还没开起来时的按键直接丢掉：缓冲一段"半连接"的输入只会让语义变模糊。
        if (!session) return;
        void session.write(encoder.encode(data)).catch(fail);
      },
      onRenderer: handlers.onRenderer,
      onRendererUnavailable(message) {
        if (!disposed) handlers.onStatus("error", message);
      },
    });
  } catch (err) {
    handlers.onStatus("error", `终端初始化失败：${describeError(err)}`);
    return () => {};
  }

  /**
   * 尺寸变化 → `fit()` → 把新行列数回传载体（`resize` 是"尽力"语义）。
   *
   * `ResizeObserver` 会连发，所以按帧合并；`force` 用于"会话刚开"这一次 ——
   * 载体是按默认尺寸起的，即使算出来的行列数和上一次一样也必须补一发。
   */
  const refit = (force = false) => {
    forceResize ||= force;
    if (disposed || frame !== 0) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      const mustSend = forceResize;
      forceResize = false;
      if (disposed) return;
      const size = surface.fit();
      if (!size) return; // 宿主还没布局好，等下一次观察
      if (!mustSend && size.cols === lastCols && size.rows === lastRows) return;
      lastCols = size.cols;
      lastRows = size.rows;
      if (!session) return; // 会话还没开：开完会 force 一发
      void session.resize(size.cols, size.rows).catch(fail);
    });
  };

  const observer = new ResizeObserver(() => refit());
  observer.observe(host);
  refit(); // 首帧也算一次：初始尺寸可能就是最终尺寸

  handlers.onStatus("connecting");
  // 会话自己结束 → 交回壳层（关标签页）。`disposed` 之后不再回调：那时这个面已经没人要了，
  // 而"关标签页"会由清理函数负责。
  const onEnded = () => {
    if (!disposed) handlers.onEnded();
  };

  /**
   * 真的去开那个会话。
   *
   * ⚠️ **SSH 这条路要推迟一个微任务**再发出去，本地那条不用 —— 差别在"开的过程中要不要问人"：
   *
   * React 的 StrictMode（dev 构建）把 effect 走两遍（挂载 → 清理 → 挂载，**同一次 commit
   * 里同步完成**）。本地终端多起一个 shell 再立刻收掉是无害的，而 SSH 会话在连的过程中会
   * **弹出提示**（凭据 / 没见过的主机密钥）—— 被丢掉的那一次如果已经发出去了，它的问题会与
   * 真正那次**叠在同一个面板里**，而用户只会答其中一个：另一个一直等（最长
   * `PROMPT_TIMEOUT`），于是"点了 SSH 却一直连不上"。
   *
   * 同一次 commit 里排的微任务在清理**之后**才跑，所以第一遍那次根本没发出去。
   */
  const start = () =>
    target.kind === "ssh"
      ? openSshTerminalSession(target.hostId, (bytes) => surface.write(bytes), onEnded)
      : openTerminalSession((bytes) => surface.write(bytes), onEnded);

  const begin = () => {
    if (disposed) return;
    void start()
      .then((opened) => {
        if (disposed) {
          // 这一次会话已经没人要了（面板被卸载 / StrictMode 的第一次挂载）—— 不显式关掉的
          // 话，每挂载一次就多留一个真会话在进程里。
          void opened.close().catch(() => {});
          return;
        }
        session = opened;
        handlers.onStatus("open");
        refit(true); // 把真实行列数补给载体（开会话时它只知道默认尺寸）
      })
      .catch(fail);
  };

  if (target.kind === "ssh") queueMicrotask(begin);
  else begin();

  return () => {
    if (disposed) return;
    disposed = true;
    if (frame !== 0) cancelAnimationFrame(frame);
    observer.disconnect();

    // ⚠️ **顺序是有意的：先关会话，再拆渲染面。**
    //
    // 这个清理函数就是"关闭标签页 → 丢弃 Session"那个动作的落点（plan 0305）。而
    // `surface.dispose()` 里跑的是 xterm 与它的 addon —— 第三方代码**会抛**
    // （实测：丢过 WebGL 上下文的终端在 dispose 时抛 TypeError）。顺序反过来写的话，
    // 渲染器一抛，**会话回收那一句就永远执行不到**：标签页从界面上消失了，PTY 与
    // 里面的作业却留在用户机器上。
    //
    // 关闭是后端的"显式 kill + 收尸"，这里只发命令、不假装等它结束。
    // 即使有批次随后到达，`surface.write` 已经在 `disposed` 之后直接返回。
    const closing = session;
    session = null;
    if (closing) void closing.close().catch(() => {});

    surface.dispose();
  };
}
