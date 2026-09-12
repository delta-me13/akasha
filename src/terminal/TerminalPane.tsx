import { useEffect, useRef, useState } from "react";
import { attachTerminal, type TerminalStatus } from "./attach";
import type { RendererKind } from "./surface";

interface Status {
  kind: TerminalStatus;
  detail?: string;
}

/**
 * 终端面板 —— React 在这里只做两件事：给一块 DOM 当宿主、显示**壳层**状态。
 *
 * **字节不进 state**（`AGENTS.md` §4.2）：输出的每一批都在 `attachTerminal` 的
 * 命令式闭包里直接进了 xterm 的写入缓冲。这个组件从 `attachTerminal` 拿到的
 * 只有"连接状态"和"渲染器种类"，没有任何一条路径会把终端字节交给 `setState`。
 */
export function TerminalPane() {
  const hostRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<Status>({ kind: "connecting" });
  const [renderer, setRenderer] = useState<RendererKind | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    // effect 的清理函数就是 detach：卸载时关会话、拆渲染面、断开尺寸观察。
    return attachTerminal(host, {
      onStatus: (kind, detail) => setStatus({ kind, detail }),
      onRenderer: setRenderer,
    });
  }, []);

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
