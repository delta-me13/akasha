// SFTP 双栏面板 —— **功能验证壳层**（`AGENTS.md` §4.0）。
//
// 它存在的理由只有一个：让"两侧各自选**本机或一台主机**、各自连接、各自列目录、双向搬文件"
// 这些**后端行为**在界面上能被看见、能被验证。它不是设计稿，也不构成产品约束 ——
// 正式界面重做时，这一块该删；不该动的是它调的那几条命令与那个 `sftp` 探针。
//
// 四条纪律：
//   1. **状态是后端说了算**：两侧的状态、失败原因、当前目录、传输的进度与结局都来自命令的
//      返回值（这里保存的那份只是渲染用的副本），界面上不做任何状态推断；
//   2. 只有 `src/ipc/` 能碰后端（`AGENTS.md` §0 禁止 #1）；
//   3. **关面板不等于结束会话**：面板是仅渲染的视图（`docs/scope.md` §5.6）——
//      所以打开面板时先看一眼"后端有没有已经开着的会话"，有就接回去。
//      结束会话是面板里那个显式动作。
//   4. **进度靠读，不靠猜**：传输在后台跑，界面每 250 ms 读一次（只在还有在跑的传输时读）。
//      落盘不变量（临时名 → 原子重命名）全在后端，界面不做任何与之相关的判断。

import { useCallback, useEffect, useState } from "react";

import { listHosts, type HostEntry } from "../ipc/hosts";
import {
  cancelSftpTransfer,
  closeSftp,
  connectSftpSide,
  listSftp,
  listSftpSessions,
  listSftpTransfers,
  openSftp,
  sftpSides,
  startSftpTransfer,
  type SftpListing,
  type SftpOrigin,
  type SftpSide,
  type SftpSideInfo,
  type SftpTransfer,
  type SftpTransferState,
} from "../ipc/sftp";

/** 两侧的显示名（后端只认 `left` / `right`）。 */
const SIDE_LABEL: Record<SftpSide, string> = { left: "左", right: "右" };

/** 对侧是哪一栏（"传到对侧"要它）。 */
const OTHER: Record<SftpSide, SftpSide> = { left: "right", right: "left" };

/** 固定顺序的两侧 —— 界面永远画两栏，哪怕后端还没有会话。 */
const SIDES: readonly SftpSide[] = ["left", "right"];

