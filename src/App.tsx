import { useCallback, useRef, useState } from "react";
import "./App.css";
import type { HostEntry } from "./ipc/hosts";
import type { SerialParams, SessionEnd, SessionTarget } from "./ipc/session";
import { BwPanel } from "./bitwarden/BwPanel";
import { SerialPicker } from "./serial/SerialPicker";
import { SftpPanel } from "./sftp/SftpPanel";
import { HostPicker } from "./ssh/HostPicker";
import { PromptPanel } from "./ssh/PromptPanel";
import { TabStrip, type TabKind, type TabView } from "./tabs/TabStrip";
import { TerminalPane } from "./terminal/TerminalPane";
import { TunnelPanel } from "./tunnels/TunnelPanel";

/** 一个标签页都没有时 `active` 的取值。刻意用 `-1` 而不是 `null`：一路是数字，比较省事。 */
const NO_TAB = -1;

/**
 * 一个打开的标签页。
 *
 * `kind` 不是多余的：**"能不能关"由它决定**（`docs/scope.md` §5.6）。三大终端
 * （local / ssh / serial）的标签页关闭 = **立刻丢弃**它的 `Session`；仅渲染的视图标签页
 * （转发 / 密码库 / 文件传输）**没有关闭按钮** —— 它们到来时**新增** `kind`，
 * 不复用 `"terminal"`（复用会让"关掉 = 后端照跑"和"关掉 = 立刻丢弃"混成同一个值）。
 *
 * `target` 与 `kind` **不是同一件事**：`kind` 管呈现与关闭语义，`target` 管"后端照哪条路
 * 开会话"（本地 shell / 主机池里的一台）。合成一个字段的话，将来 serial 到来时
 * "要不要 ×"就得从载体类型里推 —— 而那是一条会推错的规则。
 */
interface OpenTab {
  readonly key: number;
  readonly kind: TabKind;
  readonly target: SessionTarget;
  /** 标签页标题。SSH 用池里那台主机的名字。 */
  readonly title: string;
}

