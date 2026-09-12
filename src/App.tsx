import "./App.css";
import { TerminalPane } from "./terminal/TerminalPane";

function App() {
  // 壳层暂时只有终端。多标签 / 分屏的呈现方式尚未定型（`scope.md` §1.2），
  // 所以这里刻意不先搭标签栏 —— 后端叫 `Session`，怎么呈现是之后的事。
  return (
    <main className="app">
      <TerminalPane />
    </main>
  );
}

export default App;
