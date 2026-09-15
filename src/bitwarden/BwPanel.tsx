// Bitwarden 面板 —— **功能验证壳层**（`AGENTS.md` §4.0）。
//
// 它存在的理由只有一个：让"两个轴各自可切、运行时下载能拿到一份 CLI、登录 / 解锁 / 锁定
// 的三态跟着 `bw` 自己说的走、session key 只在内存"这些**后端行为**在界面上能被看见、
// 能被验证。它不是设计稿，也不构成产品约束。
//
// 两条纪律与其余面板相同：
//   1. **状态以后端为准**：这里保存的那份只是渲染用的副本，来源只有那几条命令的返回值
//      —— 界面上不做任何状态推断（三态的字面量就是 `bw status --raw` 的取值）；
//   2. 只有 `src/ipc/` 能碰后端（`AGENTS.md` §0 禁止 #1）。
//
// ⚠️ 这一版**没有**自签证书（`NODE_EXTRA_CA_CERTS`）那一栏：自托管的服务器要么用一张
// 被系统信任的证书，要么先把 CA 装进系统信任库。这条缺口记在 `docs/STATUS.md`。

import { useCallback, useEffect, useState } from "react";

import {
  BwUnavailable,
  bwLock,
  bwLogin,
  bwLogout,
  bwStatus,
  bwUnlock,
  installBwCli,
  setBwCli,
  setBwServer,
  type AppDataMode,
  type BinarySource,
  type BwSnapshot,
} from "../ipc/bitwarden";

/** 三态的中文文案。键与后端 `BwStatusView["state"]` 一一对应（`Record` 是穷尽的）。 */
const STATE_LABEL: Record<string, string> = {
  unauthenticated: "未登录",
  locked: "已登录，未解锁",
  unlocked: "已解锁",
};

/** 变体那一档的文案。`unknown` 也有一句 —— "判不出"不是"没问题"。 */
const VARIANT_LABEL: Record<string, string> = {
  oss: "OSS（GPL-3.0-only）",
  proprietary: "专有（许可限制在生产环境使用）",
  unknown: "判不出变体",
};

/** 一步动作的公共骨架：跑一个命令、把结果或那句话写进界面。 */
function useActions() {
  const [snapshot, setSnapshot] = useState<BwSnapshot | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  const run = useCallback(async (label: string, action: () => Promise<BwSnapshot>) => {
    setBusy(label);
    setFailure(null);
    try {
      setSnapshot(await action());
    } catch (err) {
      // 命令失败时**也刷新一次读数**：`bw` 的状态可能已经变了（例如登录失败之后仍是
      // `unauthenticated`），而界面此时最该显示的是它现在的样子。
      setFailure(err instanceof BwUnavailable ? err.message : String(err));
      try {
        setSnapshot(await bwStatus());
      } catch {
        // 刷新也失败：上面那句话已经是用户能拿到的唯一信息了，不覆盖它。
      }
    } finally {
      setBusy(null);
    }
  }, []);

  return { snapshot, setSnapshot, busy, failure, setFailure, run };
}

/**
 * Bitwarden 面板：CLI 那一块（两个轴 + 版本 + 变体 + 下载）、服务器那一块、
 * 登录 / 解锁那一块、状态那一块。
 */
