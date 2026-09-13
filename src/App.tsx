import { useCallback, useRef, useState } from "react";
import "./App.css";
import type { HostEntry } from "./ipc/hosts";
import type { SessionTarget } from "./ipc/session";
import { HostPicker } from "./ssh/HostPicker";
import { PromptPanel } from "./ssh/PromptPanel";
import { TabStrip, type TabKind, type TabView } from "./tabs/TabStrip";
import { TerminalPane } from "./terminal/TerminalPane";

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
      />
      {picking && <HostPicker onConnect={openSshTab} onClose={() => setPicking(false)} />}
      {/* 提示面板是**应用级**的：提问发生在"会话开起来之前"，不属于任何一个标签页。 */}
      <PromptPanel />
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

                会话**自己**结束时（终端里敲了 `exit`）走的是同一条关标签页路径：
                后端收掉它 + 发事件 → 这里 `closeTab`。 */}
            <TerminalPane
              target={tab.target}
              active={tab.key === active}
              onSessionEnded={() => closeTab(tab.key)}
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
