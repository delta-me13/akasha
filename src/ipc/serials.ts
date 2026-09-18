// 串口的两个**只读入口**（plan 1101 的池 + plan 1102 的本机端口）—— 界面据此知道
// "有哪些可以打开"。
//
// | 入口 | 读什么 | 失败那一档 |
// |---|---|---|
// | `listSerials` | 库里的串口配置池（路径 + 五个参数） | 库锁着 / 库不可用 |
// | `listPorts` | 本机枚举到的端口 | 系统调用失败 |
//
// ⚠️ 两个错误类**分开**：一个说"先解锁"，一个说"这里列不出来"；而"列不出来"与"一条都没有"
// 也是两件事（空表是正常结果，`serial` 的 `enumerate` 写明了这一点）。
//
// ⚠️ 打开设备那条命令（`openSerialSession`）**不碰库** —— 它要的六个参数由这里读出来的行提供，
// 也可以由界面**手输**。所以"锁着"挡得住列表、挡不住用一个已知参数去开一个设备；
// 而"枚举失败"挡得住端口列表、也挡不住手输路径（`useState` 里的三条输入是并列的）。
//
// ⚠️ **枚举结果不代表可用**（问题 #150）：udev 报的 devnode 可能根本不在 `/dev` 下。
// 界面因此不得把 `listPorts` 的结果当成"可用端口"。

import { commands, type SerialEntry, type SerialIpcError, type SerialPort, type VaultError } from "./bindings";

/** 界面看得见的一条串口配置（池里的一行）。 */
export type { SerialEntry };

/** 界面看得见的一条本机端口（枚举结果里的一条）。 */
export type { SerialPort };

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

/** 读本机端口失败（枚举那一次系统调用挂了 —— 与"本机没有串口"是两件事）。 */
export class SerialPortsUnavailable extends Error {
  readonly detail: SerialIpcError;

  constructor(detail: SerialIpcError) {
    super(SerialPortsUnavailable.describe(detail));
    this.name = "SerialPortsUnavailable";
    this.detail = detail;
  }

  private static describe(detail: SerialIpcError): string {
    switch (detail.kind) {
      case "enumerate":
        // 这一档的下一步是**手输一条路径** —— 那句话必须跟着一起说，
        // 否则用户会把它读成"本机没有串口"。
        return `列不出本机端口：${detail.detail.message}（手输一条设备路径仍然可用）`;
      case "settings":
        return `串口参数不合法：${detail.detail.field} = ${detail.detail.value}`;
      case "open":
        return `串口打不开：${detail.detail.path}（${detail.detail.message}）`;
      case "internal":
        return `内部状态不可用：${detail.detail.message}`;
    }
  }
}

/** 本机枚举到的端口（按路径排序）。空表 = 系统认为本机没有串口，**不是**失败。 */
export async function listPorts(): Promise<SerialPort[]> {
  const result = await commands.serialPorts();
  if (result.status === "error") throw new SerialPortsUnavailable(result.error);
  return result.data;
}
