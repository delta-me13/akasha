import { useCallback, useRef, useState } from "react";
import "./App.css";
import { TabStrip, type TabView } from "./tabs/TabStrip";
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
 */
interface OpenTab {
  readonly key: number;
  readonly kind: "terminal";
}

function App() {
  // `key` 单调递增、**永不复用**：React 的 key 一旦被复用，"卸载旧面 / 挂载新面"就不再是
  // 我们以为的那两个动作 —— 而卸载就是关会话（见 `closeTab`），认错面等于关错会话。
  const [tabs, setTabs] = useState<OpenTab[]>([{ key: 0, kind: "terminal" }]);
  const [active, setActive] = useState(0);
  const nextKey = useRef(1);

  const openTab = useCallback(() => {
    const key = nextKey.current;
    nextKey.current += 1;
    setTabs((current) => [...current, { key, kind: "terminal" }]);
    setActive(key);
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

  const views: TabView[] = tabs.map((tab, index) => ({
    key: tab.key,
    kind: tab.kind,
    // 标题暂时就是序号：OSC 标题同步是后续的事（plan 0305 的非目标）。
    title: `终端 ${index + 1}`,
  }));

  return (
    <main className="app">
      <TabStrip
        tabs={views}
        active={active}
        onSelect={setActive}
        onClose={closeTab}
        onNew={openTab}
      />
      <div className="tab-panes">
        {tabs.map((tab) => (
          <div
            key={tab.key}
            className={tab.key === active ? "tab-pane is-active" : "tab-pane"}
            data-tab-pane={tab.key}
            aria-hidden={tab.key !== active}
          >
            {/* ⚠️ 所有标签页都**保持挂载**（`active` 只用来交焦点）：卸载 = 关会话，
                见 `closeTab`。非活动的那个由 CSS 隐藏 —— 用 `visibility`，不是 `display`。 */}
            <TerminalPane active={tab.key === active} />
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
