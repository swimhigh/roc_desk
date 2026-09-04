import React, { useEffect, useState } from "react";
import { ChevronRight, Code2, FolderGit2, Laptop, Monitor, PenSquare, Plus, Server, TerminalSquare } from "lucide-react";
import { useModeStore, type WorkMode } from "../../stores/modeStore";
import { workspaceService } from "../../services/workspaceService";
import { connectionService } from "../../services/connectionService";
import { ThemeToggle } from "../shared/ThemeToggle";
import { formatError } from "../../utils/error";
import { useToastStore } from "../shared/Toast";
import type { ConnectionProfile, WorkspaceProfile } from "../../types/bindings";

interface ModuleCardProps {
  icon: React.ReactNode;
  title: string;
  description: string;
  onEnter: () => void;
  addLabel: string;
  onAdd: () => void;
  children: React.ReactNode;
}

/** 一个"工作模块"分区：大号入口（图标+标题+一句话说明，点哪里都能进模块）+
 * 下面一份该模块的最近/常用列表——入口和列表分开，用户第一眼先看懂"这是什么"，
 * 不用先扫一遍最近列表才能猜出这个分区是干什么的（用户反馈：模块入口要大气，
 * 首页要有基本的功能说明和指引）。 */
const ModuleCard: React.FC<ModuleCardProps> = ({ icon, title, description, onEnter, addLabel, onAdd, children }) => (
  <section className="home-dashboard-card">
    <button className="home-dashboard-entry" onClick={onEnter}>
      <span className="home-dashboard-entry-icon">{icon}</span>
      <span className="home-dashboard-entry-text">
        <span className="home-dashboard-entry-title">{title}</span>
        <span className="home-dashboard-entry-desc">{description}</span>
      </span>
      <ChevronRight className="home-dashboard-entry-arrow" />
    </button>
    <div className="home-dashboard-card-list">{children}</div>
    <button className="home-dashboard-card-add" onClick={onAdd}>
      <Plus size={13} /> {addLabel}
    </button>
  </section>
);

/**
 * 首页（启动器）：`docs/HOME_MODES_DESIGN.md` §3.4——不带 `--mode` 启动的进程只
 * 挂载这一个组件，不内嵌任何模块内容。点分区入口/「+」按钮 spawn 一个该模块的
 * 空白子进程；点分区里的具体项 spawn 子进程时带上"直接打开这一项"的参数。
 *
 * 首页本身不重新实现"新建连接"/"打开文件夹"这些管理功能——具体的增删改查还是
 * 在模块窗口内部用现成 UI 做，这里的按钮都只是"开一个新窗口"的快捷方式。
 */
export const HomeDashboard: React.FC = () => {
  const spawnModule = useModeStore((s) => s.spawnModule);
  const push = useToastStore((s) => s.push);
  const [workspaces, setWorkspaces] = useState<WorkspaceProfile[]>([]);
  const [connections, setConnections] = useState<ConnectionProfile[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [w, c] = await Promise.all([workspaceService.listRecent(6), connectionService.list()]);
        if (!cancelled) {
          setWorkspaces(w);
          setConnections(c.slice(0, 6));
        }
      } catch (e) {
        if (!cancelled) push("error", `加载首页列表失败：${formatError(e)}`);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const open = async (mode: WorkMode, id?: string) => {
    try {
      await spawnModule(mode, id);
    } catch (e) {
      push("error", `打开失败：${formatError(e)}`);
    }
  };

  return (
    <div className="home-dashboard">
      <ThemeToggle className="wp-theme-toggle" />
      <div className="home-dashboard-logo">
        <Code2 />
        roc_desk
      </div>
      <div className="home-dashboard-tagline">
        选一个模块开始工作——每个模块会在独立窗口中打开，可以同时开多个，随时用窗口里的"返回首页"按钮回到这里切换
      </div>

      <div className="home-dashboard-grid">
        <ModuleCard
          icon={<TerminalSquare />}
          title="SSH 桌面"
          description="类似 WindTerm / MobaXterm：管理服务器连接，随时打开终端、SFTP、RDP 远程桌面"
          onEnter={() => open("ssh")}
          addLabel="新建连接"
          onAdd={() => open("ssh")}
        >
          {loading ? null : connections.length === 0 ? (
            <div className="home-dashboard-card-empty">暂无已保存连接，点上方进入后新建</div>
          ) : (
            connections.map((c) => (
              <div key={c.id} className="home-dashboard-card-item" onClick={() => open("ssh")}>
                <Server size={14} />
                <span className="home-dashboard-item-name">{c.name}</span>
                <span className="home-dashboard-item-sub">{c.username}@{c.host}</span>
              </div>
            ))
          )}
        </ModuleCard>

        <ModuleCard
          icon={<FolderGit2 />}
          title="工作区"
          description="类似 VS Code：打开本地文件夹或远程目录，代码浏览、编辑、终端、AI 编程一体"
          onEnter={() => open("workspace")}
          addLabel="打开文件夹/连接"
          onAdd={() => open("workspace")}
        >
          {loading ? null : workspaces.length === 0 ? (
            <div className="home-dashboard-card-empty">暂无最近工作区，点上方进入后打开</div>
          ) : (
            workspaces.map((w) => (
              <div key={w.id} className="home-dashboard-card-item" onClick={() => open("workspace", w.id)}>
                {w.kind === "local" ? <Laptop size={14} /> : <Server size={14} />}
                <span className="home-dashboard-item-name">{w.display_name}</span>
                <span className="home-dashboard-item-sub">{w.root_path}</span>
              </div>
            ))
          )}
        </ModuleCard>

        <ModuleCard
          icon={<PenSquare />}
          title="编辑器"
          description="类似 UltraEdit：不依赖工作区，随时打开单个文件快速编辑"
          onEnter={() => open("editor")}
          addLabel="打开编辑器"
          onAdd={() => open("editor")}
        >
          <div className="home-dashboard-card-empty">进入后用 Ctrl+O 或拖拽打开文件</div>
        </ModuleCard>

        <ModuleCard
          icon={<Monitor />}
          title="资源管理器"
          description="类似 Total Commander：本地双栏浏览，复制/移动/删除/重命名文件"
          onEnter={() => open("explorer")}
          addLabel="打开资源管理器"
          onAdd={() => open("explorer")}
        >
          <div className="home-dashboard-card-empty">左右两栏各自浏览本地目录，可互相复制/移动文件</div>
        </ModuleCard>
      </div>
    </div>
  );
};
