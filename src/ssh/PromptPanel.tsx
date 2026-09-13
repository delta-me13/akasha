// 提示面板 —— **后端在连接中途问用户的那几件事**（plan 0504）。
//
// 判据是"这条后端行为能被验证"，不是好不好看（`AGENTS.md` §4.0）。所以这里只有两种形态：
//   * **没见过的主机密钥**：显示要核对的那串指纹 + 接受 / 拒绝。**默认不接受**（D11）。
//   * **凭据**：一句 prompt + 一个口令框。
//
// ⚠️ 三条别改回去的地方：
//   1. 口令**不进任何 state 之外的地方**：提交后立刻清空，不缓存、不写日志；
//   2. 答案提交失败若是 `gone`（那一问已经超时被撤下）→ **什么都不做**，因为界面
//      通常已经收到了 `ssh_prompt_dismissed` —— 弹一个红框只会在解释一件已经过去的事；
//   3. 面板是**应用级**的，不属于任何一个标签页：提示发生在"会话开起来之前"。

import { useEffect, useState } from "react";
import {
  answerPromptCredential,
  answerPromptHostKey,
  cancelPrompt,
  PromptRejected,
  subscribePrompts,
  type PromptRequest,
} from "../ipc/prompts";

export function PromptPanel() {
  const [prompts, setPrompts] = useState<PromptRequest[]>([]);
  const [secret, setSecret] = useState("");
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(
    () =>
      subscribePrompts(
        (prompt) => setPrompts((current) => [...current, prompt]),
        // 超时被后端撤下：**把它从列表里去掉**，否则用户会对着一问早已不存在的提示作答。
        (id) => setPrompts((current) => current.filter((item) => item.id !== id)),
      ),
    [],
  );

  const drop = (id: number) => {
    setPrompts((current) => current.filter((item) => item.id !== id));
  };

  /** 提交失败时的统一处置：`gone` 不是错误（那一问已经没了），别的照实说。 */
  const failed = (err: unknown) => {
    if (err instanceof PromptRejected && err.isGone) return;
    setProblem(err instanceof Error ? err.message : String(err));
  };

  if (prompts.length === 0) return null;

  return (
    <div className="ssh-prompts" role="dialog" aria-label="SSH 提示">
      {prompts.map((prompt) =>
        prompt.kind === "hostKey" ? (
          <div
            key={prompt.id}
            className="ssh-prompt"
            data-prompt-kind="hostKey"
            data-prompt-id={prompt.id}
          >
            <p className="ssh-prompt-title">
              没见过这台主机（{prompt.host}:{prompt.port}）的密钥，算法 {prompt.algorithm}
            </p>
            <p className="ssh-prompt-fingerprint" data-fingerprint={prompt.fingerprint}>
              {prompt.fingerprint}
            </p>
            <p className="ssh-prompt-hint">
              请与服务端上真实的那串指纹核对。接受之后它会记进 akasha 的库，下次不再问。
            </p>
            <div className="ssh-prompt-actions">
              <button
                type="button"
                className="ssh-prompt-accept"
                onClick={() => {
                  drop(prompt.id);
                  void answerPromptHostKey(prompt.id, true).catch(failed);
                }}
              >
                接受并记住
              </button>
              <button
                type="button"
                className="ssh-prompt-reject"
                onClick={() => {
                  drop(prompt.id);
                  void answerPromptHostKey(prompt.id, false).catch(failed);
                }}
              >
                拒绝
              </button>
            </div>
          </div>
        ) : (
          <form
            key={prompt.id}
            className="ssh-prompt"
            data-prompt-kind="credential"
            data-prompt-id={prompt.id}
            onSubmit={(event) => {
              event.preventDefault();
              const answer = secret;
              setSecret("");
              drop(prompt.id);
              void answerPromptCredential(prompt.id, answer).catch(failed);
            }}
          >
            <p className="ssh-prompt-title">{prompt.prompt}</p>
            <p className="ssh-prompt-hint">{prompt.target}</p>
            <input
              className="ssh-prompt-secret"
              type="password"
              autoFocus
              aria-label={prompt.prompt}
              value={secret}
              onChange={(event) => setSecret(event.target.value)}
            />
            <div className="ssh-prompt-actions">
              <button type="submit" className="ssh-prompt-submit">
                确定
              </button>
              <button
                type="button"
                className="ssh-prompt-cancel"
                onClick={() => {
                  setSecret("");
                  drop(prompt.id);
                  void cancelPrompt(prompt.id).catch(failed);
                }}
              >
                取消
              </button>
            </div>
          </form>
        ),
      )}
      {problem && (
        <p className="ssh-prompt-problem" role="alert">
          {problem}
        </p>
      )}
    </div>
  );
}
