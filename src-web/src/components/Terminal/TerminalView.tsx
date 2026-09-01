import React, { useEffect, useRef } from "react";
import { RefreshCw } from "lucide-react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen } from "@tauri-apps/api/event";
import "@xterm/xterm/css/xterm.css";
import { sshService } from "../../services/sshService";
import { ptyService } from "../../services/ptyService";
import { agentService } from "../../services/agentService";
import { useThemeStore } from "../../stores/themeStore";
import { useTerminalStore, type TerminalTab } from "../../stores/terminalStore";
import { useToastStore } from "../shared/Toast";
import { getTerminalTheme } from "../../utils/terminalTheme";
import { highlightTerminalChunk } from "../../utils/terminalHighlight";
import { formatError } from "../../utils/error";
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
 * 终端渲染（DESIGN.md §3.2）：xterm.js，SSH / Agent / 本地 PTY 三种 Tab 共用
 * 同一个组件——差别只在输入往哪个后端命令写、输出监听哪个事件
 * （`ssh:data`/`ssh:status` vs `agent:data`/`agent:status` vs `pty:data`/`pty:status`），
 * 三边事件 payload 形状一致所以能共用同一套渲染逻辑。
 */
export const TerminalView: React.FC<TerminalViewProps> = ({ tab, cwd, onDisconnected, onReconnect, onInput }) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const write = (data: Uint8Array) =>
      tab.kind === "ssh"
        ? sshService.write(tab.profileId!, tab.id, data)
        : tab.kind === "agent"
          ? agentService.write(tab.profileId!, tab.id, data)
          : ptyService.write(tab.id, data);
    const resize = (rows: number, cols: number) =>
      tab.kind === "ssh"
        ? sshService.resize(tab.profileId!, tab.id, rows, cols)
        : tab.kind === "agent"
          ? agentService.resize(tab.profileId!, tab.id, rows, cols)
          : ptyService.resize(tab.id, rows, cols);
    const dataEvent = tab.kind === "ssh" ? "ssh:data" : tab.kind === "agent" ? "agent:data" : "pty:data";
    const statusEvent = tab.kind === "ssh" ? "ssh:status" : tab.kind === "agent" ? "agent:status" : "pty:status";

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

    // {stream: true} + 复用同一个 decoder 实例：一个多字节 UTF-8 字符（中文很常见）
    // 被切在两次数据事件的边界上时，decoder 会记住上一次没解完的尾部字节，下一次
    // 调用自动拼上，不会解出乱码替换符——和 xterm 直接喂 Uint8Array 时内部解码器
    // 的行为等价，这里只是在喂给 xterm 之前多插一步高亮处理（见
    // utils/terminalHighlight.ts 顶部注释的取舍说明）。
    const decoder = new TextDecoder();
    const unlistenDataPromise = listen<SshDataEvent>(dataEvent, (event) => {
      if (event.payload.channelId !== tab.id) return;
      const text = decoder.decode(new Uint8Array(event.payload.data), { stream: true });
      term.write(highlightTerminalChunk(text));
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
      {/* 背景色必须和 terminalTheme.ts 里 Dracula 主题的 background 完全一致——
          这层 padding 容器是 xterm 画布外的留白，颜色对不上会在边缘露出一圈
          不一样的深色。 */}
      <div ref={containerRef} style={{ width: "100%", height: "100%", padding: 8, background: "#282A36" }} />
      {tab.disconnected && (
        <div className="terminal-disconnected-overlay">
          <button
            className="btn primary sm"
            onClick={() => {
              // 之前这里点了没反应（真实反馈）：重连失败时 Promise reject 没人接，
              // 按钮"点了跟没点一样"，连接池里缓存的死连接一直废在那，不重启整个
              // app 都连不上——现在池子那边已经会自动识别并清掉死连接重连，这里
              // 补上失败提示，万一还是连不上（比如目标真的不可达）用户能看到原因。
              const reconnect = onReconnect ?? useTerminalStore.getState().reconnectTerminal;
              Promise.resolve(reconnect(tab.id, cwd)).catch((e) => {
                useToastStore.getState().push("error", `重新连接失败：${formatError(e)}`);
              });
            }}
          >
            <RefreshCw style={{ width: 14, height: 14 }} /> 重新连接
          </button>
        </div>
      )}
    </div>
  );
};
