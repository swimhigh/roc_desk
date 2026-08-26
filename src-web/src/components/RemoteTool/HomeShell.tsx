import React, { useEffect, useMemo, useState } from "react";
import { X, Terminal as TerminalIcon, FolderCog, Monitor, FolderGit2, Rows3, LogOut } from "lucide-react";
import { SessionTree } from "./SessionTree";
import { SshHostStatsBar } from "./SshHostStatsBar";
import { RdpView } from "./RdpView";
import { TerminalView } from "../Terminal/TerminalView";
import { SftpBrowser } from "../SftpBrowser/SftpBrowser";
import { SftpFileViewer } from "../SftpBrowser/SftpFileViewer";
import { WorkspacePicker } from "../Workspace/WorkspacePicker";
import { useRemoteSessionStore, type RemoteSessionTab } from "../../stores/remoteSessionStore";
import { useSessionTreeStore } from "../../stores/sessionTreeStore";
import type { ConnectionProfile, FileEntry } from "../../types/bindings";

function defaultRemotePath(username: string): string {
  return username.startsWith("root") ? "/root" : `/home/${username}`;
}

const TAB_ICON: Record<RemoteSessionTab["kind"], React.ReactNode> = {
  "ssh-terminal": <TerminalIcon />,
  sftp: <FolderCog />,
  rdp: <Monitor />,
};

/**
 * 首页外壳（DESIGN.md §3.9，用户 2026-08-25 明确要求："首页应该是左边列出所有会话
 * （可以新建），右边和原版本一样列出所有工作区（可以新建）。不需要去选会话模式和
 * 工作区模式后再展现"）——左侧会话树永远在，右侧默认显示原样的 WorkspacePicker，
 * 点开一个会话就多一个可关闭的标签（标签栏里"工作区"是固定首位、不可关闭的入口，
 * 不是又一个可关的会话标签）。不存在需要提前选择的"模式"，两边一直都在。
 */
export const HomeShell: React.FC = () => {
  const tabs = useRemoteSessionStore((s) => s.tabs);
  const activeId = useRemoteSessionStore((s) => s.activeId);
  const setActive = useRemoteSessionStore((s) => s.setActive);
  const closeTab = useRemoteSessionStore((s) => s.closeTab);
  const markDisconnected = useRemoteSessionStore((s) => s.markDisconnected);
  const reconnectSshTerminal = useRemoteSessionStore((s) => s.reconnectSshTerminal);
  const openSshTerminal = useRemoteSessionStore((s) => s.openSshTerminal);
  const openSftp = useRemoteSessionStore((s) => s.openSftp);
  const multiExecEnabled = useRemoteSessionStore((s) => s.multiExecEnabled);
  const multiExecExcluded = useRemoteSessionStore((s) => s.multiExecExcluded);
  const toggleMultiExec = useRemoteSessionStore((s) => s.toggleMultiExec);
  const toggleExcludedFromMultiExec = useRemoteSessionStore((s) => s.toggleExcludedFromMultiExec);
  const broadcastInput = useRemoteSessionStore((s) => s.broadcastInput);
  const connections = useSessionTreeStore((s) => s.connections);

  const [showPicker, setShowPicker] = useState(true);
  const [viewingFile, setViewingFile] = useState<Record<string, string | null>>({});

  // 从会话树打开/激活任何一个会话标签都应该自动切离首页——用户点了就是想看它，
  // 不需要再手动点一下标签才能看到内容；反过来，关掉最后一个标签（activeId 变回
  // null）也该自动回到首页，不留在一个"什么都没有"的空白状态里。
  useEffect(() => {
    setShowPicker(!activeId);
  }, [activeId]);

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
          <div className={`remote-session-tab ${showPicker ? "active" : ""}`} onClick={() => setShowPicker(true)}>
            <FolderGit2 />
            <span>工作区</span>
          </div>
          {tabs.map((tab) => (
            <div
              key={tab.id}
              className={`remote-session-tab ${!showPicker && tab.id === activeId ? "active" : ""}`}
              onClick={() => {
                setActive(tab.id);
                setShowPicker(false);
              }}
            >
              {TAB_ICON[tab.kind]}
              <span>{tab.title}</span>
              {tab.kind === "ssh-terminal" && tab.disconnected && <span className="tab-dot connecting" />}
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

        {showPicker ? (
          <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
            <WorkspacePicker />
          </div>
        ) : (
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
                <div className="remote-session-empty">在左侧会话树里双击一个连接，开始一个 SSH / RDP / SFTP 会话</div>
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
        )}
      </div>
    </div>
  );
};
