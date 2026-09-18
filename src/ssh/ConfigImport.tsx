// 从 `~/.ssh/config` 导入（plan 0506）—— 池的**第一条写路径**的界面。
//
// 三件事必须让用户看见，缺一样"受限子集"就会退化成"悄悄错"（`scope.md` §8 风险 4）：
//
//   1. **支持哪些指令**（边界本身，静态）；
//   2. **哪些行没生效**（动态，逐条带行号 —— 比一份静态清单更有用）；
//   3. **整份被拒时是哪几处**（`Match` / `Include` / 不认识的指令，一次列全）。
//
// 它是**壳层**（`AGENTS.md` §4.0）：判据是"这条后端行为能不能被验证"，不是好不好看。
// 所以类名与 `data-*` 是稳定的，E2E 直接照它们驱动。

import { useState } from "react";
import { ConfigImportFailed, type ImportReport, importSshConfig } from "../ipc/hosts";
import type { ConfigFinding } from "../ipc/hosts";

/**
 * 界面列出的支持集。
 *
 * ⚠️ 权威在 `store::sshconfig`（那一份 `supported()` + 三档分类表），这里只是把
 * "支持什么"印在用户点之前。**六条的名字要与那边一致** —— E2E 会同时读这段文字与后端行为，
 * 两边不一致时那条用例会红。
 */
const SUPPORTED = ["Host", "HostName", "User", "Port", "IdentityFile", "ProxyJump"];

interface ConfigImportProps {
  /** 导入之后让主机列表重新读一次（池变了）。 */
  onImported(): void;
}

export function ConfigImport({ onImported }: ConfigImportProps) {
  const [path, setPath] = useState("");
  const [overwrite, setOverwrite] = useState(false);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [problems, setProblems] = useState<ConfigFinding[]>([]);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = () => {
    setBusy(true);
    setProblem(null);
    setReport(null);
    setProblems([]);
    const chosen = path.trim() === "" ? null : path.trim();
    void importSshConfig(chosen, overwrite)
      .then((result) => {
        setReport(result);
        onImported();
      })
      .catch((err: unknown) => {
        if (err instanceof ConfigImportFailed) {
          setProblems(err.problems);
          setProblem(err.message);
        } else {
          setProblem(String(err));
        }
      })
      .finally(() => setBusy(false));
  };

  return (
    <details className="config-import" data-import-panel>
      <summary className="config-import-summary">从 ~/.ssh/config 导入</summary>

      <p className="config-import-scope" data-import-supported={SUPPORTED.join(",")}>
        支持：{SUPPORTED.join(" / ")}。其余指令：认得的（保活、算法、转发、日志……）会逐条列出来
        告诉你它没生效；不认得的、以及 <code>Match</code> / <code>Include</code>{" "}
        <strong>整份报错、一行都不导入</strong> —— 宁可报错，也不猜着连。
      </p>

      <div className="config-import-form">
        <input
          className="config-import-path"
          data-import-path
          type="text"
          value={path}
          placeholder="留空 = ~/.ssh/config"
          onChange={(event) => setPath(event.target.value)}
        />
        <label className="config-import-overwrite">
          <input
            type="checkbox"
            data-import-overwrite
            checked={overwrite}
            onChange={(event) => setOverwrite(event.target.checked)}
          />
          覆盖同名条目
        </label>
        <button
          type="button"
          className="config-import-run"
          data-import-run
          disabled={busy}
          onClick={run}
        >
          {busy ? "导入中…" : "导入"}
        </button>
      </div>

      {problem && (
        <p className="config-import-problem" role="alert" data-import-problem>
          {problem}
        </p>
      )}
      {problems.length > 0 && (
        // "整份被拒"的那几处：**逐条列全**，用户一趟改完（后端就是一次列全的）。
        <ul className="config-import-problems" data-import-problems>
          {problems.map((item) => (
            <li key={`${item.line}:${item.keyword}`} data-import-problem-line={item.line}>
              第 {item.line} 行 <code>{item.keyword}</code>：{item.message}
            </li>
          ))}
        </ul>
      )}

      {report && (
        <div className="config-import-report" data-import-report>
          <p className="config-import-counts" data-import-counts>
            {report.path}：新增 {report.created.length} · 更新 {report.updated.length} · 跳过{" "}
            {report.skipped.length}
          </p>
          {report.skipped.length > 0 && (
            <ul className="config-import-skipped" data-import-skipped>
              {report.skipped.map((item) => (
                <li key={item.name}>
                  {item.name}：
                  {item.reason === "existing" ? "池里已有同名的行（没动）" : "跳板用池里已有的行"}
                </li>
              ))}
            </ul>
          )}
          {report.ignored.length > 0 && (
            <ul className="config-import-ignored" data-import-ignored>
              {report.ignored.map((item) => (
                <li key={`${item.line}:${item.keyword}`}>
                  第 {item.line} 行 <code>{item.keyword}</code>：{item.message}
                </li>
              ))}
            </ul>
          )}
          {report.notes.length > 0 && (
            <ul className="config-import-notes" data-import-notes>
              {report.notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          )}
        </div>
      )}
    </details>
  );
}
