// 主机指纹面板 —— **功能验证壳层**（`AGENTS.md` §4.0）。
//
// 它存在的理由只有一个：让"我们记下了哪些主机密钥、删掉之后下次连接会不会重新问"这些
// **后端行为**在界面上能被看见、能被验证。它不是设计稿，也不构成产品约束。
//
// 三条纪律：
//   1. **只有看与删**：添加与修改发生在连接过程中（ADR-0003 D11）—— 这里没有"信任新密钥"
//      这类控件，密钥变化仍然由写路径拒绝（同键不同值 → `Conflict`）；
//   2. 列表只覆盖**我们库里的缓存**，用户的 `~/.ssh/known_hosts` 只读、不在列表里；
//   3. 只有 `src/ipc/` 能碰后端（`AGENTS.md` §0 禁止 #1）。

import { useCallback, useEffect, useState } from "react";

import {
  KnownHostsUnavailable,
  forgetKnownHost,
  listKnownHosts,
  type KnownHostEntry,
} from "../ipc/knownHosts";

/**
 * 主机指纹面板：列出库里记下的主机密钥，并允许逐行**遗忘**。
 *
 * 打开时读一次、删除之后刷新；另有一个"刷新"按钮 —— 这条需求不需要事件推送，
 * 新增一条事件就是新增一份契约（plan 0507 的非目标）。
 */
export function KnownHostsPanel({ onClose }: { readonly onClose: () => void }) {
  const [rows, setRows] = useState<readonly KnownHostEntry[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  /** 删除成功之后的那句话：它说的是"下一步会发生什么"（重新询问），不是"操作成功"。 */
  const [notice, setNotice] = useState<string | null>(null);
  /** 正在删的那一行：删除是本地事务，很快，但按钮该灰住。 */
  const [busy, setBusy] = useState<number | null>(null);

  const refresh = useCallback(async () => {
    try {
      setRows(await listKnownHosts());
      setProblem(null);
    } catch (err) {
      setProblem(describe(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const forget = useCallback(
    async (row: KnownHostEntry) => {
      setBusy(row.id);
      setNotice(null);
      try {
        await forgetKnownHost(row.id);
        setNotice(`已删除 ${row.host}:${row.port} —— 下次连接会重新询问`);
        await refresh();
      } catch (err) {
        setProblem(describe(err));
      } finally {
        setBusy(null);
      }
    },
    [refresh],
  );

  return (
    <section className="known-hosts-panel" aria-label="主机指纹">
      <header className="known-hosts-title">
        <span>主机指纹</span>
        <button
          type="button"
          className="known-hosts-refresh"
          onClick={() => void refresh()}
          aria-label="刷新主机指纹"
        >
          刷新
        </button>
        <button
          type="button"
          className="known-hosts-close"
          onClick={onClose}
          aria-label="关闭主机指纹面板"
        >
          ×
        </button>
      </header>

      <p className="known-hosts-hint">
        这里只有我们记下来的指纹（库里的缓存）；你自己的 ~/.ssh/known_hosts 只读，不在列表里。
        删除 = 遗忘：下次连接同一台会重新询问。密钥变化不由这里处理 —— 连接时它会拒绝，
        并把新旧两个指纹都显示出来。
      </p>

      {problem !== null && <p className="known-hosts-problem">{problem}</p>}
      {notice !== null && <p className="known-hosts-notice">{notice}</p>}
      {rows === null && problem === null && <p className="known-hosts-empty">正在读记下的主机密钥…</p>}
      {rows !== null && rows.length === 0 && (
        <p className="known-hosts-empty">
          还没有记下任何主机密钥 —— 连接一台没见过的机器并确认之后，这里会出现一行。
        </p>
      )}

      {rows !== null && rows.length > 0 && (
        <ul className="known-hosts-list">
          {rows.map((row) => (
            <li key={row.id} className="known-hosts-item" data-known-host-id={row.id}>
              <span className="known-hosts-names">
                {row.names.length > 0 ? row.names.join(" / ") : "主机池里没有这一台"}
              </span>
              <span className="known-hosts-route">
                {row.host}:{row.port} · {row.keyType}
              </span>
              <span
                className="known-hosts-fingerprint"
                data-known-host-fingerprint={row.fingerprint}
              >
                {row.fingerprint}
              </span>
              <button
                type="button"
                className="known-hosts-forget"
                disabled={busy === row.id}
                onClick={() => void forget(row)}
              >
                删除
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** 错误 → 界面上的那一句。`KnownHostsUnavailable` 自己已经写好了说法。 */
function describe(err: unknown): string {
  if (err instanceof KnownHostsUnavailable) return err.message;
  return err instanceof Error ? err.message : String(err);
}
