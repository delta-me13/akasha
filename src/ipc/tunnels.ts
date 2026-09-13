// 隧道（端口转发）—— plan 0601 的三条命令与状态事件在前端这一侧的接线。
//
// 为什么在 `src/ipc/`：前端唯一允许碰后端的目录（`AGENTS.md` §0 禁止 #1）。
// 这一层**不做业务判断**：状态机在后端（`akasha_core::TunnelState`，"哪些转移合法"在那里
// 定死），这里只把"打开 / 重试 / 停止"翻成三次调用，把事件翻成一个回调。
//
// ⚠️ 与 `session.ts` 的区别：隧道**不是**标签页。一条转发规则是一个独立的 `Session`
// （`docs/scope.md` §2.2），它不随任何终端标签页关闭而死，也不需要一个终端先存在。

import {
  commands,
  events,
  type ForwardEntry,
  type TunnelAttempt,
  type TunnelError,
  type TunnelStateChanged,
  type VaultError,
} from "./bindings";

export type { ForwardEntry, TunnelAttempt, TunnelStateChanged };

/** 读转发规则池失败（库锁着 / 库用不了）。 */
export class ForwardsUnavailable extends Error {
  readonly detail: VaultError;

  constructor(detail: VaultError) {
    super(ForwardsUnavailable.describe(detail));
    this.name = "ForwardsUnavailable";
    this.detail = detail;
  }

  /** 库没解锁 —— 界面该说的是"先解锁"，而不是"读不出来"。 */
  get isLocked(): boolean {
    return this.detail.kind === "locked";
  }

  private static describe(detail: VaultError): string {
    if (detail.kind === "locked") return "库是锁着的：转发规则在库里（先解锁）";
    if ("detail" in detail && "message" in detail.detail) return `库不可用：${detail.detail.message}`;
    return `库不可用：${detail.kind}`;
  }
}

/** 打开或重试隧道时的失败。 */
export class TunnelFailed extends Error {
  readonly detail: TunnelError;

  constructor(detail: TunnelError) {
    super(TunnelFailed.describe(detail));
    this.name = "TunnelFailed";
    this.detail = detail;
  }

  /**
   * 主机密钥那一类警报（与后端的 `SshFailureKind` 同一个判据）。
   *
   * 界面据此把"要不要让用户去核对什么"分开 —— 而不是把错误消息拿去匹配字符串
   * （那是最容易在改文案时静默失效的一种判据）。
   */
  get isHostKeyAlert(): boolean {
    if (this.detail.kind !== "failed") return false;
    const kind = this.detail.detail.kind;
    return kind === "hostKeyChanged" || kind === "hostKeyRejected" || kind === "hostKeyUnknown";
  }

  private static describe(detail: TunnelError): string {
    switch (detail.kind) {
      case "locked":
        return "库是锁着的：转发规则在库里（先解锁）";
      case "noSuchForward":
        return `转发规则池里没有这一条（id ${detail.detail.id}）`;
      case "noSuchHost":
        return `主机池里没有 id ${detail.detail.id} 这台主机`;
      case "notATunnel":
        return `会话 ${detail.detail.handle} 不是一条隧道（已停止或未打开）`;
      case "unsupported":
        return `这条规则的方向是 ${detail.detail.direction}：本版本只支持本地转发（local）`;
      case "bind":
        return `本地监听 ${detail.detail.address} 绑定失败：${detail.detail.message}`;
      case "failed":
        return `隧道连接失败：${detail.detail.message}`;
      case "transition":
        return `隧道状态转移被拒：${detail.detail.message}`;
      case "internal":
        return `内部状态不可用：${detail.detail.message}`;
    }
  }
}

/**
 * 一次"打开 / 重试"的结果是**生成物里的那个类型**（`bindings.ts` 的 `TunnelAttempt`）。
 *
 * ⚠️ 手写第二份同样的形状就是"前端自己造一份签名"的开端：`AGENTS.md` §5 只允许 Rust → TS
 * 一个方向。这里只做一次 `export`，不重新声明。
 */

/** 库里转发规则池的全部行。库锁着 → [`ForwardsUnavailable::isLocked`]。 */
export async function listForwards(): Promise<ForwardEntry[]> {
  const result = await commands.vaultForwards();
  if (result.status === "error") throw new ForwardsUnavailable(result.error);
  return result.data;
}

/** 打开一条隧道（按转发规则的行 id）。 */
export async function openTunnel(forwardId: number): Promise<TunnelAttempt> {
  const result = await commands.tunnelOpen(forwardId);
  if (result.status === "error") throw new TunnelFailed(result.error);
  return result.data;
}

/** 重试一条已经登记着的隧道（`失败 / 已停止 → 连接中`，尝试次数清零）。 */
export async function retryTunnel(handle: number): Promise<TunnelAttempt> {
  const result = await commands.tunnelRetry(handle);
  if (result.status === "error") throw new TunnelFailed(result.error);
  return result.data;
}

/** 停止一条隧道：断开连接并把它从后端注销。 */
export async function stopTunnel(handle: number): Promise<void> {
  const result = await commands.tunnelStop(handle);
  if (result.status === "error") throw new TunnelFailed(result.error);
}

/**
 * 订阅隧道状态事件，返回退订函数。
 *
 * ⚠️ 它同时把每一次状态变化记进 `window.__akashaTunnels` —— 那是**测试接口**
 * （`docs/scope.md` §1.3：探针可以有，但它们是测试的接口，不是 UI 规范）。
 * E2E 靠它断言"状态变化**真的发了事件**并到了前端"，而不是只看后端 probe 猜前端收没收到。
 */
export function subscribeTunnelStates(onChange: (event: TunnelStateChanged) => void): () => void {
  const stop = events.tunnelState.listen((event) => {
    recordTunnelEvent(event.payload);
    onChange(event.payload);
  });
  return () => {
    void stop.then((unlisten) => unlisten());
  };
}

/** 事件日志的类型（测试接口，见 [`subscribeTunnelStates`]）。 */
export interface TunnelEventLog {
  events: TunnelStateChanged[];
}

function recordTunnelEvent(event: TunnelStateChanged): void {
  const holder = window as Window & { __akashaTunnels?: TunnelEventLog };
  if (!holder.__akashaTunnels) holder.__akashaTunnels = { events: [] };
  holder.__akashaTunnels.events.push(event);
}