function App() {
  // `key` 单调递增、**永不复用**：React 的 key 一旦被复用，"卸载旧面 / 挂载新面"就不再是
  // 我们以为的那两个动作 —— 而卸载就是关会话（见 `closeTab`），认错面等于关错会话。
  const [tabs, setTabs] = useState<OpenTab[]>([
    { key: 0, kind: "terminal", target: { kind: "local" }, title: "终端 1" },
  ]);
  const [active, setActive] = useState(0);
  /** 主机选择器开着没有（选中一台、或者取消之后关掉）。 */
  const [picking, setPicking] = useState(false);
  /** 串口选择器开着没有（plan 1101）。与主机选择器同类：选一条配置就关掉。 */
  const [pickingSerial, setPickingSerial] = useState(false);
  /** 隧道面板开着没有。**它不是一个标签页**：关掉面板不停任何隧道（见 `TabStrip`）。 */
  const [tunnelsOpen, setTunnelsOpen] = useState(false);
  /** SFTP 面板开着没有。同上：关掉面板不停那个会话（`docs/scope.md` §5.6）。 */
  const [sftpOpen, setSftpOpen] = useState(false);
  /** Bitwarden 面板开着没有（plan 0902 / 0905）。同上：关掉面板什么后端状态都不变。 */
  const [bitwardenOpen, setBitwardenOpen] = useState(false);
  /**
   * 最近一次"会话自己结束"的那句话（plan 1103）。
   *
   * 为什么它必须住在壳层：说出这句话的那个面**随即就被卸载了**（标签页与会话同生命期）。
   * 它也补上了一个一直没被显示过的字段 —— `session_ended` 从一开始就带着 `status`
   * （"退出码 N" / 被信号终止），只是此前中间那几层把它丢掉了。
   */
  const [notice, setNotice] = useState<string | null>(null);
  const nextKey = useRef(1);

  const openLocalTab = useCallback(() => {
    const key = nextKey.current;
    nextKey.current += 1;
    setTabs((current) => [
      ...current,
      {
        key,
        kind: "terminal",
        target: { kind: "local" },
        // 标题暂时就是序号：OSC 标题同步是后续的事（plan 0305 的非目标）。
        title: `终端 ${current.filter((tab) => tab.kind === "terminal").length + 1}`,
      },
    ]);
    setActive(key);
  }, []);

  /**
   * 选好一台主机 → 开一个 SSH 标签页。
   *
   * 这里只做"开一个面"：**连接是那个面自己发起的**（`attachTerminal`）—— 于是连接中途的
   * 提问（凭据 / 没见过的主机密钥）由应用级的提示面板接，不需要这一个动作去等它。
   */
  const openSshTab = useCallback((host: HostEntry) => {
    const key = nextKey.current;
    nextKey.current += 1;
    setTabs((current) => [
      ...current,
      { key, kind: "ssh", target: { kind: "ssh", hostId: host.id }, title: host.name },
    ]);
    setActive(key);
    setPicking(false);
  }, []);

  /**
   * 收到一张填好的表单 → 开一个串口标签页（plan 1101 起，plan 1102 改成收参数而不是池行）。
   *
   * 与 SSH 那条同形：这里只做"开一个面"，**打开设备是那个面自己发起的**（`attachTerminal`）——
   * 于是参数不合法（数据位 9）时那句"字段与取值"由那个面的报错行说出来，而不是由这里弹一句。
   * 差别只有一处：串口**不碰库**（没有秘密），所以它不需要先解锁 —— 只有"列出池里有哪些"
   * 那一步需要（`SerialPicker` 自己会说清那一句）。
   */
  const openSerialTab = useCallback((params: SerialParams, title: string) => {
    const key = nextKey.current;
    nextKey.current += 1;
    setTabs((current) => [
      ...current,
      { key, kind: "serial", target: { kind: "serial", params }, title },
    ]);
    setActive(key);
    setPickingSerial(false);
  }, []);

  /**
   * 关闭一个标签页 —— **这就是"丢弃 `Session`"这个动作本身**（`docs/scope.md` §5.6）。
   *
   * 这里**没有第二条关闭路径**：从列表里移除 → React 卸载 `<TerminalPane>` →
   * `attachTerminal` 的清理函数发 `close_session` → 后端 kill + wait 收尸（plan 0204）。
   * 不弹确认、不留宽限期；**只丢这一个**，别的标签页的会话与进程不动。
   *
   * ⚠️ 所以**不要**在这里顺手"多收一点"（比如关闭时顺带清掉别的标签页）：
   * 一个标签页只拥有一个 `Session`。
   */
  const closeTab = useCallback(
    (key: number) => {
      const index = tabs.findIndex((tab) => tab.key === key);
      if (index < 0) return;
      const rest = tabs.filter((tab) => tab.key !== key);
      setTabs(rest);
      if (active === key) {
        // 关掉的是当前这个：优先接左边，其次接右边；一个不剩就是空状态。
        const neighbour = rest[index - 1] ?? rest[index] ?? null;
        setActive(neighbour ? neighbour.key : NO_TAB);
      }
    },
    [tabs, active],
  );

  /**
   * 一个会话**自己**结束了：先把那句话挂到通知行，再关掉它的标签页。
   *
   * 顺序是有意的 —— 反过来的话，说出这句话的那个面已经没了，而"为什么没了"正是本条
   * 唯一要留给用户的东西。
   */
  const endedTab = useCallback(
    (tab: OpenTab, status: SessionEnd) => {
      setNotice(status ? `「${tab.title}」已结束：${status}` : `「${tab.title}」已结束`);
      closeTab(tab.key);
    },
    [closeTab],
  );

  const views: TabView[] = tabs.map((tab) => ({
    key: tab.key,
    kind: tab.kind,
    title: tab.title,
  }));

  return (
    <main className="app">
      <TabStrip
        tabs={views}
        active={active}
        onSelect={setActive}
        onClose={closeTab}
        onNew={openLocalTab}
        onNewSsh={() => setPicking(true)}
        onNewSerial={() => setPickingSerial((open) => !open)}
        onNewTunnel={() => setTunnelsOpen((open) => !open)}
        onNewSftp={() => setSftpOpen((open) => !open)}
        onNewBitwarden={() => setBitwardenOpen((open) => !open)}
      />
      {picking && <HostPicker onConnect={openSshTab} onClose={() => setPicking(false)} />}
      {/* 串口面板与主机选择器同类（plan 1101，plan 1102 起是一张三输入的表单）：应用级浮层。 */}
      {pickingSerial && (
        <SerialPicker onConnect={openSerialTab} onClose={() => setPickingSerial(false)} />
      )}
      {/* 隧道面板与主机选择器同类：**应用级浮层**，不是标签页 —— 隧道是独立的 `Session`，
          关掉面板不停它（`docs/scope.md` §2.2 / §5.6）。 */}
      {tunnelsOpen && <TunnelPanel onClose={() => setTunnelsOpen(false)} />}
      {/* SFTP 面板同一条理由：它也不是标签页，关掉面板不停那个会话（仅渲染的视图）。 */}
      {sftpOpen && <SftpPanel onClose={() => setSftpOpen(false)} />}
      {/* Bitwarden 面板：既不是终端也不带关闭语义 —— 那几条命令都是显式动作。 */}
      {bitwardenOpen && <BwPanel onClose={() => setBitwardenOpen(false)} />}
      {/* 提示面板是**应用级**的：提问发生在"会话开起来之前"，不属于任何一个标签页。 */}
      <PromptPanel />
      {/* 会话结束时的那句话（plan 1103）：它说的是**为什么**这个标签页没了，所以留在壳层。 */}
      {notice && (
        <p className="app-notice" role="status" data-session-notice>
          {notice}
        </p>
      )}
      <div className="tab-panes">
        {tabs.map((tab) => (
          <div
            key={tab.key}
            className={tab.key === active ? "tab-pane is-active" : "tab-pane"}
            data-tab-pane={tab.key}
            aria-hidden={tab.key !== active}
          >
            {/* ⚠️ 所有标签页都**保持挂载**（`active` 只用来交焦点）：卸载 = 关会话，
                见 `closeTab`。非活动的那个由 CSS 隐藏 —— 用 `visibility`，不是 `display`。

                会话**自己**结束时（终端里敲了 `exit`、串口设备被拔掉、SSH 掉线）走的是
                同一条关标签页路径：后端收掉它 + 发事件 → `endedTab`（先记下那句话再关）。 */}
            <TerminalPane
              target={tab.target}
              active={tab.key === active}
              onSessionEnded={(status) => endedTab(tab, status)}
            />
          </div>
        ))}
        {tabs.length === 0 && (
          <p className="tab-empty">
            没有打开的标签页 —— 用左上角的 + 开一个终端。关闭终端标签页等于立刻丢弃它的会话
            （docs/scope.md §5.6）。
          </p>
        )}
      </div>
    </main>
  );
}

export default App;
