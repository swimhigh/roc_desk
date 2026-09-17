import React, { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ChevronRight, Code2, Database, FolderGit2, GripVertical, Laptop, Minus, Monitor, PenSquare, Plus, Send, Server, TerminalSquare } from "lucide-react";
import { useModeStore, type WorkMode } from "../../stores/modeStore";
import { workspaceService } from "../../services/workspaceService";
import { connectionService } from "../../services/connectionService";
import { sqlService } from "../../services/sqlService";
import { ThemeToggle } from "../shared/ThemeToggle";
import { formatError } from "../../utils/error";
import { useToastStore } from "../shared/Toast";
import type { ConnectionProfile, DataSourceProfile, WorkspaceProfile } from "../../types/bindings";

interface ModuleCardProps {
  id: string;
  icon: React.ReactNode;
  title: string;
  description: string;
  onEnter: () => void;
  addLabel: string;
  onAdd: () => void;
  children: React.ReactNode;
  dragging: boolean;
  dragOver: boolean;
  onDragStart: (id: string, e: React.PointerEvent) => void;
  onDragMove: (e: React.PointerEvent) => void;
  onDragEnd: (e: React.PointerEvent) => void;
}

/** 一个"工作模块"分区：大号入口（图标+标题+一句话说明，点哪里都能进模块）+
 * 下面一份该模块的最近/常用列表——入口和列表分开，用户第一眼先看懂"这是什么"，
 * 不用先扫一遍最近列表才能猜出这个分区是干什么的（用户反馈：模块入口要大气，
 * 首页要有基本的功能说明和指引）。顶部整条"拖动排序"横条可以拖拽调整分区顺序。
 *
 * 2026-09 用户反馈"完全拖不动，一点效果也没有"——根因是这个应用依赖 Tauri
 * 默认开启的 `dragDropEnabled`（双栏文件浏览器拖外部文件进来要用，见
 * `useDualPaneDnd.ts`），WebView2 在这个模式下会整体接管原生 OS 拖拽手势，
 * HTML5 `draggable`/`dragstart` 系列事件根本不会触发——不是没写好，是原生
 * 拖拽 API 在这个环境里走不通，之前那版看起来"什么反应都没有"就是因为
 * `dragstart` 从来没发生过。改用纯指针事件（`onPointerDown/Move/Up` +
 * `setPointerCapture`）自己实现拖拽，和 `SqlWorkspace.tsx` 拖动分栏宽度是
 * 同一套机制，不经过浏览器原生拖拽 API，因此不受 `dragDropEnabled` 影响。
 * 父组件用 `document.elementFromPoint` 配合 `data-module-id` 做命中测试。 */
