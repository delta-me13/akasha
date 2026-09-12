// 标签栏 —— **壳层**组件：只回答"有哪些标签页、哪个是当前的、关哪个"。
//
// 一条规则在这里落地（`docs/scope.md` §5.6）：
//   * **三大终端**（local / ssh / serial）的标签页**有关闭按钮**，关闭 = 立刻丢弃它的 `Session`；
//   * **仅渲染的视图标签页**（SSH 转发 / 密码库 / 文件传输）**没有关闭按钮** ——
//     它们只是后端的客户端视图，关掉前端不该影响后端执行。
//
// 所以 `×` 是**按 `kind` 渲染**的，而不是"所有标签页都长一样"。

/** 标签页的种类。当前只有终端；转发 / 密码库 / 文件传输到来时**新增**变体，不复用终端。 */
export type TabKind = "terminal";

/** 标签栏要显示的一项。**只是呈现数据** —— 会话的归属与回收在后端（`Session`）。 */
export interface TabView {
  readonly key: number;
  readonly kind: TabKind;
  readonly title: string;
}

interface TabStripProps {
  readonly tabs: readonly TabView[];
  /** 当前标签页的 `key`；一个都不剩时是 `-1`（见 `App.tsx` 的 `NO_TAB`）。 */
  readonly active: number;
  onSelect(key: number): void;
  onClose(key: number): void;
  onNew(): void;
}

export function TabStrip({ tabs, active, onSelect, onClose, onNew }: TabStripProps) {
  return (
    <div className="tab-strip" role="tablist" aria-label="终端标签页">
      {tabs.map((tab) => (
        <div
          key={tab.key}
          className={tab.key === active ? "tab is-active" : "tab"}
          data-tab-key={tab.key}
        >
          <button
            type="button"
            className="tab-label"
            role="tab"
            aria-selected={tab.key === active}
            onClick={() => onSelect(tab.key)}
          >
            {tab.title}
          </button>
          {tab.kind === "terminal" && (
            <button
              type="button"
              className="tab-close"
              // 关掉就是**真的关掉**：后端会 kill + wait 收尸，所以按钮上把后果写清楚，
              // 不靠一个确认弹窗来弥补（`docs/scope.md` §5.6：无宽限期、无二次确认）。
              title="关闭标签页 = 丢弃这个会话"
              aria-label={`关闭 ${tab.title}`}
              onClick={() => onClose(tab.key)}
            >
              ×
            </button>
          )}
        </div>
      ))}
      <button
        type="button"
        className="tab-new"
        title="新建终端标签页"
        aria-label="新建终端标签页"
        onClick={onNew}
      >
        +
      </button>
    </div>
  );
}
