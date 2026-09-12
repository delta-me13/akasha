// 终端**渲染面** —— xterm 与它的 addon 都住在这里，React 一行都看不见。
//
// 两条纪律（`AGENTS.md` §4.1 / §4.2）：
//   1. 渲染走 **WebGL**，不可用才退 **canvas**；两者都起不来就**明确报错**，
//      **不静默落到 DOM 渲染器** —— 后者在高吞吐下必卡，不作为默认。
//   2. `term.write` **全工程只有这一处调用点**：日后接背压只改这一个地方。

import { CanvasAddon } from "@xterm/addon-canvas";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { SerializeAddon } from "@xterm/addon-serialize";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

/** 实际在用的渲染器。`"none"` = WebGL 与 canvas 都没起来（此时一定会明确报错）。 */
export type RendererKind = "webgl" | "canvas";

/**
 * E2E 探针（**只在 dev 构建里存在**）。
 *
 * 为什么必须有它：canvas / WebGL 渲染器**不把文本放进 DOM** —— 屏幕内容只活在 xterm
 * 的缓冲里。没有探针，Victauri 就只能去猜 canvas 像素，而那种断言证明不了任何事
 * （`AGENTS.md` §7：观察内部状态要注册 probe，不要靠反推）。
 *
 * 生产构建里 `import.meta.env.DEV` 为 `false`，整个探针被摇掉。
 */
export interface TerminalProbe {
  readonly renderer: RendererKind | "none";
  /** 屏幕（含回滚）文本，默认最后 200 行。 */
  screenText(maxLines?: number): string;
  /** 还没被 xterm 消化掉的字节数 —— 归零才说明写入路径排空了。 */
  pendingBytes(): number;
  /** 收到过多少批。 */
  batches(): number;
}

declare global {
  interface Window {
    __akashaTerminal?: TerminalProbe;
  }
}

export interface SurfaceHooks {
  /** 用户按键。xterm 已经把它解码成字符串；编码成 UTF-8 字节是调用方的事。 */
  onInput(data: string): void;
  /** 渲染器选定 / 降级（只用于展示与验收）。 */
  onRenderer(kind: RendererKind): void;
  /** WebGL 与 canvas 都不可用 —— 只用于**明确报错**，不是兜底信号。 */
  onRendererUnavailable(message: string): void;
}

export interface TerminalSurface {
  readonly renderer: RendererKind | "none";
  write(bytes: Uint8Array): void;
  /** 按宿主尺寸重算行列数；宿主还没布局好时返回 `null`（"尽力"语义）。 */
  fit(): { cols: number; rows: number } | null;
  dispose(): void;
}

/** 把任何抛出物变成一句人能读的话（渲染器探测失败时要原样报给用户）。 */
export function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

const THEME = {
  background: "#101216",
  foreground: "#d8dee9",
  cursor: "#8fd3ff",
  selectionBackground: "#3b4a5a",
} as const;

