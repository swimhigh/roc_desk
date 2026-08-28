import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ErrorBoundary } from "./components/shared/ErrorBoundary";
import { logFrontendError } from "./services/diagnosticsService";
import "./monacoSetup";
import "./styles/globals.css";
import "./styles/components.css";

// 全局兜底（2026-08-28 用户反馈"程序有可能会自动退出不见了"）：ErrorBoundary 只能
// 捕获渲染期异常，事件回调/异步代码里抛出的异常和 Promise rejection 不会被它捕获，
// 之前这两类完全没有任何记录——静默失败，UI 可能卡住但连日志里都找不到线索。
window.addEventListener("error", (event) => {
  logFrontendError(event.message, event.error?.stack, event.filename ? `${event.filename}:${event.lineno}:${event.colno}` : undefined);
});
window.addEventListener("unhandledrejection", (event) => {
  const reason = event.reason;
  const message = reason instanceof Error ? reason.message : String(reason);
  const stack = reason instanceof Error ? reason.stack : undefined;
  logFrontendError(message, stack, "unhandledrejection");
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
