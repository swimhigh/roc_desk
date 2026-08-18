import React, { useEffect, useRef } from "react";
import { RefreshCw } from "lucide-react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen } from "@tauri-apps/api/event";
import "@xterm/xterm/css/xterm.css";
import { sshService } from "../../services/sshService";
import { ptyService } from "../../services/ptyService";
import { useThemeStore } from "../../stores/themeStore";
import { useTerminalStore, type TerminalTab } from "../../stores/terminalStore";
import { getTerminalTheme } from "../../utils/terminalTheme";
import type { SshDataEvent, SshStatusEvent } from "../../types/bindings";

interface TerminalViewProps {
  tab: TerminalTab;
  /** 断线重连时用来重新 cd 进同一个目录（DESIGN.md §3.2 默认工作区目录）。*/
  cwd?: string;
}

/**
 * 终端渲染（DESIGN.md §3.2）：xterm.js，SSH 和本地 PTY 两种 Tab 共用同一个组件——
 * 差别只在输入往哪个后端命令写、输出监听哪个事件（`ssh:data`/`ssh:status` vs
 * `pty:data`/`pty:status`），两边事件 payload 形状一致所以能共用同一套渲染逻辑。
 */
export const TerminalView: React.FC<TerminalViewProps> = ({ tab, cwd }) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const write = (data: Uint8Array) =>
      tab.kind === "ssh" ? sshService.write(tab.profileId!, tab.id, data) : ptyService.write(tab.id, data);
    const resize = (rows: number, cols: number) =>
      tab.kind === "ssh" ? sshService.resize(tab.profileId!, tab.id, rows, cols) : ptyService.resize(tab.id, rows, cols);
    const dataEvent = tab.kind === "ssh" ? "ssh:data" : "pty:data";
    const statusEvent = tab.kind === "ssh" ? "ssh:status" : "pty:status";

    const term = new Terminal({
      fontFamily: "'JetBrains Mono', 'Cascadia Code', Consolas, 'Courier New', monospace",
      fontSize: 14,
      lineHeight: 1.25,
      letterSpacing: 0.3,
      theme: getTerminalTheme(useThemeStore.getState().theme),
      cursorBlink: true,
      cursorStyle: "bar",
      cursorWidth: 2,
      scrollback: 5000,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(containerRef.current);
    fitAddon.fit();
    termRef.current = term;

    const onDataDisposable = term.onData((data) => {
      write(new TextEncoder().encode(data));
    });

    const resizeObserver = new ResizeObserver(() => {
      fitAddon.fit();
      resize(term.rows, term.cols);
    });
    resizeObserver.observe(containerRef.current);

    const unlistenDataPromise = listen<SshDataEvent>(dataEvent, (event) => {
      if (event.payload.channelId !== tab.id) return;
      term.write(new Uint8Array(event.payload.data));
    });

    // 之前这个事件后端一直在发，前端从没听过——Channel 断开时终端就是"安静下来了"，
    // 用户分不清是命令没输出还是连接已经死了（真实反馈的间接成因之一）。
    const unlistenStatusPromise = listen<SshStatusEvent>(statusEvent, (event) => {
      if (event.payload.channelId !== tab.id) return;
      if (event.payload.status === "disconnected") {
        term.write("\r\n\x1b[31m[连接已断开]\x1b[0m\r\n");
        useTerminalStore.getState().markDisconnected(tab.id);
      }
    });

    return () => {
      onDataDisposable.dispose();
      resizeObserver.disconnect();
      unlistenDataPromise.then((unlisten) => unlisten());
      unlistenStatusPromise.then((unlisten) => unlisten());
      term.dispose();
    };
  }, [tab.id, tab.kind, tab.profileId]);

  // 主题切换时更新配色但不重建整个终端实例，否则会丢掉当前的 scrollback 历史。
  useEffect(() => {
    return useThemeStore.subscribe((s) => {
      if (termRef.current) termRef.current.options.theme = getTerminalTheme(s.theme);
    });
  }, []);

  return (
    <div style={{ position: "relative", width: "100%", height: "100%" }}>
      <div ref={containerRef} style={{ width: "100%", height: "100%", padding: 8, background: "var(--bg-terminal, #0c0d0e)" }} />
      {tab.disconnected && (
        <div className="terminal-disconnected-overlay">
          <button className="btn primary sm" onClick={() => useTerminalStore.getState().reconnectTerminal(tab.id, cwd)}>
            <RefreshCw style={{ width: 14, height: 14 }} /> 重新连接
          </button>
        </div>
      )}
    </div>
  );
};
