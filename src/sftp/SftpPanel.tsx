// SFTP 双栏面板 —— **功能验证壳层**（`AGENTS.md` §4.0）。
//
// 它存在的理由只有一个：让"两侧各自选主机、各自连接、各自列目录"这些**后端行为**
// 在界面上能被看见、能被验证。它不是设计稿，也不构成产品约束 —— 正式界面重做时，
// 这一块该删；不该动的是它调的那几条命令与那个 `sftp` 探针。
//
// 三条纪律：
//   1. **状态是后端说了算**：两侧的状态、失败原因、当前目录都来自命令的返回值
//      （这里保存的那份只是渲染用的副本），界面上不做任何状态推断；
//   2. 只有 `src/ipc/` 能碰后端（`AGENTS.md` §0 禁止 #1）；
//   3. **关面板不等于结束会话**：面板是仅渲染的视图（`docs/scope.md` §5.6）——
//      所以打开面板时先看一眼"后端有没有已经开着的会话"，有就接回去。
//      结束会话是面板里那个显式动作。

import { useCallback, useEffect, useState } from "react";

import { listHosts, type HostEntry } from "../ipc/hosts";
import {
  closeSftp,
  connectSftpSide,
  listSftp,
  listSftpSessions,
  openSftp,
  sftpSides,
  type SftpListing,
  type SftpSide,
  type SftpSideInfo,
} from "../ipc/sftp";

/** 两侧的显示名（后端只认 `left` / `right`）。 */
const SIDE_LABEL: Record<SftpSide, string> = { left: "左", right: "右" };

/** 固定顺序的两侧 —— 界面永远画两栏，哪怕后端还没有会话。 */
const SIDES: readonly SftpSide[] = ["left", "right"];

/** 一侧在界面上的默认状态（后端还没有这一侧的信息时用）。 */
function idleSide(side: SftpSide): SftpSideInfo {
  return {
    side,
    hostId: null,
    name: "",
    state: "disconnected",
    failure: null,
    path: null,
  };
}

/** 后端四个取值的中文文案。少一个键这里编译不过（`Record` 是穷尽的）。 */
const STATE_LABEL: Record<SftpSideInfo["state"], string> = {
  disconnected: "未连接",
  connecting: "连接中",
  connected: "已连接",
  failed: "失败",
};