export function mountTerminalSurface(host: HTMLElement, hooks: SurfaceHooks): TerminalSurface {
  const term = new Terminal({
    fontFamily: 'ui-monospace, SFMono-Regular, "JetBrains Mono", Menlo, Consolas, monospace',
    fontSize: 14,
    cursorBlink: true,
    scrollback: 10_000,
    theme: THEME,
    // ⚠️ 必须开：`addon-unicode11` 的 `term.unicode` 是 **proposed API**（宽字符按
    // 东半球宽度算的唯一入口）。不开的话 `loadAddon(Unicode11Addon)` 会抛，
    // 而它抛在 React effect 里 —— 症状是**整个界面白屏**，不是一句报错。
    allowProposedApi: true,
  });

  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  term.loadAddon(new Unicode11Addon());
  // 宽字符（CJK / emoji）按东半球宽度算 —— 少了它，中文与 emoji 会错位一格。
  term.unicode.activeVersion = "11";
  // 这两个先装上、暂不接 UI：搜索与序列化（会话恢复）是后续 plan 的入口。
  term.loadAddon(new SearchAddon());
  term.loadAddon(new SerializeAddon());

  term.onData((data) => hooks.onInput(data));

  // ⚠️ 顺序要紧：渲染器 addon 必须在 `open()` **之后**装载 —— 它们要往已挂载的
  // DOM 上贴 canvas，`open()` 之前 `term.element` 还不存在。
  term.open(host);
  term.focus();

  let renderer: RendererKind | "none" = "none";
  let disposed = false;
  let pendingBytes = 0;
  let batches = 0;

  function activate(kind: RendererKind): void {
    const addon = kind === "webgl" ? new WebglAddon() : new CanvasAddon();
    // 起不来就在这里抛：由调用方决定是降级还是报错，这里不吞。
    term.loadAddon(addon);
    renderer = kind;
    host.dataset.renderer = kind;
    hooks.onRenderer(kind);

    if (addon instanceof WebglAddon) {
      // 上下文丢失（驱动重启 / 显存不足）之后 WebGL 不再可用，唯一去处是 canvas。
      addon.onContextLoss(() => {
        try {
          addon.dispose();
        } catch {
          // 已经坏了的渲染器再 dispose 一次失败没有意义，也不该盖住真正的错误。
        }
        renderer = "none";
        host.dataset.renderer = "none";
        try {
          activate("canvas");
        } catch (err) {
          hooks.onRendererUnavailable(`WebGL 上下文丢失，canvas 回退也失败：${describeError(err)}`);
        }
      });
    }
  }

  try {
    activate("webgl");
  } catch (webglError) {
    try {
      activate("canvas");
    } catch (canvasError) {
      // 两条路都断了：把两边的**原始**原因都报出来，不要只说一句"渲染器不可用"。
      host.dataset.renderer = "none";
      hooks.onRendererUnavailable(
        `没有可用的画布渲染器 —— WebGL: ${describeError(webglError)}；canvas: ${describeError(canvasError)}`,
      );
    }
  }

  function write(bytes: Uint8Array): void {
    if (disposed || bytes.byteLength === 0) return;
    pendingBytes += bytes.byteLength;
    batches += 1;
    host.dataset.pending = String(pendingBytes);
    // ⚠️ 全工程**唯一**的 `term.write` 调用点（`AGENTS.md` §4.2 的背压入口）。
    // 回调在"这批已被解析并交给渲染器"之后触发 —— 它归零才说明输出真的消化完了。
    term.write(bytes, () => {
      pendingBytes -= bytes.byteLength;
      if (!disposed) host.dataset.pending = String(pendingBytes);
    });
  }

  function fit(): { cols: number; rows: number } | null {
    if (disposed) return null;
    // 先问"能不能算"再动手：宿主还没布局好时 `fit()` 会抛。
    const proposed = fitAddon.proposeDimensions();
    if (
      !proposed ||
      !Number.isFinite(proposed.cols) ||
      !Number.isFinite(proposed.rows) ||
      proposed.cols < 2 ||
      proposed.rows < 1
    ) {
      return null;
    }
    fitAddon.fit();
    return { cols: term.cols, rows: term.rows };
  }

  // 探针只在 dev 构建里存在：写成三元 + 返回清理函数，是为了让生产构建能把
  // 整个 `installProbe`（连同它的缓冲读取代码与 `window` 键名）摇掉 ——
  // 而不是留一个"永远不进的判断"和一对孤儿字符串。
  const clearProbe = import.meta.env.DEV
    ? installProbe(
        () => ({ renderer, term, disposed }),
        () => ({ pendingBytes, batches }),
      )
    : undefined;

  function dispose(): void {
    if (disposed) return;
    disposed = true;
    clearProbe?.();
    term.dispose();
  }

  return {
    get renderer() {
      return renderer;
    },
    write,
    fit,
    dispose,
  };
}

function installProbe(
  state: () => { renderer: RendererKind | "none"; term: Terminal; disposed: boolean },
  counters: () => { pendingBytes: number; batches: number },
): () => void {
  const probe: TerminalProbe = {
    get renderer() {
      return state().renderer;
    },
    screenText(maxLines = 200) {
      const { term, disposed } = state();
      if (disposed) return "";
      const buffer = term.buffer.active;
      const rows = term.rows;
      const first = Math.max(0, buffer.baseY + rows - maxLines);
      const last = buffer.baseY + rows;
      const lines: string[] = [];
      for (let y = first; y < last; y += 1) {
        const line = buffer.getLine(y);
        if (line) lines.push(line.translateToString(true));
      }
      return lines.join("\n");
    },
    pendingBytes: () => counters().pendingBytes,
    batches: () => counters().batches,
  };

  window.__akashaTerminal = probe;
  // 挂载期结束后由 `dispose` 调用：只摘掉**自己**那只探针（StrictMode 会挂两次）。
  return () => {
    if (window.__akashaTerminal === probe) delete window.__akashaTerminal;
  };
}
