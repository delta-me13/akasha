import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

/**
 * 装配顺序很重要：**先装模拟后端，再渲染**。
 *
 * 浏览器里跑 `just dev-web` 时没有 app、也没有 `__TAURI_INTERNALS__` —— 不装的话
 * 终端一开就只会报"会话不存在"。生产构建里 `import.meta.env.DEV` 是常量 `false`，
 * 这个分支连同 `./ipc/mock` 那个 chunk 一起被摇掉（后端只能由 Rust 提供）。
 */
async function bootstrap(): Promise<void> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in globalThis)) {
    const { installDevBackend } = await import("./ipc/mock");
    installDevBackend();
  }

  const root = document.getElementById("root");
  if (!root) throw new Error("index.html 里没有 #root");

  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void bootstrap().catch((err: unknown) => {
  // 这一步失败等于白屏，所以把原因留在控制台（前端唯一的"报错出口"）。
  console.error("akasha 启动失败：", err);
});