const ModuleCard: React.FC<ModuleCardProps> = ({ id, icon, title, description, onEnter, addLabel, onAdd, children, dragging, dragOver, onDragStart, onDragMove, onDragEnd }) => (
  <section className={`home-dashboard-card ${dragging ? "dragging" : ""} ${dragOver ? "drag-over" : ""}`} data-module-id={id}>
    <div
      className="home-dashboard-drag-handle"
      title="按住拖动，调整模块位置"
      style={{ touchAction: "none" }}
      onPointerDown={(e) => onDragStart(id, e)}
      onPointerMove={onDragMove}
      onPointerUp={onDragEnd}
      onPointerCancel={onDragEnd}
    >
      <GripVertical size={13} /> 拖动排序
    </div>
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

const EMPTY_SLOT_ID = "__empty__";
const MODULE_ORDER_KEY = "home-dashboard-module-order";
const EXTRA_SLOTS_KEY = "home-dashboard-extra-slots";
const DEFAULT_MODULE_ORDER = ["ssh", "workspace", "editor", "sql", "http", "explorer"];
const MIN_EXTRA_SLOTS = 0;
const MAX_EXTRA_SLOTS = 9;

function loadModuleOrder(): string[] {
  try {
    const raw = localStorage.getItem(MODULE_ORDER_KEY);
    if (!raw) return DEFAULT_MODULE_ORDER;
    const saved = (JSON.parse(raw) as string[]).filter((id) => DEFAULT_MODULE_ORDER.includes(id));
    // 兼容以后新增模块：已保存的顺序里没有的新模块 id，追加在末尾，不会因为
    // 用户之前保存过一份旧顺序就永远看不到新模块。
    const missing = DEFAULT_MODULE_ORDER.filter((id) => !saved.includes(id));
    return [...saved, ...missing];
  } catch {
    return DEFAULT_MODULE_ORDER;
  }
}

/** 网格里额外空白格的数量（2026-09 用户反馈"格子数量也可以自己加"）——默认 0，
 * 用户可以点"+"最多加到 9 格，凑成一个名副其实的九宫格，给以后新增模块或者
 * 单纯想把卡片摆得松散一点留出空间。空白格本身不携带身份，拖模块到任意一个
 * 空白格上都等价于"挪到最后"，加减格子只影响数量，不影响 `order` 里已有
 * 模块的顺序。 */
function loadExtraSlots(): number {
  const raw = Number(localStorage.getItem(EXTRA_SLOTS_KEY));
  return Number.isFinite(raw) ? Math.min(MAX_EXTRA_SLOTS, Math.max(MIN_EXTRA_SLOTS, raw)) : MIN_EXTRA_SLOTS;
}

/** 拖拽过程中"预览一下松手会变成什么样"的显示顺序——纯计算，不改真实的
 * `order` 状态，真正提交发生在松手那一刻（见 `ModuleCard` 文档）。 */
function previewOrder(order: string[], dragId: string | null, overId: string | null): string[] {
  if (!dragId || !overId || dragId === overId) return order;
  const next = order.filter((id) => id !== dragId);
  if (overId === EMPTY_SLOT_ID) {
    next.push(dragId);
    return next;
  }
  const index = next.indexOf(overId);
  if (index === -1) return order;
  next.splice(index, 0, dragId);
  return next;
}

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
  // 2026-09 需求：不同模块各自维护自己"添加过"的工作区子集，不再共用同一份
  // "最近工作区"（见 WorkspacePicker.tsx 文档）——首页两张卡片各自单独拉一份。
  const [workspaceModuleWorkspaces, setWorkspaceModuleWorkspaces] = useState<WorkspaceProfile[]>([]);
  const [httpModuleWorkspaces, setHttpModuleWorkspaces] = useState<WorkspaceProfile[]>([]);
  const [connections, setConnections] = useState<ConnectionProfile[]>([]);
  const [dataSources, setDataSources] = useState<DataSourceProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const [order, setOrder] = useState<string[]>(loadModuleOrder);
  const [extraSlots, setExtraSlots] = useState<number>(loadExtraSlots);
  const [dragId, setDragId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async (showSpinner: boolean) => {
      if (showSpinner) setLoading(true);
      try {
        // 工作区卡片本身已有固定高度的滚动列表；必须取足够多的记录，不能只取
        // 前 6 条，否则工作区多时后面的记录根本没有机会通过滚动看到。
        const [wWorkspace, wHttp, c, ds] = await Promise.all([
          workspaceService.listForModule("workspace"),
          workspaceService.listForModule("http"),
          connectionService.list(),
          sqlService.listDataSources(),
        ]);
        if (!cancelled) {
          setWorkspaceModuleWorkspaces(wWorkspace);
          setHttpModuleWorkspaces(wHttp);
          setConnections(c.slice(0, 6));
          setDataSources(ds.slice(0, 6));
        }
      } catch (e) {
        if (!cancelled) push("error", `加载首页列表失败：${formatError(e)}`);
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    void load(true);
    // 首页窗口是独立进程，常驻后台不关闭——在 SSH/工作区/SQL 桌面等模块窗口
    // （各自另起进程）里新增连接/数据源后，回到首页窗口不会自动收到通知，
    // 之前只在挂载时拉一次列表，导致"新增了看不到"（2026-09 用户实测反馈）。
    // 用原生窗口 focus 事件在每次首页重新获得焦点时补拉一次，不用轮询。
    const unlistenPromise = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) void load(false);
    });
    return () => {
      cancelled = true;
      unlistenPromise.then((unlisten) => unlisten());
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

  // 指针事件版拖拽（原生 HTML5 dnd 在这个应用里被 Tauri 的 dragDropEnabled
  // 吃掉了，见 ModuleCard 文档）。拖动过程中只更新 `overId`（纯预览，见
  // `previewOrder`），真正的 `order` 状态只在松手那一刻才提交+落盘——拖到
  // 无效区域（`elementFromPoint` 找不到任何 `data-module-id`）就相当于取消，
  // 不会误改顺序。
  const handleDragStart = (id: string, e: React.PointerEvent) => {
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    setDragId(id);
    setOverId(id);
  };

  const handleDragMove = (e: React.PointerEvent) => {
    if (!dragId) return;
    const hit = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-module-id]");
    setOverId(hit?.dataset.moduleId ?? null);
  };

  const handleDragEnd = (e: React.PointerEvent) => {
    if (dragId && overId) {
      const next = previewOrder(order, dragId, overId);
      setOrder(next);
      localStorage.setItem(MODULE_ORDER_KEY, JSON.stringify(next));
    }
    setDragId(null);
    setOverId(null);
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      // 指针已经因为其他原因释放（比如 pointercancel），忽略。
    }
  };

  const addExtraSlot = () => {
    setExtraSlots((cur) => {
      const next = Math.min(MAX_EXTRA_SLOTS, cur + 1);
      localStorage.setItem(EXTRA_SLOTS_KEY, String(next));
      return next;
    });
  };

  const removeExtraSlot = () => {
    setExtraSlots((cur) => {
      const next = Math.max(MIN_EXTRA_SLOTS, cur - 1);
      localStorage.setItem(EXTRA_SLOTS_KEY, String(next));
      return next;
    });
  };

  const displayOrder = previewOrder(order, dragId, overId);

  const dragProps = (id: string) => ({
    id,
    dragging: dragId === id,
    dragOver: dragId !== null && dragId !== id && overId === id,
    onDragStart: handleDragStart,
    onDragMove: handleDragMove,
    onDragEnd: handleDragEnd,
  });

  const cardsById: Record<string, React.ReactNode> = {
    ssh: (
      <ModuleCard
        key="ssh"
        {...dragProps("ssh")}
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
    ),
    workspace: (
      <ModuleCard
        key="workspace"
        {...dragProps("workspace")}
        icon={<FolderGit2 />}
        title="工作区"
        description="类似 VS Code：打开本地文件夹或远程目录，代码浏览、编辑、终端、AI 编程一体"
        onEnter={() => open("workspace")}
        addLabel="打开文件夹/连接"
        onAdd={() => open("workspace")}
      >
        {loading ? null : workspaceModuleWorkspaces.length === 0 ? (
          <div className="home-dashboard-card-empty">暂无已添加的工作区，点上方进入后打开或选择</div>
        ) : (
          workspaceModuleWorkspaces.map((w) => (
            <div key={w.id} className="home-dashboard-card-item" onClick={() => open("workspace", w.id)}>
              {w.kind === "local" ? <Laptop size={14} /> : <Server size={14} />}
              <span className="home-dashboard-item-name">{w.display_name}</span>
              <span className="home-dashboard-item-sub">{w.root_path}</span>
            </div>
          ))
        )}
      </ModuleCard>
    ),
    editor: (
      <ModuleCard
        key="editor"
        {...dragProps("editor")}
        icon={<PenSquare />}
        title="编辑器"
        description="类似 UltraEdit：不依赖工作区，随时打开单个文件快速编辑"
        onEnter={() => open("editor")}
        addLabel="打开编辑器"
        onAdd={() => open("editor")}
      >
        <div className="home-dashboard-card-empty">进入后用 Ctrl+O 或拖拽打开文件</div>
      </ModuleCard>
    ),
    sql: (
      <ModuleCard
        key="sql"
        {...dragProps("sql")}
        icon={<Database />}
        title="SQL 桌面"
        description="类似 DBeaver：管理多数据库连接，SQL 编辑、执行、AI 辅助一体"
        onEnter={() => open("sql")}
        addLabel="新建数据源"
        onAdd={() => open("sql")}
      >
        {loading ? null : dataSources.length === 0 ? (
          <div className="home-dashboard-card-empty">暂无已保存数据源，点上方进入后新建</div>
        ) : (
          dataSources.map((d) => (
            <div key={d.id} className="home-dashboard-card-item" onClick={() => open("sql", d.id)}>
              <Database size={14} />
              <span className="home-dashboard-item-name">{d.name}</span>
              <span className="home-dashboard-item-sub">{d.db_kind}@{d.host}</span>
            </div>
          ))
        )}
      </ModuleCard>
    ),
    http: (
      <ModuleCard
        key="http"
        {...dragProps("http")}
        icon={<Send />}
        title="HTTP 测试工作台"
        description="类似 Apifox/Postman：添加一个工作区（本地文件夹或远程目录），接口调试、环境变量、历史记录"
        onEnter={() => open("http")}
        addLabel="添加工作区"
        onAdd={() => open("http")}
      >
        {loading ? null : httpModuleWorkspaces.length === 0 ? (
          <div className="home-dashboard-card-empty">暂无已添加的工作区，点上方进入后新建或选择</div>
        ) : (
          httpModuleWorkspaces.map((w) => (
            <div key={w.id} className="home-dashboard-card-item" onClick={() => open("http", w.id)}>
              {w.kind === "local" ? <Laptop size={14} /> : <Server size={14} />}
              <span className="home-dashboard-item-name">{w.display_name}</span>
              <span className="home-dashboard-item-sub">{w.root_path}</span>
            </div>
          ))
        )}
      </ModuleCard>
    ),
    explorer: (
      <ModuleCard
        key="explorer"
        {...dragProps("explorer")}
        icon={<Monitor />}
        title="资源管理器"
        description="类似 Total Commander：本地双栏浏览，复制/移动/删除/重命名文件"
        onEnter={() => open("explorer")}
        addLabel="打开资源管理器"
        onAdd={() => open("explorer")}
      >
        <div className="home-dashboard-card-empty">左右两栏各自浏览本地目录，可互相复制/移动文件</div>
      </ModuleCard>
    ),
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
      <div className="home-dashboard-version">版本 v2.1.0&nbsp; · &nbsp;更新日期 2026-09-17</div>

      <div className="home-dashboard-grid">
        {displayOrder.map((id) => cardsById[id]).filter(Boolean)}
        {/* 常驻空白格（2026-09 用户反馈"格子数量也可以自己加"）——不只是拖拽时
            才出现，平时也一直显示成虚线"背板"，拖模块过去代表"挪到最后"，
            悬停时高亮。数量由用户用下面的 +/− 自己调，默认 0（不多占地方），
            凑够 9 格就是一个名副其实的九宫格。 */}
        {Array.from({ length: extraSlots }).map((_, i) => (
          <div
            key={`empty-${i}`}
            className={`home-dashboard-empty-slot ${dragId && overId === EMPTY_SLOT_ID ? "drag-over" : ""}`}
            data-module-id={EMPTY_SLOT_ID}
          >
            {dragId ? "放到这里排到最后" : "空白格"}
          </div>
        ))}
      </div>
      <div className="home-dashboard-grid-controls">
        <button className="btn ghost sm" onClick={removeExtraSlot} disabled={extraSlots <= MIN_EXTRA_SLOTS} title="减少一个空白格">
          <Minus size={13} />
        </button>
        <span>空白格 {extraSlots}</span>
        <button className="btn ghost sm" onClick={addExtraSlot} disabled={extraSlots >= MAX_EXTRA_SLOTS} title="增加一个空白格，可以把模块拖得更松散">
          <Plus size={13} />
        </button>
      </div>
    </div>
  );
};
