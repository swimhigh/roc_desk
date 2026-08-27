import React, { useRef } from "react";
import { Plus, X, ChevronDown, TerminalSquare } from "lucide-react";
import { useTerminalStore } from "../../stores/terminalStore";
import { TerminalView } from "./TerminalView";
import { SshHostStatsBar } from "../RemoteTool/SshHostStatsBar";

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
  const {
    allTabs,
    currentWorkspaceId,
    tabs,
    activeId,
    panelOpen,
    panelHeight,
    openTerminal,
    closeTerminal,
    setActive,
    setPanelOpen,
    setPanelHeight,
  } = useTerminalStore();
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

  // 折叠面板不能 `return null`——那会把所有工作区、所有 Tab 的 TerminalView 全部
  // 卸载，销毁它们各自的 xterm.js 实例（连同 scrollback 一起丢失）。改成纯 CSS
  // 折叠（高度 0 + 溢出隐藏），渲染树保持不变，和下面"跨工作区保活"用的是
  // 同一个原则：只隐藏，不摘除。
  return (
    <div
      className="terminal-panel"
      style={{
        height: panelOpen ? panelHeight : 0,
        // `.terminal-panel` 的 CSS 类里有 `min-height: 140px`（保证正常展开时不会被
        // 拖得太矮），折叠时必须显式覆盖掉，否则光设 `height: 0` 不生效——
        // min-height 仍然会把它撑到 140px。
        minHeight: panelOpen ? undefined : 0,
        overflow: panelOpen ? undefined : "hidden",
      }}
    >
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
        {/* 这里必须遍历 `allTabs`（所有仍保活的工作区的全部 Tab），不能只遍历
            `tabs`（当前工作区的 Tab）——否则切到另一个工作区时，这个工作区的
            Tab 会从渲染树里整个消失，对应的 TerminalView 被卸载、xterm.js 实例
            连同 scrollback 一起销毁；切回来看到的就是一个内容空白的新终端，
            这正是"终端会话保持"功能实际复现的 bug（后端 Channel 确实还活着——
            资源使用率状态栏能正常轮询就是证据——但前端渲染状态早没了，
            后端也不会替你重放历史输出）。只对不是"当前工作区 + 当前激活 Tab"
            的项目用 CSS 隐藏，渲染树本身保持不变。 */}
        {allTabs.map((tab) => {
          const isVisible = tab.workspaceId === currentWorkspaceId && tab.id === activeId;
          return (
            <div
              key={tab.id}
              style={{ width: "100%", height: "100%", display: isVisible ? "flex" : "none", flexDirection: "column" }}
            >
              <div style={{ flex: 1, minHeight: 0 }}>
                <TerminalView tab={tab} cwd={target.cwd} />
              </div>
              {/* 资源使用率状态栏（参考远程工具模式 SSH 会话下方的同款组件）：只有
                  远程 SSH 终端才有意义——本地 PTY 是当前这台机器自己，探针脚本本身
                  也是针对 Linux/Unix 远程主机写的（ssh/monitor.rs 顶部注释）。 */}
              {tab.kind === "ssh" && <SshHostStatsBar profileId={tab.profileId!} active={isVisible} />}
            </div>
          );
        })}
      </div>
    </div>
  );
};
