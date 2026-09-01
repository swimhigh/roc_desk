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

// 全局兜底（2026-09-01 用户反馈：在文件树空白处右键点了"刷新"，整个工作区直接
// 回到了首页，重新进工作区后终端 su root 的会话状态也丢了）：WebView2 有一套自己
// 的原生右键菜单（重新加载/后退/前进等），只要某个区域没有组件自己 preventDefault
// 掉，右键就会露出这套菜单——点"重新加载"效果等于按 F5，整个 React 应用连同
// Zustand 里所有内存状态（当前工作区、终端会话）一起被扔掉重新起来，但后端的
// SSH/Agent 连接池和 PTY 进程并不知道前端刚刚"重生"了一次，重新打开工作区时
// 前端只能新开一个 channel/shell，旧的 su root 状态自然接不回去。这不是某一个
// 组件的 bug，是这个桌面应用压根不应该暴露浏览器那套原生右键菜单——所有自定义
// 右键菜单都是组件自己 preventDefault 之后再弹的，不依赖这个全局兜底；这里只是
// 把"没人处理的空白区域"也一并盖住，和大多数 Electron/Tauri 桌面应用的默认做法
// 一致。
//
// 终端（xterm.js）曾经是例外——那时候这个应用没有自己实现终端复制/粘贴，靠的是
// WebView2 原生右键菜单（2026-09-01 一度被这条全局屏蔽误伤）。现在
// `TerminalView.tsx` 自己实现了一套 CMD QuickEdit 风格的右键（有选区就复制、
// 没有就粘贴到光标处，见那边 `onContextMenu`），会在事件冒泡到这里之前就
// `preventDefault()`，不再需要放行原生菜单这个特例了——统一交给这里兜底屏蔽。
window.addEventListener("contextmenu", (event) => {
  event.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
