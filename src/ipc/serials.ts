// serial 配置池的读取（plan 1101）—— 界面据此列出"有哪些串口可以打开"。
//
// 只读：串口配置池的增删改查界面与主机池的那个缺口是同一批（尚未规划的界面工作）。
// ⚠️ 打开设备那条命令（`openSerialSession`）**不碰库** —— 它要的六个参数由这里读出来的行提供。
// 所以"锁着"只挡得住**列表**，挡不住"用已经知道的参数去开一个设备"。

import { commands, type SerialEntry, type VaultError } from "./bindings";

/** 界面看得见的一条串口配置（池里的一行）。 */
export type { SerialEntry };

/** 读串口配置池失败。 */
export class SerialsUnavailable extends Error {
  readonly detail: VaultError;

  constructor(detail: VaultError) {
    super(SerialsUnavailable.describe(detail));
    this.name = "SerialsUnavailable";
    this.detail = detail;
  }

  /** 库没解锁 —— 界面该说的是"先解锁"，而不是"一条配置都没有"。 */
  get isLocked(): boolean {
    return this.detail.kind === "locked";
  }

  private static describe(detail: VaultError): string {
    if (detail.kind === "locked") return "库是锁着的：先解锁才能列出串口配置（池在库里）";
    // "库不可用"那几个变体里只有一部分带 `message`（同 `hosts.ts` 的理由：列一遍就会漏一个）。
    if ("detail" in detail && "message" in detail.detail) return `库不可用：${detail.detail.message}`;
    return `库不可用：${detail.kind}`;
  }
}

/** 库里 serial 配置池的全部行（按名字排序）。库锁着 → [`SerialsUnavailable::isLocked`]。 */
export async function listSerials(): Promise<SerialEntry[]> {
  const result = await commands.vaultSerials();
  if (result.status === "error") throw new SerialsUnavailable(result.error);
  return result.data;
}
