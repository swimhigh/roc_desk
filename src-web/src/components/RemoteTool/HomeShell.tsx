import React, { useMemo, useState } from "react";
import { X, Terminal as TerminalIcon, FolderCog, Home, Monitor, Rows3, LogOut, HardDrive, ArrowLeftRight } from "lucide-react";
import { SessionTree } from "./SessionTree";
import { SshHostStatsBar } from "./SshHostStatsBar";
import { RdpView } from "./RdpView";
import { TerminalView } from "../Terminal/TerminalView";
import { SftpBrowser } from "../SftpBrowser/SftpBrowser";
import { SftpFileViewer } from "../SftpBrowser/SftpFileViewer";
import { AgentBrowser } from "../SftpBrowser/AgentBrowser";
import { useRemoteSessionStore, type RemoteSessionTab } from "../../stores/remoteSessionStore";
import { useSessionTreeStore } from "../../stores/sessionTreeStore";
import { useModeStore } from "../../stores/modeStore";
import type { ConnectionProfile, FileEntry } from "../../types/bindings";

function defaultRemotePath(username: string): string {
  return username.startsWith("root") ? "/root" : `/home/${username}`;
}

const TAB_ICON: Record<RemoteSessionTab["kind"], React.ReactNode> = {
  "ssh-terminal": <TerminalIcon />,
  "agent-terminal": <HardDrive />,
  sftp: <FolderCog />,
  "agent-browse": <ArrowLeftRight />,
  rdp: <Monitor />,
};

/**
 * SSH 桌面模块（`docs/HOME_MODES_DESIGN.md` §3.2/§3.5）——`--mode=ssh` 启动的
 * 模块窗口只挂载这一个组件：左侧会话树 + 右侧会话标签（SSH/Agent 终端、SFTP、
 * 文件传输、RDP）。原本（DESIGN.md §3.9）这个组件右侧还内嵌一份 `WorkspacePicker`
 * 充当"整个应用的首页"，现在"打开工作区"已经拆成独立的 `workspace` 模块窗口，
 * 这里不再需要那部分——标签栏最左边的固定入口从"工作区"改成"返回首页"，点了是
 * `useModeStore.goHome()`（唤起/聚焦启动器进程），不是应用内部切视图。
 */
