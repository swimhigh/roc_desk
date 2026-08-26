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
  /** 断线/重连事件默认写回全局 `terminalStore`（工作区模式底部终端面板的既有行为）。
   * 远程工具模式的会话标签用的是独立的 `remoteSessionStore`（DESIGN.md §3.9，两套
   * 标签互不影响），传这两个回调覆盖默认行为，写回正确的 store，而不是复制一份
   * xterm 渲染逻辑出来。 */
  onDisconnected?: (id: string) => void;
  onReconnect?: (id: string, cwd?: string) => void;
  /** 每次用户在这个终端里敲字符都会额外调一次（在正常写入自己的 Channel 之后）——
   * 多路执行模式（DESIGN.md §3.9，参考 MobaXterm MultiExec）用它把输入转发给其它
   * 终端，TerminalView 本身不知道"多路执行"这个概念，只是单纯地上报"这里有输入"。*/
  onInput?: (id: string, data: Uint8Array) => void;
}

/**
 * 终端渲染（DESIGN.md §3.2）：xterm.js，SSH 和本地 PTY 两种 Tab 共用同一个组件——
 * 差别只在输入往哪个后端命令写、输出监听哪个事件（`ssh:data`/`ssh:status` vs
 * `pty:data`/`pty:status`），两边事件 payload 形状一致所以能共用同一套渲染逻辑。
 */
export const TerminalView: React.FC<TerminalViewProps> = ({ tab, cwd, onDisconnected, onReconnect, onInput }) => {
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
      fontFamily: "'Cascadia Mono', 'JetBrains Mono', 'Cascadia Code', Consolas, monospace",
      fontSize: 14,
      fontWeight: "400",
      fontWeightBold: "600",
      lineHeight: 1.22,
      letterSpacing: 0.15,
      theme: getTerminalTheme(useThemeStore.getState().theme, tab.kind),
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
      const bytes = new TextEncoder().encode(data);
      write(bytes);
      onInput?.(tab.id, bytes);
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
        (onDisconnected ?? useTerminalStore.getState().markDisconnected)(tab.id);
      }
    });

    return () => {
      onDataDisposable.dispose();
      resizeObserver.disconnect();
      unlistenDataPromise.then((unlisten) => unlisten());
      unlistenStatusPromise.then((unlisten) => unlisten());
      term.dispose();
    };
  }, [tab.id, tab.kind, tab.profileId, onDisconnected, onInput]);

  // 主题切换时更新配色但不重建整个终端实例，否则会丢掉当前的 scrollback 历史。
  useEffect(() => {
    return useThemeStore.subscribe((s) => {
      if (termRef.current) termRef.current.options.theme = getTerminalTheme(s.theme, tab.kind);
    });
  }, [tab.kind]);

  return (
    <div style={{ position: "relative", width: "100%", height: "100%" }}>
      <div ref={containerRef} style={{ width: "100%", height: "100%", padding: 8, background: "#20221F" }} />
      {tab.disconnected && (
        <div className="terminal-disconnected-overlay">
          <button
            className="btn primary sm"
            onClick={() => (onReconnect ?? useTerminalStore.getState().reconnectTerminal)(tab.id, cwd)}
          >
            <RefreshCw style={{ width: 14, height: 14 }} /> 重新连接
          </button>
        </div>
      )}
    </div>
  );
};