export function SftpPanel({ onClose }: { readonly onClose: () => void }) {
  /** 当前驱动的那个 SFTP 会话（后端登记过、正在被这个面板用）。 */
  const [handle, setHandle] = useState<number | null>(null);
  const [hosts, setHosts] = useState<readonly HostEntry[] | null>(null);
  const [sides, setSides] = useState<readonly SftpSideInfo[]>([]);
  const [listings, setListings] = useState<Partial<Record<SftpSide, SftpListing>>>({});
  /** 每一栏选中的主机（池行 id）。**只是界面的选择**，后端要到连接时才看到它。 */
  const [choice, setChoice] = useState<Partial<Record<SftpSide, number>>>({});
  const [busy, setBusy] = useState<SftpSide | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    listHosts()
      .then((rows) => {
        if (alive) setHosts(rows);
      })
      .catch((err: unknown) => {
        if (alive) setProblem(describe(err));
      });
    // 后端可能已经有一个开着的会话（用户关掉面板又打开）—— 接回去，而不是另开一个。
    listSftpSessions()
      .then((sessions) => {
        if (!alive) return;
        const existing = sessions[sessions.length - 1];
        if (existing) {
          setHandle(existing.handle);
          setSides(existing.sides);
        }
      })
      .catch((err: unknown) => {
        if (alive) setProblem(describe(err));
      });
    return () => {
      alive = false;
    };
  }, []);

  const refreshSides = useCallback(async (target: number) => {
    const next = await sftpSides(target);
    setSides(next);
  }, []);

  /** 新建一个 SFTP 会话（两侧都还没连）。**显式动作** —— 见文件头第 3 条。 */
  const start = useCallback(async () => {
    setProblem(null);
    try {
      const handle = await openSftp();
      setHandle(handle);
      setSides(await sftpSides(handle));
    } catch (err) {
      setProblem(describe(err));
    }
  }, []);

  /** 连接某一侧，连上之后立刻把默认目录列出来（判据要看的正是这一步）。 */
  const connect = useCallback(
    async (side: SftpSide) => {
      if (handle === null) return;
      const hostId = choice[side];
      if (hostId === undefined) {
        setProblem(`先在${SIDE_LABEL[side]}栏选一台主机`);
        return;
      }
      setBusy(side);
      setProblem(null);
      try {
        await connectSftpSide(handle, side, hostId);
        const listing = await listSftp(handle, side, ".");
        setListings((current) => ({ ...current, [side]: listing }));
      } catch (err) {
        setProblem(describe(err));
      } finally {
        // 无论成败都刷新两侧状态：失败的原因在后端那一侧记着（界面上不猜）。
        try {
          await refreshSides(handle);
        } catch {
          // 会话可能在后端已经不在了（例如被另一个面板关掉）—— 状态刷新失败不足以再报一次。
        }
        setBusy(null);
      }
    },
    [handle, choice, refreshSides],
  );

  /** 进入某个目录（`..` 走的是同一条路，交给服务端的 `realpath` 解析）。 */
  const enter = useCallback(
    async (side: SftpSide, path: string) => {
      if (handle === null) return;
      setProblem(null);
      try {
        const listing = await listSftp(handle, side, path);
        setListings((current) => ({ ...current, [side]: listing }));
        await refreshSides(handle);
      } catch (err) {
        setProblem(describe(err));
      }
    },
    [handle, refreshSides],
  );

  /** 结束会话：两侧断开、后端注销。幂等。 */
  const stop = useCallback(async () => {
    if (handle === null) return;
    setProblem(null);
    try {
      await closeSftp(handle);
      setHandle(null);
      setSides([]);
      setListings({});
    } catch (err) {
      setProblem(describe(err));
    }
  }, [handle]);

  return (
    <section className="sftp-panel" aria-label="SFTP" data-sftp-handle={handle ?? ""}>
      <header className="sftp-title">
        <span>SFTP（双栏）</span>
        {handle === null ? (
          <button type="button" className="sftp-start" onClick={() => void start()}>
            新建会话
          </button>
        ) : (
          <button type="button" className="sftp-stop" onClick={() => void stop()}>
            结束会话
          </button>
        )}
        <button type="button" className="sftp-close" onClick={onClose} aria-label="关闭 SFTP 面板">
          ×
        </button>
      </header>

      {problem !== null && <p className="sftp-problem">{problem}</p>}
      {handle === null && problem === null && (
        <p className="sftp-empty">
          还没有 SFTP 会话 —— 点「新建会话」开一个。它不依赖任何终端标签页
          （docs/scope.md §5.1）。
        </p>
      )}

      {handle !== null && (
        <div className="sftp-panes">
          {SIDES.map((side) => {
            const info = sides.find((candidate) => candidate.side === side) ?? idleSide(side);
            const listing = listings[side];
            return (
              <section
                key={side}
                className="sftp-pane"
                data-side={side}
                data-sftp-state={info.state}
                data-sftp-host={info.hostId ?? ""}
              >
                <header className="sftp-pane-head">
                  <span className="sftp-pane-name">
                    {SIDE_LABEL[side]} · {info.name === "" ? "未选择主机" : info.name}
                  </span>
                  <span className="sftp-pane-state">{STATE_LABEL[info.state]}</span>
                </header>

                <div className="sftp-pane-controls">
                  <select
                    className="sftp-host"
                    aria-label={`${SIDE_LABEL[side]}栏的主机`}
                    value={choice[side] ?? ""}
                    disabled={hosts === null}
                    onChange={(event) => {
                      const value = event.target.value;
                      setChoice((current) => ({
                        ...current,
                        [side]: value === "" ? undefined : Number(value),
                      }));
                    }}
                  >
                    <option value="">选择主机…</option>
                    {(hosts ?? []).map((host) => (
                      <option key={host.id} value={host.id}>
                        {host.name}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className="sftp-connect"
                    disabled={busy !== null || choice[side] === undefined}
                    onClick={() => void connect(side)}
                  >
                    {busy === side ? "连接中…" : "连接"}
                  </button>
                  <button
                    type="button"
                    className="sftp-up"
                    disabled={info.state !== "connected" || listing === undefined}
                    onClick={() => void enter(side, `${listing?.path ?? "."}/..`)}
                  >
                    上级
                  </button>
                </div>

                {info.failure !== null && <p className="sftp-failure">{info.failure}</p>}

                <p className="sftp-path" data-sftp-path={listing?.path ?? ""}>
                  {listing?.path ?? (info.path ?? "（尚未列目录）")}
                </p>

                <ul className="sftp-entries">
                  {(listing?.entries ?? []).map((entry) => (
                    <li
                      key={entry.name}
                      className="sftp-entry"
                      data-entry-name={entry.name}
                      data-entry-kind={entry.kind}
                    >
                      {entry.kind === "directory" ? (
                        <button
                          type="button"
                          className="sftp-entry-dir"
                          onClick={() =>
                            void enter(
                              side,
                              `${listing?.path ?? "."}/${entry.name}`,
                            )
                          }
                        >
                          {entry.name}/
                        </button>
                      ) : (
                        <span className="sftp-entry-file">{entry.name}</span>
                      )}
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
        </div>
      )}
    </section>
  );
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