export const HomeShell: React.FC = () => {
  const tabs = useRemoteSessionStore((s) => s.tabs);
  const activeId = useRemoteSessionStore((s) => s.activeId);
  const setActive = useRemoteSessionStore((s) => s.setActive);
  const closeTab = useRemoteSessionStore((s) => s.closeTab);
  const markDisconnected = useRemoteSessionStore((s) => s.markDisconnected);
  const reconnectSshTerminal = useRemoteSessionStore((s) => s.reconnectSshTerminal);
  const reconnectAgentTerminal = useRemoteSessionStore((s) => s.reconnectAgentTerminal);
  const openSshTerminal = useRemoteSessionStore((s) => s.openSshTerminal);
  const openAgentTerminal = useRemoteSessionStore((s) => s.openAgentTerminal);
  const openSftp = useRemoteSessionStore((s) => s.openSftp);
  const openAgentBrowse = useRemoteSessionStore((s) => s.openAgentBrowse);
  const multiExecEnabled = useRemoteSessionStore((s) => s.multiExecEnabled);
  const multiExecExcluded = useRemoteSessionStore((s) => s.multiExecExcluded);
  const toggleMultiExec = useRemoteSessionStore((s) => s.toggleMultiExec);
  const toggleExcludedFromMultiExec = useRemoteSessionStore((s) => s.toggleExcludedFromMultiExec);
  const broadcastInput = useRemoteSessionStore((s) => s.broadcastInput);
  const connections = useSessionTreeStore((s) => s.connections);
  const goHome = useModeStore((s) => s.goHome);

  const [viewingFile, setViewingFile] = useState<Record<string, string | null>>({});

  const [sidebarWidth, setSidebarWidth] = useState(() => {
    const stored = Number(localStorage.getItem("roc_desk-remote-sidebar-width"));
    return stored >= 160 && stored <= 600 ? stored : 240;
  });
  const onSidebarDragStart = (e: React.MouseEvent) => {
    const startX = e.clientX;
    const startWidth = sidebarWidth;
    let latest = startWidth;
    const onMove = (ev: MouseEvent) => {
      latest = Math.max(160, Math.min(600, startWidth + (ev.clientX - startX)));
      setSidebarWidth(latest);
    };
    const onUp = () => {
      localStorage.setItem("roc_desk-remote-sidebar-width", String(latest));
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const sshTerminalTabs = useMemo(() => tabs.filter((t): t is Extract<RemoteSessionTab, { kind: "ssh-terminal" }> => t.kind === "ssh-terminal"), [tabs]);

  const profileOf = (id: string): ConnectionProfile | undefined => connections.find((c) => c.id === id);

  /** 会话内的"SFTP"/"终端"快捷按钮（用户 2026-08-25 需求："远程会话模式进入SSH界面
   * 后，应该像工作区模式一样，上面有一个SFTP的按钮"）——同一台服务器已经开着的
   * SFTP/终端标签就直接切过去，没有才新开一个，不会每点一次都堆一个新标签。 */
  const jumpToSftp = (profileId: string) => {
    const existing = tabs.find((t) => t.kind === "sftp" && t.profileId === profileId);
    if (existing) {
      setActive(existing.id);
    } else {
      const profile = profileOf(profileId);
      if (profile) openSftp(profile);
    }
  };
  const jumpToTerminal = (profileId: string) => {
    const existing = tabs.find((t) => t.kind === "ssh-terminal" && t.profileId === profileId);
    if (existing) {
      setActive(existing.id);
    } else {
      const profile = profileOf(profileId);
      if (profile) void openSshTerminal(profile);
    }
  };
  const jumpToAgentBrowse = (profileId: string) => {
    const existing = tabs.find((t) => t.kind === "agent-browse" && t.profileId === profileId);
    if (existing) {
      setActive(existing.id);
    } else {
      const profile = profileOf(profileId);
      if (profile) openAgentBrowse(profile);
    }
  };
  const jumpToAgentTerminal = (profileId: string) => {
    const existing = tabs.find((t) => t.kind === "agent-terminal" && t.profileId === profileId);
    if (existing) {
      setActive(existing.id);
    } else {
      const profile = profileOf(profileId);
      if (profile) void openAgentTerminal(profile);
    }
  };

  return (
    <div style={{ display: "flex", height: "100vh", overflow: "hidden" }}>
      <div
        style={{
          width: sidebarWidth,
          flexShrink: 0,
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          background: "var(--bg-surface)",
        }}
      >
        <SessionTree />
      </div>
      <div className="sidebar-resize-handle" onMouseDown={onSidebarDragStart} />

      <div className="remote-tool-shell">
        <div className="remote-session-tabs">
          <div className="remote-session-tab" onClick={() => void goHome()} title="唤起/聚焦首页进程">
            <Home />
            <span>首页</span>
          </div>
          {tabs.map((tab) => (
            <div
              key={tab.id}
              className={`remote-session-tab ${tab.id === activeId ? "active" : ""}`}
              onClick={() => setActive(tab.id)}
            >
              {TAB_ICON[tab.kind]}
              <span>{tab.title}</span>
              {(tab.kind === "ssh-terminal" || tab.kind === "agent-terminal") && tab.disconnected && <span className="tab-dot connecting" />}
              <button
                className="remote-session-tab-close"
                title="关闭"
                onClick={(e) => {
                  e.stopPropagation();
                  void closeTab(tab.id);
                }}
              >
                <X />
              </button>
            </div>
          ))}
          {sshTerminalTabs.length >= 2 && (
            <button
              className={`quick-tool-btn ${multiExecEnabled ? "active" : ""}`}
              style={{ marginLeft: "auto", flexShrink: 0 }}
              title={multiExecEnabled ? "退出多路执行模式" : "多路执行模式：输入同步到所有 SSH 终端（参考 MobaXterm MultiExec）"}
              onClick={toggleMultiExec}
            >
              <Rows3 />
            </button>
          )}
        </div>

        <>
            {/* SFTP/RDP 标签（尤其是 RDP——内嵌的是真实进程/原生窗口，不是能随便丢的
                画面）不能因为切进/切出多路执行模式就被卸载重建，所以这块和下面的
                grid 是两棵并存的子树，用 display 切换可见性，不是互斥渲染。ssh-terminal
                标签是例外：多路执行模式下它们已经在 grid 里挂了一份 TerminalView，
                这里对应位置要跳过，否则同一个 Channel 会有两个 xterm 实例同时收
                ssh:data 事件。 */}
            {multiExecEnabled && (
              <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
                <div className="host-stats-bar" style={{ borderTop: "none", borderBottom: "1px solid var(--border-default)" }}>
                  <strong>多路执行模式</strong>：在任意未排除的终端里输入，会同步发给其它未排除的终端
                  <button className="btn ghost sm" style={{ marginLeft: "auto" }} onClick={toggleMultiExec}>
                    <LogOut style={{ width: 12, height: 12 }} /> 退出
                  </button>
                </div>
                <div className="multiexec-grid">
                  {sshTerminalTabs.map((tab) => (
                    <div key={tab.id} className="multiexec-cell">
                      <div className="multiexec-cell-header">
                        <TerminalIcon />
                        <span>{tab.title}</span>
                        <label className="multiexec-exclude">
                          <input
                            type="checkbox"
                            checked={multiExecExcluded.has(tab.id)}
                            onChange={() => toggleExcludedFromMultiExec(tab.id)}
                          />
                          不参与广播
                        </label>
                      </div>
                      <div style={{ flex: 1, minHeight: 0 }}>
                        <TerminalView
                          tab={{ id: tab.id, kind: "ssh", profileId: tab.profileId, title: tab.title, disconnected: tab.disconnected }}
                          onDisconnected={markDisconnected}
                          onReconnect={(id) => void reconnectSshTerminal(id)}
                          onInput={(id, data) => broadcastInput(id, data)}
                        />
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            <div style={{ flex: 1, minHeight: 0, display: multiExecEnabled ? "none" : "flex", flexDirection: "column", overflow: "hidden" }}>
              {tabs.length === 0 ? (
                <div className="remote-session-empty">在左侧会话树里双击一个连接，开始一个 SSH / Agent / RDP / SFTP 会话</div>
              ) : (
                tabs.map((tab) => {
                  if (multiExecEnabled && tab.kind === "ssh-terminal") return null;
                  const profile = profileOf(tab.profileId);
                  const isActive = tab.id === activeId && !multiExecEnabled;
                  return (
                    <div key={tab.id} style={{ flex: 1, minHeight: 0, display: isActive ? "flex" : "none", flexDirection: "column", overflow: "hidden" }}>
                      {tab.kind === "ssh-terminal" && (
                        <>
                          <div className="remote-session-subbar">
                            <button className="quick-tool-btn" title="打开这台服务器的 SFTP" onClick={() => jumpToSftp(tab.profileId)}>
                              <FolderCog /> <span>SFTP</span>
                            </button>
                          </div>
                          <div style={{ flex: 1, minHeight: 0 }}>
                            <TerminalView
                              tab={{ id: tab.id, kind: "ssh", profileId: tab.profileId, title: tab.title, disconnected: tab.disconnected }}
                              onDisconnected={markDisconnected}
                              onReconnect={(id) => void reconnectSshTerminal(id)}
                            />
                          </div>
                          <SshHostStatsBar profileId={tab.profileId} active={isActive} />
                        </>
                      )}
                      {tab.kind === "agent-terminal" && (
                        <>
                          <div className="remote-session-subbar">
                            <button className="quick-tool-btn" title="打开这台服务器的文件传输" onClick={() => jumpToAgentBrowse(tab.profileId)}>
                              <ArrowLeftRight /> <span>文件传输</span>
                            </button>
                          </div>
                          <div style={{ flex: 1, minHeight: 0 }}>
                            <TerminalView
                              tab={{ id: tab.id, kind: "agent", profileId: tab.profileId, title: tab.title, disconnected: tab.disconnected }}
                              onDisconnected={markDisconnected}
                              onReconnect={(id) => void reconnectAgentTerminal(id)}
                            />
                          </div>
                        </>
                      )}
                      {tab.kind === "agent-browse" && (
                        <>
                          <div className="remote-session-subbar">
                            <button className="quick-tool-btn" title="打开这台服务器的终端" onClick={() => jumpToAgentTerminal(tab.profileId)}>
                              <HardDrive /> <span>终端</span>
                            </button>
                          </div>
                          <div style={{ flex: 1, minHeight: 0 }}>
                            <AgentBrowser profileId={tab.profileId} workspaceId={tab.profileId} rememberRemotePath />
                          </div>
                        </>
                      )}
                      {tab.kind === "sftp" && (
                        <>
                          <div className="remote-session-subbar">
                            <button className="quick-tool-btn" title="打开这台服务器的终端" onClick={() => jumpToTerminal(tab.profileId)}>
                              <TerminalIcon /> <span>终端</span>
                            </button>
                          </div>
                          <div style={{ flex: 1, minHeight: 0 }}>
                            {viewingFile[tab.id] ? (
                              <SftpFileViewer
                                profileId={tab.profileId}
                                path={viewingFile[tab.id]!}
                                onBack={() => setViewingFile((s) => ({ ...s, [tab.id]: null }))}
                              />
                            ) : (
                              <SftpBrowser
                                profileId={tab.profileId}
                                workspaceId={tab.profileId}
                                initialRemotePath={defaultRemotePath(profile?.username ?? "")}
                                rememberRemotePath
                                onOpenFile={(entry: FileEntry) => setViewingFile((s) => ({ ...s, [tab.id]: entry.path }))}
                              />
                            )}
                          </div>
                        </>
                      )}
                      {tab.kind === "rdp" && profile && <RdpView profile={profile} visible={isActive} />}
                    </div>
                  );
                })
              )}
            </div>
          </>
      </div>
    </div>
  );
};
