import React from "react";
import { logFrontendError } from "../../services/diagnosticsService";

interface ErrorBoundaryState {
  error: Error | null;
}

/**
 * 顶层渲染异常兜底（2026-08-28 用户反馈"程序有可能会自动退出不见了"）：之前没有
 * 任何 ErrorBoundary，任何一处渲染期未捕获异常会让 React 把整棵树卸载，只剩一片
 * 白屏——从用户视角看和"程序不见了"没区别，而且什么线索都不会留下。这里只兜底
 * "渲染阶段抛出的异常"（React 的 componentDidCatch 只能捕获这个范围），事件回调里
 * 的异步异常靠 main.tsx 的 window.onerror/unhandledrejection 兜底。
 */
export class ErrorBoundary extends React.Component<React.PropsWithChildren, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    logFrontendError(error.message, `${error.stack ?? ""}\n${info.componentStack ?? ""}`, "ErrorBoundary");
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          height: "100vh",
          gap: 12,
          padding: 24,
          textAlign: "center",
          color: "var(--text-primary)",
          background: "var(--bg-base)",
        }}
      >
        <div style={{ fontSize: 16, fontWeight: 600 }}>界面出现了一个错误</div>
        <div style={{ fontSize: 12, color: "var(--text-secondary)", maxWidth: 480, wordBreak: "break-word" }}>
          {this.state.error.message}
        </div>
        <button className="btn primary sm" onClick={() => window.location.reload()}>
          重新加载
        </button>
      </div>
    );
  }
}
