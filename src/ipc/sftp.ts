// SFTP（plan 0701）—— 四条命令在前端这一侧的接线。
//
// 为什么在 `src/ipc/`：前端唯一允许碰后端的目录（`AGENTS.md` §0 禁止 #1）。
// 这一层**不做业务判断**：两侧的连接、失败落在哪一侧、目录内容都在后端
// （`ADR-0006` D3 / D7），这里只把"开 / 连 / 列 / 关"翻成调用，把失败翻成能读的句子。
//
// ⚠️ 与 `session.ts` 的区别同 `tunnels.ts`：SFTP 是一个**独立的 `Session`**，
// 它不随任何终端标签页关闭而死，也不需要先有一个终端。

import {
  commands,
  type SftpEntry,
  type SftpError,
  type SftpListing,
  type SftpSide,
  type SftpSideInfo,
  type SftpSideState,
  type SftpSummary,
} from "./bindings";

export type { SftpEntry, SftpListing, SftpSide, SftpSideInfo, SftpSideState, SftpSummary };

/** SFTP 操作失败。 */
export class SftpFailed extends Error {
  readonly detail: SftpError;

  constructor(detail: SftpError) {
    super(SftpFailed.describe(detail));
    this.name = "SftpFailed";
    this.detail = detail;
  }

  /** 库没解锁 —— 界面该说的是"先解锁"，而不是"连不上"。 */
  get isLocked(): boolean {
    return this.detail.kind === "locked";
  }

  /**
   * 主机密钥那一类警报（与后端的 `SshFailureKind` 同一个判据）。
   *
   * 界面据此把"要不要让用户去核对什么"分开 —— 而不是把错误消息拿去匹配字符串。
   */
  get isHostKeyAlert(): boolean {
    if (this.detail.kind !== "failed") return false;
    const kind = this.detail.detail.kind;
    return kind === "hostKeyChanged" || kind === "hostKeyRejected" || kind === "hostKeyUnknown";
  }

  private static describe(detail: SftpError): string {
    switch (detail.kind) {
      case "locked":
        return "库是锁着的：SFTP 要先解锁（要连的主机在库里）";
      case "noSuchHost":
        return `主机池里没有 id ${detail.detail.id} 这台主机`;
      case "notAnSftp":
        return `会话 ${detail.detail.handle} 不是一个 SFTP 会话（已关闭或未打开）`;
      case "notConnected":
        return `SFTP 的 ${detail.detail.side} 这一侧还没有连接（先连接，再列目录）`;
      case "failed":
        return `SFTP 连接失败：${detail.detail.message}`;
      case "internal":
        return `内部状态不可用：${detail.detail.message}`;
    }
  }
}

/** 登记一个新的 SFTP 会话（两侧都还没连），返回它的句柄。 */
export async function openSftp(): Promise<number> {
  const result = await commands.sftpOpen();
  if (result.status === "error") throw new SftpFailed(result.error);
  return result.data;
}

/** 让某一侧连上池里那台主机。 */
export async function connectSftpSide(
  handle: number,
  side: SftpSide,
  hostId: number,
): Promise<SftpSideInfo> {
  const result = await commands.sftpConnect(handle, side, hostId);
  if (result.status === "error") throw new SftpFailed(result.error);
  return result.data;
}

/** 列某一侧某个目录。`path` 传 `"."` = 从默认目录（服务端 `realpath` 的结果）开始。 */
export async function listSftp(
  handle: number,
  side: SftpSide,
  path: string,
): Promise<SftpListing> {
  const result = await commands.sftpList(handle, side, path);
  if (result.status === "error") throw new SftpFailed(result.error);
  return result.data;
}

/** 读两侧的状态（界面刷新用）。 */
export async function sftpSides(handle: number): Promise<SftpSideInfo[]> {
  const result = await commands.sftpSides(handle);
  if (result.status === "error") throw new SftpFailed(result.error);
  return result.data;
}

/**
 * 后端已经登记的全部 SFTP 会话。
 *
 * ⚠️ 关面板**不等于**结束会话（`docs/scope.md` §5.6）：面板重新打开时靠它接回那个会话，
 * 否则会话还在而没有任何地方能操作它。
 */
export async function listSftpSessions(): Promise<SftpSummary[]> {
  const result = await commands.sftpSessions();
  if (result.status === "error") throw new SftpFailed(result.error);
  return result.data;
}

/**
 * 关掉这个 SFTP 会话：两侧连接断开，会话从后端注销。
 *
 * ⚠️ 它是这个会话**唯一的关闭入口**：面板是仅渲染的视图（`docs/scope.md` §5.6），
 * 关面板不停后端会话。幂等：后端对已经关过的句柄返回成功。
 */
export async function closeSftp(handle: number): Promise<void> {
  const result = await commands.sftpClose(handle);
  if (result.status === "error") throw new SftpFailed(result.error);
}
