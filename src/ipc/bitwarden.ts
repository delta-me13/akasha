// Bitwarden 的入口（plan 0902 / 0905）：两个轴、运行时下载、登录 / 解锁 / 锁定 / 登出。
//
// 为什么这些命令住在 `src/ipc/`：前端唯一允许碰后端的目录（`AGENTS.md` §0 禁止 #1）。
//
// 这一层的职责只有两条：
//
// 1. **把 `kind` 与"用户该做什么"接起来**（`BwUnavailable.describe`）—— 界面按 `kind`
//    分辨，不匹配消息字符串；
// 2. **说清哪些是用户动作**：下载、切换、登录、解锁、锁定、登出全是显式动作，
//    没有任何一条会在后台自己发生（ADR-0007 D2 / plan 0905 的非目标）。
//
// ⚠️ session key **从来不过这一层**：它只在 Rust 进程的内存里（受保护页）。
// 界面能看到的只有 `hasSession` 这个布尔值（ADR-0007 D7）。

import { commands, type BwIpcError, type BwSnapshot } from "./bindings";

/** 界面看得见的一份完整读数（CLI 那一块 + 三态 + 有没有 session）。 */
export type { BwSnapshot, BwIpcError };

/** 两个轴的取值。**故意不用布尔**：`host` 与 `managed` 是两件不同的事，不是开关的两面。 */
export type BinarySource = "host" | "managed";
export type AppDataMode = "host" | "managed";

/** 一条 Bitwarden 命令失败。 */
export class BwUnavailable extends Error {
  readonly detail: BwIpcError;

  constructor(detail: BwIpcError) {
    super(BwUnavailable.describe(detail));
    this.name = "BwUnavailable";
    this.detail = detail;
  }

  get kind(): BwIpcError["kind"] {
    return this.detail.kind;
  }

  /**
   * `kind` → 用户该做的那件事。
   *
   * ⚠️ **`missingBinary` 与 `notInstalled` 是两句话**（ADR-0007 D5）：前者是"这台机器上
   * 没有 `bw`"，后者是"这一轴还没下载过"。把它们并成一句，用户就不知道自己是该装一个
   * 还是该点下载。
   *
   * 认不出的档（`commandFailed` 等）**保留 `bw` 自己的原话** —— 那是唯一能定位问题的东西，
   * 换成我们编的一句只会把人带偏。
   */
  private static describe(detail: BwIpcError): string {
    switch (detail.kind) {
      case "missingBinary":
        return "这台机器上没有 bw：PATH 里找不到它（可以改用运行时下载的那一份）";
      case "notInstalled":
        return "还没下载过 bw：先把它下下来";
      case "notRunnable":
        return `找到的那份 bw 跑不起来：${detail.message}`;
      case "unsupportedTarget":
        return `上游没有这个平台 / 架构的资产：${detail.message}`;
      case "noRelease":
        return `读不出上游的版本：${detail.message}`;
      case "network":
        return `网络失败：${detail.message}`;
      case "tls":
        return `TLS 信任失败：${detail.message}`;
      case "insecureUrl":
        return `服务器地址必须是 https：${detail.message}`;
      case "invalidServer":
        return `服务器地址不合法：${detail.message}`;
      case "notLoggedIn":
        return "还没有登录这个 vault：先登录（登录过就先解锁）";
      case "timeout":
        return `bw 没有在期限内结束：${detail.message}`;
      case "protectedPage":
        return `session key 放不进受保护页：${detail.message}`;
      default:
        // `commandFailed` / `parse` / `archive` / `io` / `internal`：**原话照抄**。
        return detail.message;
    }
  }
}

function guard(result: { status: "ok"; data: BwSnapshot } | { status: "error"; error: BwIpcError }): BwSnapshot {
  if (result.status === "error") throw new BwUnavailable(result.error);
  return result.data;
}

/** 读一次 CLI 与状态（面板打开时用）。 */
export async function bwStatus(): Promise<BwSnapshot> {
  return commands.bwCliStatus();
}

/** 换两个轴。**只改选择并记下来**，不下载。 */
export async function setBwCli(binary: BinarySource, appdata: AppDataMode): Promise<BwSnapshot> {
  return guard(await commands.bwCliSettings(binary, appdata));
}

/** 下载一份 OSS 变体并改用它（约 45 MB，唯一一条会联网很久的命令）。 */
export async function installBwCli(): Promise<BwSnapshot> {
  return guard(await commands.bwCliInstall());
}

/** 设自托管服务器地址。返回**读回来的**生效值（上游会自己补前缀）。 */
export async function setBwServer(url: string): Promise<BwSnapshot> {
  return guard(await commands.bwServerSet(url));
}

/** 登录（邮箱 + 主密码；两步验证可选）。 */
export async function bwLogin(
  email: string,
  password: string,
  twoFactor?: { method: string; code: string },
): Promise<BwSnapshot> {
  return guard(
    await commands.bwLogin(email, password, twoFactor?.method ?? null, twoFactor?.code ?? null),
  );
}

/** 解锁（会换掉手上那份 session key）。 */
export async function bwUnlock(password: string): Promise<BwSnapshot> {
  return guard(await commands.bwUnlock(password));
}

/** 锁定：CLI 那一侧的 key 失效，内存里那份也没了。 */
export async function bwLock(): Promise<BwSnapshot> {
  return guard(await commands.bwLock());
}

/** 登出：连登录态一起清掉。 */
export async function bwLogout(): Promise<BwSnapshot> {
  return guard(await commands.bwLogout());
}
