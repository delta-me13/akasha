import { useEffect, useRef, useState } from "react";
import { attachTerminal, type TerminalStatus } from "./attach";
import { activateProbe, type RendererKind } from "./surface";

interface Status {
  kind: TerminalStatus;
  detail?: string;
}

interface TerminalPaneProps {
  /**
   * 这个面是不是**当前显示的那个**标签页。
   *
   * ⚠️ 它**不决定挂载**：所有标签页的面都保持挂载（卸载 = 关会话，见 `App.tsx`）。
   * 它的唯一用途是"用户切到这个标签页时把键盘焦点与验收探针交给它"。
   */
  readonly active: boolean;
  /**
   * 这个面的会话**自己**结束了（终端里敲了 `exit` / shell 崩了）。
   *
   * 壳层收到就关掉这个标签页 —— 标签页与会话同生命期（`docs/scope.md` §5.6）。
   */
  onSessionEnded(): void;
}

/**
 * 终端面板 —— React 在这里只做两件事：给一块 DOM 当宿主、显示**壳层**状态。
 *
 * **字节不进 state**（`AGENTS.md` §4.2）：输出的每一批都在 `attachTerminal` 的
 * 命令式闭包里直接进了 xterm 的写入缓冲。这个组件从 `attachTerminal` 拿到的
 * 只有"连接状态"和"渲染器种类"，没有任何一条路径会把终端字节交给 `setState`。
 */
export function TerminalPane({ active, onSessionEnded }: TerminalPaneProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<Status>({ kind: "connecting" });
  const [renderer, setRenderer] = useState<RendererKind | null>(null);

  // ⚠️ 会话结束的回调**必须走 ref**：`attachTerminal` 只在挂载时接一次线（依赖是 `[]`），
  // 而 `App` 每次渲染都会传一个新的箭头函数（它闭包着**当时**的标签页列表）。
  // 直接接那个函数的话，晚一点才结束的会话会走到一份过期闭包里 —— 表现是"标签页关不掉"。
  const endedRef = useRef(onSessionEnded);
  useEffect(() => {
    endedRef.current = onSessionEnded;
  }, [onSessionEnded]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    // effect 的清理函数就是 detach：卸载时关会话、拆渲染面、断开尺寸观察。
    return attachTerminal(host, {
      onStatus: (kind, detail) => setStatus({ kind, detail }),
      onRenderer: setRenderer,
      onEnded: () => endedRef.current(),
    });
  }, []);

  useEffect(() => {
    if (!active) return;
    const host = hostRef.current;
    if (!host) return;
    // 两件**只跟"当前是哪个标签页"有关**的事：
    //   1. 把键盘焦点交给 xterm 的输入框 —— 用户点标签页就是想接着敲键盘；
    //   2. 把验收探针挂成当前活动面（多标签之后"最后挂载的那个面"不再等于"正在看的那个面"）。
    // 两者都**不碰终端内容**：字节仍然只从 `attachTerminal` 的闭包进 xterm（§4.2）。
    activateProbe(host);
    host.querySelector<HTMLElement>(".xterm-helper-textarea")?.focus();
  }, [active]);

  return (
    <div className="terminal-pane">
      <div className="terminal-host" ref={hostRef} />
      {status.kind === "error" && (
        <p className="terminal-error" role="alert">
          终端不可用：{status.detail}
        </p>
      )}
      <footer className="terminal-status">
        <span>
          {status.kind === "open" ? "已连接" : status.kind === "connecting" ? "连接中…" : "出错"}
        </span>
        {/* 渲染器是**壳层**信息，进 state 无妨；终端字节永远不进。 */}
        <span>{renderer ? `渲染器 ${renderer}` : "探测渲染器…"}</span>
      </footer>
    </div>
  );
}
