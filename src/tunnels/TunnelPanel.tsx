// 隧道面板 —— **功能验证壳层**（`AGENTS.md` §4.0）。
//
// 它存在的理由只有一个：让"五态可观测、失败可见、手动重试"这些**后端行为**在界面上
// 能被看见、能被验证。它不是设计稿，也不构成产品约束 —— 正式界面重做时，这一块该删；
// 不该动的是它调的那三条命令与它接的那条事件。
//
// 两条纪律与 `session.ts` / `prompts.ts` 相同：
//   1. **状态是后端说了算**：这里保存的那份只是渲染用的副本，来源只有 `tunnel_state`
//      事件与那两条命令的返回值 —— 界面上不做任何状态推断；
//   2. 只有 `src/ipc/` 能碰后端（`AGENTS.md` §0 禁止 #1）。

import { useCallback, useEffect, useState } from "react";

import {
  TunnelFailed,
  listForwards,
  openTunnel,
  retryTunnel,
  stopTunnel,
  subscribeTunnelStates,
  type ForwardEntry,
  type TunnelStateChanged,
} from "../ipc/tunnels";

/**
 * 后端五态的中文文案。
 *
 * ⚠️ 键与后端 `TunnelStateName` 的取值一一对应（`connecting` / `connected` /
 * `reconnecting` / `failed` / `stopped`）—— 少一个键这里编译不过，
 * 而多写一个不存在状态同样编译不过（`Record` 是穷尽的）。
 */
const STATE_LABEL: Record<TunnelStateChanged["state"], string> = {
  connecting: "连接中",
  connected: "已连接",
  reconnecting: "重连中",
  failed: "失败",
  stopped: "已停止",
};

/** 一条隧道在界面上的最近一次状态。 */
interface Current {
  readonly state: TunnelStateChanged["state"];
  readonly attempt: number | null;
}

/**
 * 隧道面板：列出转发规则池里的规则，并驱动它们的打开 / 重试 / 停止。
 *
 * ⚠️ **规则池的增删改仍无界面**（plan 0601 的非目标）：这里只读、只驱动。
 */
export function TunnelPanel({ onClose }: { readonly onClose: () => void }) {
  const [rules, setRules] = useState<readonly ForwardEntry[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  /**
   * handle → 最近一次状态。
   *
   * 按 `handle` 存而不是按规则存：后端在 `tunnel_open` 返回**之前**就先发了 `连接中`
   * 那条事件，而那时界面还不知道 handle。按事件自带的 handle 存，事件就不会丢。
   */
  const [states, setStates] = useState<Readonly<Record<number, Current>>>({});
  /** 规则 id → handle（打开成功才知道）。 */
  const [handles, setHandles] = useState<Readonly<Record<number, number>>>({});
  /** 正在连接的那条规则：连接要几秒，按钮该灰住。 */
  const [busy, setBusy] = useState<number | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    listForwards()
      .then((rows) => {
        if (alive) setRules(rows);
      })
      .catch((err: unknown) => {
        if (alive) setProblem(describe(err));
      });
    const stop = subscribeTunnelStates((event) => {
      setStates((current) => ({
        ...current,
        [event.handle]: { state: event.state, attempt: event.attempt },
      }));
    });
    return () => {
      alive = false;
      stop();
    };
  }, []);

  /** 打开或重试一条隧道。`handle` 给了就是重试。 */
  const connect = useCallback(async (rule: ForwardEntry, handle?: number) => {
    setBusy(rule.id);
    setFailure(null);
    try {
      const result = handle === undefined ? await openTunnel(rule.id) : await retryTunnel(handle);
      setHandles((current) => ({ ...current, [rule.id]: result.handle }));
      // `failure` 非空 = 连不上，但这条**已经在册**（状态 `失败`）—— 界面上还能重试。
      if (result.failure) setFailure(new TunnelFailed(result.failure).message);
    } catch (err) {
      setFailure(describe(err));
    } finally {
      setBusy(null);
    }
  }, []);

  /** 停止：后端断开连接并注销，界面上把它退回"未打开"。 */
  const stop = useCallback(async (rule: ForwardEntry, handle: number) => {
    setFailure(null);
    try {
      await stopTunnel(handle);
      setHandles((current) => {
        const next = { ...current };
        delete next[rule.id];
        return next;
      });
    } catch (err) {
      setFailure(describe(err));
    }
  }, []);

  return (
    <section className="tunnel-panel" aria-label="隧道">
      <header className="tunnel-title">
        <span>隧道（端口转发）</span>
        <button
          type="button"
          className="tunnel-close"
          onClick={onClose}
          aria-label="关闭隧道面板"
        >
          ×
        </button>
      </header>

      {problem !== null && <p className="tunnel-problem">{problem}</p>}
      {failure !== null && <p className="tunnel-failure">{failure}</p>}
      {rules === null && problem === null && <p className="tunnel-empty">正在读转发规则池…</p>}
      {rules !== null && rules.length === 0 && (
        <p className="tunnel-empty">
          转发规则池是空的 —— 一条规则都没有时没有可打开的隧道（规则的增删改仍无界面）。
        </p>
      )}

      {rules !== null && rules.length > 0 && (
        <ul className="tunnel-list">
          {rules.map((rule) => {
            const handle = handles[rule.id];
            const current = handle === undefined ? undefined : states[handle];
            const state = current?.state ?? "connecting";
            const attempt = current?.attempt ?? null;
            return (
              <li key={rule.id} className="tunnel-item" data-rule-id={rule.id}>
                <span className="tunnel-name">{rule.name}</span>
                <span className="tunnel-route">{route(rule)}</span>
                {handle !== undefined && (
                  <span
                    className="tunnel-state"
                    data-tunnel-handle={handle}
                    data-tunnel-state={state}
                  >
                    {STATE_LABEL[state]}
                    {state === "reconnecting" && attempt !== null ? `（第 ${attempt} 次）` : ""}
                  </span>
                )}
                <span className="tunnel-actions">
                  {handle === undefined ? (
                    <button
                      type="button"
                      className="tunnel-open"
                      disabled={busy === rule.id}
                      onClick={() => void connect(rule)}
                    >
                      打开
                    </button>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="tunnel-retry"
                        disabled={busy === rule.id}
                        onClick={() => void connect(rule, handle)}
                      >
                        重试
                      </button>
                      <button
                        type="button"
                        className="tunnel-stop"
                        onClick={() => void stop(rule, handle)}
                      >
                        停止
                      </button>
                    </>
                  )}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

/** 一条规则要往哪儿转（给人看的一句话；方向决定哪一头是绑定）。 */
function route(rule: ForwardEntry): string {
  const bind = `${rule.bindHost}:${rule.bindPort}`;
  if (rule.direction === "dynamic") return `${bind} · SOCKS5`;
  const target = `${rule.targetHost ?? "?"}:${rule.targetPort ?? "?"}`;
  // `local`：本地监听 → 远端目标；`remote`：远端监听 → 本地目标。
  return rule.direction === "local" ? `${bind} → ${target}` : `${target} → ${bind}`;
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