/** 一侧在界面上的默认状态（后端还没有这一侧的信息时用）。 */
function idleSide(side: SftpSide): SftpSideInfo {
  return {
    side,
    origin: null,
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

/** 传输四个取值的中文文案（穷尽的 `Record`，同上一张表）。 */
const TRANSFER_LABEL: Record<SftpTransferState, string> = {
  running: "传输中",
  done: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

/** 下拉框里的选择 → 后端的 `origin`（`""` = 还没选）。 */
function parseOrigin(choice: string): SftpOrigin | null {
  if (choice === "local") return { kind: "local" };
  if (choice.startsWith("host:")) return { kind: "host", id: Number(choice.slice(5)) };
  return null;
}

/** `origin` → 下拉框里的选择（接回已有会话时要把选中项摆回去）。 */
function originChoice(origin: SftpOrigin | null): string {
  if (origin === null) return "";
  return origin.kind === "local" ? "local" : `host:${origin.id}`;
}

export function SftpPanel({ onClose }: { readonly onClose: () => void }) {
  /** 当前驱动的那个 SFTP 会话（后端登记过、正在被这个面板用）。 */
  const [handle, setHandle] = useState<number | null>(null);
  const [hosts, setHosts] = useState<readonly HostEntry[] | null>(null);
  const [sides, setSides] = useState<readonly SftpSideInfo[]>([]);
  const [listings, setListings] = useState<Partial<Record<SftpSide, SftpListing>>>({});
  /** 每一栏选中的来源。**只是界面的选择**，后端要到连接时才看到它。 */
  const [choice, setChoice] = useState<Partial<Record<SftpSide, string>>>({});
  /** 每一栏的路径输入框（"前往"用）。 */
  const [goto, setGoto] = useState<Partial<Record<SftpSide, string>>>({});
  const [transfers, setTransfers] = useState<readonly SftpTransfer[]>([]);
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
        if (!existing) return;
        setHandle(existing.handle);
        setSides(existing.sides);
        setTransfers(existing.transfers);
        // 顺便把每一栏选中的来源摆回下拉框（否则界面上会显示"未选择"，而后端明明连着）。
        setChoice(
          Object.fromEntries(
            existing.sides
              .filter((info) => info.origin !== null)
              .map((info) => [info.side, originChoice(info.origin)]),
          ),
        );
      })
      .catch((err: unknown) => {
        if (alive) setProblem(describe(err));
      });
    return () => {
      alive = false;
    };
  }, []);

  const refreshSides = useCallback(async (target: number) => {
    setSides(await sftpSides(target));
  }, []);

  const refreshTransfers = useCallback(async (target: number) => {
    try {
      setTransfers(await listSftpTransfers(target));
    } catch {
      // 会话可能在后端已经不在了（例如被另一个面板关掉）—— 读传输不该再报一次错。
    }
  }, []);

  // 还有在跑的传输时才继续读：跑完之后界面不再每 250 ms 抖一次（`AGENTS.md` §4.2）。
  const running = transfers.some((transfer) => transfer.state === "running");
  useEffect(() => {
    if (handle === null || !running) return;
    const timer = setInterval(() => void refreshTransfers(handle), 250);
    return () => clearInterval(timer);
  }, [handle, running, refreshTransfers]);

  /** 新建一个 SFTP 会话（两侧都还没连）。**显式动作** —— 见文件头第 3 条。 */
  const start = useCallback(async () => {
    setProblem(null);
    try {
      const handle = await openSftp();
      setHandle(handle);
      setSides(await sftpSides(handle));
      setTransfers([]);
    } catch (err) {
      setProblem(describe(err));
    }
  }, []);

  /** 连接某一侧，连上之后立刻把默认目录列出来（判据要看的正是这一步）。 */
  const connect = useCallback(
    async (side: SftpSide) => {
      if (handle === null) return;
      const origin = parseOrigin(choice[side] ?? "");
      if (origin === null) {
        setProblem(`先在${SIDE_LABEL[side]}栏选本机或一台主机`);
        return;
      }
      setBusy(side);
      setProblem(null);
      try {
        const info = await connectSftpSide(handle, side, origin);
        // 本机那一档的起点由后端给（`LocalEndpoint::default_dir`）—— 界面不猜家目录。
        const listing = await listSftp(handle, side, info.path ?? ".");
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

  /** 进入某个目录（`..` 走的是同一条路，交给端点的规范化解析）。 */
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

  /** 把某一栏的一个文件搬到**对侧当前的目录**里（判据要看的动作）。 */
  const send = useCallback(
    async (side: SftpSide, name: string) => {
      if (handle === null) return;
      const other = OTHER[side];
      const targetDir = listings[other]?.path;
      const sourceDir = listings[side]?.path;
      if (targetDir === undefined) {
        setProblem(`对侧（${SIDE_LABEL[other]}）还没有打开一个目录 —— 先连上并列出它`);
        return;
      }
      if (sourceDir === undefined) return;
      setProblem(null);
      try {
        await startSftpTransfer(
          handle,
          side,
          join(sourceDir, name),
          other,
          join(targetDir, name),
        );
        await refreshTransfers(handle);
      } catch (err) {
        setProblem(describe(err));
      }
    },
    [handle, listings, refreshTransfers],
  );

  /** 取消一次传输。**只是推信号** —— 状态由后端翻（见 `cancelSftpTransfer` 的文档）。 */
  const cancel = useCallback(
    async (id: number) => {
      if (handle === null) return;
      setProblem(null);
      try {
        await cancelSftpTransfer(handle, id);
        await refreshTransfers(handle);
      } catch (err) {
        setProblem(describe(err));
      }
    },
    [handle, refreshTransfers],
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
      setTransfers([]);
      setChoice({});
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
                data-sftp-origin={originChoice(info.origin)}
              >
                <header className="sftp-pane-head">
                  <span className="sftp-pane-name">
                    {SIDE_LABEL[side]} · {info.name === "" ? "未选择" : info.name}
                  </span>
                  <span className="sftp-pane-state">{STATE_LABEL[info.state]}</span>
                </header>

                <div className="sftp-pane-controls">
                  <select
                    className="sftp-origin"
                    aria-label={`${SIDE_LABEL[side]}栏的来源`}
                    value={choice[side] ?? ""}
                    disabled={hosts === null}
                    onChange={(event) => {
                      const value = event.target.value;
                      setChoice((current) => ({ ...current, [side]: value }));
                    }}
                  >
                    <option value="">选择…</option>
                    <option value="local">本机</option>
                    {(hosts ?? []).map((host) => (
                      <option key={host.id} value={`host:${host.id}`}>
                        {host.name}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className="sftp-connect"
                    disabled={busy !== null || (choice[side] ?? "") === ""}
                    onClick={() => void connect(side)}
                  >
                    {busy === side ? "连接中…" : "连接"}
                  </button>
                  <button
                    type="button"
                    className="sftp-up"
                    disabled={info.state !== "connected" || listing === undefined}
                    onClick={() => void enter(side, join(listing?.path ?? ".", ".."))}
                  >
                    上级
                  </button>
                </div>

                <div className="sftp-pane-controls">
                  <input
                    className="sftp-goto"
                    aria-label={`${SIDE_LABEL[side]}栏的路径`}
                    placeholder="绝对路径"
                    value={goto[side] ?? ""}
                    onChange={(event) => {
                      const value = event.target.value;
                      setGoto((current) => ({ ...current, [side]: value }));
                    }}
                  />
                  <button
                    type="button"
                    className="sftp-goto-go"
                    disabled={info.state !== "connected" || (goto[side] ?? "") === ""}
                    onClick={() => void enter(side, goto[side] ?? "")}
                  >
                    前往
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
                          onClick={() => void enter(side, join(listing?.path ?? ".", entry.name))}
                        >
                          {entry.name}/
                        </button>
                      ) : (
                        <>
                          <span className="sftp-entry-file">{entry.name}</span>
                          <button
                            type="button"
                            className="sftp-send"
                            data-entry-name={entry.name}
                            onClick={() => void send(side, entry.name)}
                          >
                            传到对侧
                          </button>
                        </>
                      )}
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
        </div>
      )}

      {handle !== null && (
        <section className="sftp-transfers" aria-label="传输">
          <header className="sftp-transfers-head">传输</header>
          {transfers.length === 0 ? (
            <p className="sftp-empty">还没有传输。</p>
          ) : (
            <ul className="sftp-transfer-list">
              {transfers.map((transfer) => (
                <li
                  key={transfer.id}
                  className="sftp-transfer"
                  data-transfer-id={transfer.id}
                  data-transfer-state={transfer.state}
                  data-transfer-done={transfer.done}
                  data-transfer-total={transfer.total}
                >
                  <span className="sftp-transfer-route">
                    {SIDE_LABEL[transfer.from]} {transfer.fromPath} → {SIDE_LABEL[transfer.to]}{" "}
                    {transfer.toPath}
                  </span>
                  <span className="sftp-transfer-state">{TRANSFER_LABEL[transfer.state]}</span>
                  <span className="sftp-transfer-bytes">
                    {transfer.done} / {transfer.total}
                  </span>
                  {transfer.state === "running" && (
                    <button
                      type="button"
                      className="sftp-transfer-cancel"
                      onClick={() => void cancel(transfer.id)}
                    >
                      取消
                    </button>
                  )}
                  {transfer.failure !== null && (
                    <span className="sftp-transfer-failure">{transfer.failure}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </section>
      )}
    </section>
  );
}

/** 拼一个路径片段。空目录名（相对路径）时不去添一个前导斜杠。 */
function join(directory: string, name: string): string {
  if (directory === "" || directory.endsWith("/")) return `${directory}${name}`;
  return `${directory}/${name}`;
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
