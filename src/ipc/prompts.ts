// SSH 的**提示往返** —— 「后端问 → 前端答」在前端这一侧的接线（plan 0504）。
//
// 与 `session.ts` 分开是有意的：提示**不属于某一个会话** —— 它发生在"连上之前"，
// 而一次连接可能问好几次（先问主机密钥、再问口令），也可能一次不问。
// 把它挂在会话上会让"这个面板属于哪个标签页"变成一个没人答得上来的问题。
//
// ⚠️ 两条纪律与 `session.ts` 相同：
//   1. **答案不进 React state**（`AGENTS.md` §4.2）：口令是一个字符串参数，
//      提交之后立刻丢掉，不进任何 store / state；
//   2. 除了 `src/ipc/`，前端不许在别处出现裸 `invoke("字符串命令名")`（`AGENTS.md` §0 禁止 #1）。

import { commands, events, type PromptError, type PromptRequest } from "./bindings";

/** 一条提示：后端在连接中途要用户回答一件事（判别式是 `kind`）。 */
export type { PromptRequest };

/**
 * 订阅提示，返回退订函数。
 *
 * 两条事件都要接：`sshPrompt` 是"有一问"，`sshPromptDismissed` 是"这一问不用答了"
 * （超时被后端撤下）—— 少了后者，界面会一直显示一个早已不存在的提问，
 * 而用户点"确定"只会得到 `gone`。
 */
export function subscribePrompts(
  onAsk: (prompt: PromptRequest) => void,
  onDismiss: (id: number) => void,
): () => void {
  const stopAsk = events.sshPrompt.listen((event) => onAsk(event.payload));
  const stopDismiss = events.sshPromptDismissed.listen((event) => onDismiss(event.payload.id));
  return () => {
    void stopAsk.then((unlisten) => unlisten());
    void stopDismiss.then((unlisten) => unlisten());
  };
}

/** 回答一句秘密（口令 / 验证码 / 私钥口令）。 */
export async function answerPromptCredential(id: number, secret: string): Promise<void> {
  const result = await commands.sshPromptCredential(id, secret);
  if (result.status === "error") throw new PromptRejected(result.error);
}

/** 回答"这把没见过的主机密钥认不认"。 */
export async function answerPromptHostKey(id: number, accept: boolean): Promise<void> {
  const result = await commands.sshPromptHostKey(id, accept);
  if (result.status === "error") throw new PromptRejected(result.error);
}

/** 撤销这一问（用户关掉了面板）。**不是"接受"的另一种写法**：连接会以取消结束。 */
export async function cancelPrompt(id: number): Promise<void> {
  const result = await commands.sshPromptCancel(id);
  if (result.status === "error") throw new PromptRejected(result.error);
}

/**
 * 回答提问时的失败。
 *
 * `gone` **不是用户错误**：那一问已经超时被撤下（界面自己也收到了 `ssh_prompt_dismissed`），
 * 所以调用方对它的正确处置是"什么都不做" —— 别弹一个红框去解释一件已经过去的事。
 */
export class PromptRejected extends Error {
  readonly detail: PromptError;

  constructor(detail: PromptError) {
    super(PromptRejected.describe(detail));
    this.name = "PromptRejected";
    this.detail = detail;
  }

  /** 那一问已经不在了（超时被撤 / 已经答过）。 */
  get isGone(): boolean {
    return this.detail.kind === "gone";
  }

  private static describe(detail: PromptError): string {
    switch (detail.kind) {
      case "gone":
        return `提问 ${detail.detail.id} 已经不在了（超时或已作答）`;
      case "invalid":
        return `答案不合法：${detail.detail.message}`;
      case "internal":
        return `内部状态不可用：${detail.detail.message}`;
    }
  }
}
