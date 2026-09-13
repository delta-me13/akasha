// 主机池的**只读读取**（plan 0504）—— 界面凭什么把一台主机交给 SSH 那条路。
//
// 为什么这些命令住在 `src/ipc/`：前端唯一允许碰后端的目录（`AGENTS.md` §0 禁止 #1）。
// 为什么没有"新建 / 改 / 删"三条：那是**用户动作**，各有各的判据（重名、跳板成环、
// 删掉被引用的行怎么解释），属于仍未规划的界面工作 —— 本步只把库里已有的东西列出来，
// **不改任何状态**。

import { commands, type HostEntry, type VaultError } from "./bindings";

/** 界面看得见的一台主机（池里的一行）。 */
export type { HostEntry };

/** 读主机池失败。 */
export class HostsUnavailable extends Error {
  readonly detail: VaultError;

  constructor(detail: VaultError) {
    super(HostsUnavailable.describe(detail));
    this.name = "HostsUnavailable";
    this.detail = detail;
  }

  /** 库没解锁 —— 界面该说的是"先解锁"，而不是"读不出来"。 */
  get isLocked(): boolean {
    return this.detail.kind === "locked";
  }

  private static describe(detail: VaultError): string {
    if (detail.kind === "locked") return "库是锁着的：先解锁才能列出主机（池在库里）";
    // "库不可用"那几个变体里只有一部分带 `message`（例如 `passphraseTooLong` 带的是 `max`）
    // —— 所以这里按"带不带"取，而不是按 kind 逐个列举：列一遍就会漏一个。
    if ("detail" in detail && "message" in detail.detail) return `库不可用：${detail.detail.message}`;
    return `库不可用：${detail.kind}`;
  }
}

/** 库里主机池的全部行（按名字排序）。库锁着 → [`HostsUnavailable::isLocked`]。 */
export async function listHosts(): Promise<HostEntry[]> {
  const result = await commands.vaultHosts();
  if (result.status === "error") throw new HostsUnavailable(result.error);
  return result.data;
}
