// 记下来的主机密钥（known_hosts **缓存**）的读取与遗忘（plan 0507）。
//
// 为什么这些命令住在 `src/ipc/`：前端唯一允许碰后端的目录（`AGENTS.md` §0 禁止 #1）。
// 为什么只有"看"与"删"：按 ADR-0003 D11，**添加与修改只发生在连接过程中** ——
// 未知密钥由提问接受、密钥变化由写路径拒绝，界面不该有第二条路。

import { commands, type KnownHostEntry, type VaultError } from "./bindings";

/** 界面看得见的一把主机密钥（缓存里的一行）。 */
export type { KnownHostEntry };

/** 读 / 删记下的主机密钥失败。 */
export class KnownHostsUnavailable extends Error {
  readonly detail: VaultError;

  constructor(detail: VaultError) {
    super(KnownHostsUnavailable.describe(detail));
    this.name = "KnownHostsUnavailable";
    this.detail = detail;
  }

  /** 库没解锁 —— 界面该说的是"先解锁"，而不是"读不出来"。 */
  get isLocked(): boolean {
    return this.detail.kind === "locked";
  }

  private static describe(detail: VaultError): string {
    if (detail.kind === "locked") return "库是锁着的：先解锁才能列出记下的主机密钥（缓存也在库里）";
    // 与 `hosts.ts` 同一手法：带 `message` 的变体按"带不带"取，不逐个列举 kind。
    if ("detail" in detail && "message" in detail.detail) return `库不可用：${detail.detail.message}`;
    return `库不可用：${detail.kind}`;
  }
}

/** 库里记下的全部主机密钥（按 host / port / key_type 排序）。 */
export async function listKnownHosts(): Promise<KnownHostEntry[]> {
  const result = await commands.knownHostsList();
  if (result.status === "error") throw new KnownHostsUnavailable(result.error);
  return result.data;
}

/**
 * 忘掉一行 —— 下一次连同一台会**重新询问**。
 *
 * ⚠️ 这不是"信任新密钥"：界面上的删除只是**遗忘**；接受新密钥仍然只能在连接过程中
 * 确认（ADR-0003 D11）。
 */
export async function forgetKnownHost(id: number): Promise<void> {
  const result = await commands.knownHostsForget(id);
  if (result.status === "error") throw new KnownHostsUnavailable(result.error);
}
