import React, { useRef } from "react";
import { Plus, X, ChevronDown, TerminalSquare } from "lucide-react";
import { useTerminalStore } from "../../stores/terminalStore";
import { TerminalView } from "./TerminalView";

interface TerminalPanelProps {
  /** 本地工作区传 { kind: 'local' }，远程工作区传 { kind: 'ssh', profileId }；
   * `cwd` 是工作区根目录，新开的终端默认停在这里（参考 VS Code）。*/
  target: { kind: "local"; cwd: string } | { kind: "ssh"; profileId: string; cwd: string };
}

/**
 * VS Code 风格的底部终端面板：停靠在编辑器下方，可折叠、可拖拽调整高度，
 * 支持在同一个工作区里开多个终端 Tab（远程工作区对应 DESIGN.md §3.2.2 多路复用——
 * 每个 Tab 都是同一条 SSH 连接上的独立 Channel；本地工作区每个 Tab 是独立的本地 PTY 进程）。
 */
export const TerminalPanel: React.FC<TerminalPanelProps> = ({ target }) => {
  const { tabs, activeId, panelOpen, panelHeight, openTerminal, closeTerminal, setActive, setPanelOpen, setPanelHeight } =
    useTerminalStore();
  const dragRef = useRef<{ startY: number; startHeight: number } | null>(null);

  const onDragStart = (e: React.MouseEvent) => {
    dragRef.current = { startY: e.clientY, startHeight: panelHeight };
    const onMove = (ev: MouseEvent) => {
      if (!dragRef.current) return;
      const delta = dragRef.current.startY - ev.clientY;
      setPanelHeight(dragRef.current.startHeight + delta);
    };
    const onUp = () => {
      dragRef.current = null;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const openNew = () =>
    target.kind === "ssh"
      ? openTerminal({ kind: "ssh", profileId: target.profileId, cwd: target.cwd })
      : openTerminal({ kind: "local", cwd: target.cwd });

  if (!panelOpen) return null;

  return (
    <div className="terminal-panel" style={{ height: panelHeight }}>
      <div className="terminal-panel-resize-handle" onMouseDown={onDragStart} />
      <div className="terminal-panel-header">
        <div className="terminal-panel-tabs">
          {tabs.map((tab) => (
            <div
              key={tab.id}
              className={`terminal-tab ${tab.id === activeId ? "active" : ""} ${tab.disconnected ? "disconnected" : ""}`}
              onClick={() => setActive(tab.id)}
              title={tab.disconnected ? "连接已断开" : undefined}
            >
              <TerminalSquare className="tab-icon" />
              <span>{tab.title}</span>
              {tab.disconnected && <span className="terminal-tab-dot" />}
              <button
                className="terminal-tab-close"
                title="关闭终端"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTerminal(tab.id);
                }}
              >
                <X />
              </button>
            </div>
          ))}
          <button className="terminal-panel-add" title="新建终端" onClick={openNew}>
            <Plus />
          </button>
        </div>
        <button className="terminal-panel-collapse" title="收起终端面板" onClick={() => setPanelOpen(false)}>
          <ChevronDown />
        </button>
      </div>
      <div className="terminal-panel-body">
        {tabs.map((tab) => (
          <div key={tab.id} style={{ width: "100%", height: "100%", display: tab.id === activeId ? "block" : "none" }}>
            <TerminalView tab={tab} cwd={target.cwd} />
          </div>
        ))}
      </div>
    </div>
  );
};