export function BwPanel({ onClose }: { readonly onClose: () => void }) {
  const { snapshot, setSnapshot, busy, failure, setFailure, run } = useActions();
  /** 服务器地址那一栏（空 = 官方云的默认值）。 */
  const [server, setServer] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  /** 两步验证那一对（空着就是不用）。 */
  const [method, setMethod] = useState("");
  const [code, setCode] = useState("");

  useEffect(() => {
    let alive = true;
    bwStatus().then(
      (next) => {
        if (!alive) return;
        setSnapshot(next);
        // 只回填第一次：用户正在改那一栏时别把它顶掉。
        setServer((current) => current || (next.server ?? ""));
      },
      (err: unknown) => {
        if (alive) setFailure(err instanceof Error ? err.message : String(err));
      },
    );
    return () => {
      alive = false;
    };
  }, [setFailure, setSnapshot]);

  const cli = snapshot?.cli;
  const state = snapshot?.status?.state ?? null;

  /** 两个轴：一个下拉一个下拉，**各自独立**（ADR-0007 D4）。 */
  const switchTo = (binary: BinarySource, appdata: AppDataMode) =>
    run("switch", () => setBwCli(binary, appdata));

  return (
    <section className="bw-panel" aria-label="Bitwarden">
      <header className="bw-head">
        <h2>Bitwarden</h2>
        <button type="button" onClick={onClose} data-bw-close>
          关闭面板
        </button>
      </header>

      {/* ── CLI 那一块 ───────────────────────────────────────────────── */}
      <fieldset className="bw-cli">
        <legend>bw CLI</legend>
        <label>
          二进制来源
          <select
            value={cli?.binary ?? "host"}
            disabled={busy !== null}
            data-bw-binary
            onChange={(event) =>
              switchTo(event.target.value as BinarySource, (cli?.appdata ?? "host") as AppDataMode)
            }
          >
            <option value="host">宿主机 PATH 里的 bw</option>
            <option value="managed">运行时下载的那一份</option>
          </select>
        </label>
        <label>
          CLI 状态目录
          <select
            value={cli?.appdata ?? "host"}
            disabled={busy !== null}
            data-bw-appdata
            onChange={(event) =>
              switchTo((cli?.binary ?? "host") as BinarySource, event.target.value as AppDataMode)
            }
          >
            <option value="host">它自己的默认位置</option>
            <option value="managed">数据目录下的隔离目录</option>
          </select>
        </label>
        <button
          type="button"
          disabled={busy !== null}
          data-bw-install
          onClick={() => run("install", installBwCli)}
        >
          {busy === "install" ? "正在下载…" : "下载一份 OSS 变体并改用它"}
        </button>

        <p className="bw-cli-facts" data-bw-cli>
          {cli?.version ? `版本 ${cli.version}` : "版本未知"}
          {cli?.variant ? ` · ${VARIANT_LABEL[cli.variant] ?? cli.variant}` : ""}
          {cli?.program ? ` · ${cli.program}` : ""}
        </p>
        {/* "没有 bw"与"还没下载"是两句不同的话 —— 这里显示的就是后端那两句之一。 */}
        {cli?.problem && (
          <p className="bw-problem" role="status" data-bw-problem>
            {cli.problem}
          </p>
        )}
        {/* 专有变体的许可证提示：后端决定要不要给这一句（ADR-0007 D5）。 */}
        {cli?.licenseNotice && (
          <p className="bw-license" role="status" data-bw-license>
            {cli.licenseNotice}
          </p>
        )}
      </fieldset>

      {/* ── 服务器那一块（自托管在这里） ─────────────────────────────── */}
      <fieldset className="bw-server">
        <legend>服务器</legend>
        <label>
          地址（留空 = 官方云）
          <input
            value={server}
            placeholder="https://vault.example.com"
            disabled={busy !== null}
            data-bw-server
            onChange={(event) => setServer(event.target.value)}
          />
        </label>
        <button
          type="button"
          disabled={busy !== null || server.trim() === ""}
          data-bw-server-set
          onClick={() => run("server", () => setBwServer(server))}
        >
          使用这个服务器
        </button>
        {/* 显示的是 **CLI 里配置的**那一个（`bw config server` 的回读），不是 `bw status`
            里的：未登录时上游的 `status` 给的是 `null`，而配置好的地址一直读得出来。 */}
        <p className="bw-server-current" data-bw-server-current>
          当前：{snapshot?.server ?? "（还没有读到）"}
        </p>
      </fieldset>

      {/* ── 登录 / 解锁那一块 ────────────────────────────────────────── */}
      <fieldset className="bw-login">
        <legend>登录 / 解锁</legend>
        <label>
          邮箱
          <input
            value={email}
            disabled={busy !== null}
            data-bw-email
            onChange={(event) => setEmail(event.target.value)}
          />
        </label>
        <label>
          主密码
          <input
            type="password"
            value={password}
            disabled={busy !== null}
            data-bw-password
            onChange={(event) => setPassword(event.target.value)}
          />
        </label>
        <label>
          两步验证方式（可留空）
          <input
            value={method}
            placeholder="0 / 1 / 3"
            disabled={busy !== null}
            data-bw-method
            onChange={(event) => setMethod(event.target.value)}
          />
        </label>
        <label>
          两步验证码（可留空）
          <input
            value={code}
            disabled={busy !== null}
            data-bw-code
            onChange={(event) => setCode(event.target.value)}
          />
        </label>
        <div className="bw-buttons">
          <button
            type="button"
            disabled={busy !== null || email.trim() === "" || password === ""}
            data-bw-login
            onClick={() => {
              // ⚠️ 提交之后**立刻**清掉界面里那一份主密码（成功失败都清）：它已经交给后端
              // 转手送进子进程的环境（ADR-0007 D8），留在输入框里没有任何理由。
              const attempt = run("login", () =>
                bwLogin(
                  email,
                  password,
                  method.trim() !== "" && code.trim() !== ""
                    ? { method: method.trim(), code: code.trim() }
                    : undefined,
                ),
              );
              void attempt.finally(() => setPassword(""));
            }}
          >
            {busy === "login" ? "登录中…" : "登录"}
          </button>
          <button
            type="button"
            disabled={busy !== null || password === ""}
            data-bw-unlock
            onClick={() => {
              const attempt = run("unlock", () => bwUnlock(password));
              void attempt.finally(() => setPassword(""));
            }}
          >
            {busy === "unlock" ? "解锁中…" : "解锁"}
          </button>
          <button type="button" disabled={busy !== null} data-bw-lock onClick={() => run("lock", bwLock)}>
            锁定
          </button>
          <button
            type="button"
            disabled={busy !== null}
            data-bw-logout
            onClick={() => run("logout", bwLogout)}
          >
            登出
          </button>
        </div>
        {/* ⚠️ 主密码只在这一刻交给后端（它转手交给子进程的环境，不进 argv，ADR-0007 D8）。
            提交之后立刻清掉界面里那份 —— 它没有任何留下来的理由。 */}
      </fieldset>

      {/* ── 状态那一块 ───────────────────────────────────────────────── */}
      <fieldset className="bw-state">
        <legend>状态</legend>
        <p data-bw-status>
          {state ? STATE_LABEL[state] ?? state : "（读不到）"}
          {snapshot?.status?.userEmail ? ` · ${snapshot.status.userEmail}` : ""}
          {snapshot?.status?.lastSync ? ` · 上次同步 ${snapshot.status.lastSync}` : ""}
        </p>
        <p data-bw-has-session>
          {snapshot?.hasSession ? "session key 在内存里" : "内存里没有 session key"}
        </p>
        {/* 刷新：状态的三态**只有后端一个来源**，而它可能被我们之外的动作改掉
            （例如用户在终端里跑了 `bw lock`）—— 那时界面该能重新问一次（ADR-0007 D10）。 */}
        <button type="button" disabled={busy !== null} data-bw-refresh onClick={() => run("refresh", bwStatus)}>
          刷新状态
        </button>
        {snapshot?.problem && (
          <p className="bw-problem" role="status" data-bw-status-problem>
            {snapshot.problem}
          </p>
        )}
      </fieldset>

      {failure && (
        <p className="bw-failure" role="alert" data-bw-failure>
          {failure}
        </p>
      )}
    </section>
  );
}
