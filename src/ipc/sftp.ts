// SFTP（plan 0701 / 0702）—— 命令在前端这一侧的接线。
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
  type SftpInFlight,
  type SftpListing,
  type SftpOrigin,
  type SftpSide,
  type SftpSideInfo,
  type SftpSideState,
  type SftpSummary,
  type SftpTransfer,
  type SftpTransferState,
} from "./bindings";

export type {
  SftpEntry,
  SftpInFlight,
  SftpListing,
  SftpOrigin,
  SftpSide,
  SftpSideInfo,
  SftpSideState,
  SftpSummary,
  SftpTransfer,
  SftpTransferState,
};

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
      case "noSuchTransfer":
        return `没有编号为 ${detail.detail.id} 的传输（它可能已经结束了）`;
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

/**
 * 让某一侧连上**本机**或者池里那台主机（plan 0702 起一栏可以选「本机」）。
 *
 * 本机那一档没有连接、没有认证，也就没有会失败的地方 —— 它连上就是"这一栏可以用了"。
 */
export async function connectSftpSide(
  handle: number,
  side: SftpSide,
  origin: SftpOrigin,
): Promise<SftpSideInfo> {
  const result = await commands.sftpConnect(handle, side, origin);
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

/**
 * 把 `from` 那一侧的一个文件搬到 `to` 那一侧的某个路径上，返回传输的编号。
 *
 * ⚠️ 它**立刻返回**：搬运在后台跑，进度与结局要读 [`listSftpTransfers`]（或 `sftp` 探针）。
 * 这不是偷懒 —— 一条几十秒的命令会把 IPC 堵住，而"看得见进度"正是产品要的。
 */
export async function startSftpTransfer(
  handle: number,
  from: SftpSide,
  fromPath: string,
  to: SftpSide,
  toPath: string,
): Promise<number> {
  const result = await commands.sftpTransfer(handle, from, fromPath, to, toPath);
  if (result.status === "error") throw new SftpFailed(result.error);
  return result.data;
}

/**
 * 取消一次传输。
 *
 * ⚠️ 它也只推信号：临时文件是搬运那条任务删的。**"取消完成了"的判据是**
 * [`listSftpTransfers`] 里那条不再是 `running` —— 界面不得在它变之前就宣布已经取消。
 */
export async function cancelSftpTransfer(handle: number, id: number): Promise<void> {
  const result = await commands.sftpTransferCancel(handle, id);
  if (result.status === "error") throw new SftpFailed(result.error);
}

/** 这个会话发起过的传输（新的在前）。 */
export async function listSftpTransfers(handle: number): Promise<SftpTransfer[]> {
  const result = await commands.sftpTransfers(handle);
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
