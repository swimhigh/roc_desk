import { create } from "zustand";
import { sqlService } from "../services/sqlService";
import type {
  DataSourceInput,
  DataSourceProfile,
  ExecuteResult,
  FileChange,
  ObjectRef,
  PendingWrite,
  QueryHistoryEntry,
  WorkspaceTab,
} from "../types/bindings";
import { formatError } from "../utils/error";

export interface TabRunState {
  running: boolean;
  queryId: string | null;
  result: ExecuteResult | null;
  error: string | null;
  needsConfirmation: boolean;
  pendingWrite: PendingWrite | null;
  /** DDL 类语句被 `NeedsConfirmation` 挡下后，用户点确认时要重新提交同一段
   * SQL（带上 `confirmed=true`），这里记一份供 `confirmPendingDdl` 使用。 */
  lastSql: string;
}

/** `FileChange.path`/`FileSyncInfo.path` 是磁盘绝对路径（Windows 下用反斜杠），
 * `WorkspaceTab.file_path` 是存库时统一用正斜杠拼的相对路径
 * （`queries/<tab_id>.sql`，见 `sql::workspace_cache`）——要匹配两者，把绝对
 * 路径的分隔符也归一化成正斜杠再比较，不能反过来处理相对路径那一侧。 */
export function pathMatchesTab(absPath: string, tabFilePath: string): boolean {
  return absPath.replace(/\\/g, "/").endsWith(tabFilePath);
}

const EMPTY_RUN_STATE: TabRunState = {
  running: false,
  queryId: null,
  result: null,
  error: null,
  needsConfirmation: false,
  pendingWrite: null,
  lastSql: "",
};

const saveTimers = new Map<string, ReturnType<typeof setTimeout>>();
const saveQueues = new Map<string, Promise<void>>();
const cursorSaveTimers = new Map<string, ReturnType<typeof setTimeout>>();

interface SqlState {
  connectionStatus: "idle" | "connecting" | "connected" | "failed";
  saveStatus: Record<string, "saving" | "saved" | "error">;
  closedTabs: WorkspaceTab[];
  flushSaves: () => Promise<boolean>;
  leaveDataSource: () => Promise<boolean>;
  reopenTab: () => Promise<void>;
  renameTab: (id: string, title: string) => Promise<void>;
  dataSources: DataSourceProfile[];
  currentDataSourceId: string | null;
  databases: string[];
  currentDatabase: string | null;
  objects: ObjectRef[];
  objectsLoading: boolean;
  tabs: WorkspaceTab[];
  activeTabId: string | null;
  /** 双击表/生成模板刚插入一段 SQL 后，要求编辑器把这个标签页的全文选中——
   * 这样用户不用再手动框选就能直接点"运行语句/选区"（2026-09 用户反馈）。
   * 只存 tabId，`SqlEditorTabs.tsx` 消费后立即清空，不是持续生效的状态。 */
  selectAllRequestTabId: string | null;
  requestSelectAll: (tabId: string) => void;
  clearSelectAllRequest: () => void;
  tabContents: Record<string, string>;
  tabRuns: Record<string, TabRunState>;
  changesById: Record<string, FileChange>;
  history: QueryHistoryEntry[];
  error: string | null;

  loadDataSources: () => Promise<void>;
  selectDataSource: (id: string) => Promise<void>;
  saveDataSource: (id: string | null, input: DataSourceInput) => Promise<DataSourceProfile>;
  deleteDataSource: (id: string) => Promise<void>;

  refreshObjects: () => Promise<void>;
  loadDatabases: () => Promise<void>;
  switchDatabase: (name: string) => Promise<void>;

  createTab: (title: string) => Promise<void>;
  selectTab: (tabId: string) => Promise<void>;
  deleteTab: (tabId: string) => Promise<void>;
  setTabContent: (tabId: string, content: string) => void;
  saveTabContent: (tabId: string) => Promise<void>;
  setResultViewMode: (tabId: string, mode: "table" | "text") => Promise<void>;
  /** 光标位置持久化（2026-09 用户反馈"下次进来要保持原样"）——存进
   * `sql_workspace_tabs.cursor_json`，重新打开这个数据源/标签页时用它还原
   * 光标，不只是同一次会话内切标签页保留（那个是 Monaco `saveViewState`
   * 管的，跨进程/跨次启动就没了）。 */
  updateTabCursor: (tabId: string, cursorJson: string) => void;

  runSql: (tabId: string, sql: string, confirmed: boolean) => Promise<void>;
  pollUntilDone: (tabId: string, queryId: string) => Promise<void>;
  cancelQuery: (tabId: string) => Promise<void>;
  confirmWrite: (tabId: string) => Promise<void>;
  rollbackWrite: (tabId: string) => Promise<void>;

  loadHistory: () => Promise<void>;

  applyFileChange: (change: FileChange) => void;
  acceptChange: (changeId: string) => Promise<void>;
  rejectChange: (changeId: string) => Promise<void>;
  undoChange: (changeId: string) => Promise<void>;
}

export const useSqlStore = create<SqlState>((set, get) => ({
  connectionStatus: "idle",
  saveStatus: {},
  closedTabs: [],
  flushSaves: async () => {
    await Promise.all(Object.keys(get().tabContents).map((id) => get().saveTabContent(id)));
    return !Object.values(get().saveStatus).includes("error");
  },
  leaveDataSource: async () => {
    if (Object.values(get().tabRuns).some((r) => r.running || r.pendingWrite || r.needsConfirmation)) {
      set({ error: "请先停止查询，并处理所有待确认操作，再离开或切换连接。" });
      return false;
    }
    if (!await get().flushSaves()) return false;
    const id = get().currentDataSourceId;
    try {
      if (id) await sqlService.closeSession(id);
      set({ currentDataSourceId: null, connectionStatus: "idle", error: null });
      return true;
    } catch (e) { set({ error: formatError(e) }); return false; }
  },
  reopenTab: async () => {
    const closed = get().closedTabs;
    const tab = closed[closed.length - 1];
    if (!tab) return;
    set((s) => ({ tabs: [...s.tabs, tab], closedTabs: s.closedTabs.slice(0, -1) }));
    await get().selectTab(tab.id);
  },
  renameTab: async (id, title) => {
    const tab = get().tabs.find((t) => t.id === id);
    if (!tab || !title.trim()) return;
    try {
      await sqlService.updateTabMeta(id, title.trim(), tab.result_view_mode, tab.cursor_json, tab.sort_order);
      set((s) => ({ tabs: s.tabs.map((t) => t.id === id ? { ...t, title: title.trim() } : t) }));
    } catch (e) { set({ error: formatError(e) }); }
  },
  dataSources: [],
  currentDataSourceId: null,
  databases: [],
  currentDatabase: null,
  objects: [],
  objectsLoading: false,
  tabs: [],
  activeTabId: null,
  selectAllRequestTabId: null,
  requestSelectAll: (tabId) => set({ selectAllRequestTabId: tabId }),
  clearSelectAllRequest: () => set({ selectAllRequestTabId: null }),
  tabContents: {},
  tabRuns: {},
  changesById: {},
  history: [],
  error: null,

  loadDataSources: async () => {
    try {
      const list = await sqlService.listDataSources();
      set({ dataSources: list });
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  selectDataSource: async (id) => {
    if (get().connectionStatus === "connecting") return;
    if (get().currentDataSourceId && !await get().leaveDataSource()) return;
    set({
      connectionStatus: "connecting", error: null, saveStatus: {}, closedTabs: [],
      currentDataSourceId: id,
      databases: [],
      currentDatabase: null,
      objects: [],
      tabs: [],
      activeTabId: null,
      tabContents: {},
      tabRuns: {},
      history: [],
    });
    try {
      await sqlService.openSession(id);
      const [tabs] = await Promise.all([
        sqlService.listTabs(id),
        get().refreshObjects(),
        get().loadDatabases(),
      ]);
      set({ tabs, connectionStatus: "connected" });
      if (tabs.length > 0) {
        await get().selectTab(tabs[0].id);
      }
      await get().loadHistory();
    } catch (e) {
      set({ error: formatError(e), connectionStatus: "failed" });
    }
  },

  loadDatabases: async () => {
    const dsId = get().currentDataSourceId;
    if (!dsId) return;
    try {
      const [databases, currentDatabase] = await Promise.all([
        sqlService.listDatabases(dsId),
        sqlService.currentDatabase(dsId),
      ]);
      set({ databases, currentDatabase });
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  switchDatabase: async (name) => {
    const dsId = get().currentDataSourceId;
    if (!dsId || name === get().currentDatabase) return;
    if (Object.values(get().tabRuns).some((r) => r.running || r.pendingWrite || r.needsConfirmation)) {
      set({ error: "请先结束查询和待确认事务，再切换数据库。" });
      return;
    }
    try {
      await sqlService.switchDatabase(dsId, name);
      set({ currentDatabase: name, objects: [] });
      await get().refreshObjects();
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  saveDataSource: async (id, input) => {
    const saved = await sqlService.saveDataSource(id, input);
    await get().loadDataSources();
    return saved;
  },

  deleteDataSource: async (id) => {
    await sqlService.deleteDataSource(id);
    if (get().currentDataSourceId === id) {
      set({ currentDataSourceId: null, tabs: [], activeTabId: null, objects: [] });
    }
    await get().loadDataSources();
  },

  refreshObjects: async () => {
    const dsId = get().currentDataSourceId;
    if (!dsId) return;
    set({ objectsLoading: true });
    try {
      const page = await sqlService.listObjects(dsId);
      set({ objects: page.objects, objectsLoading: false });
    } catch (e) {
      set({ error: formatError(e), objectsLoading: false });
    }
  },

  createTab: async (title) => {
    const dsId = get().currentDataSourceId;
    if (!dsId) return;
    const tab = await sqlService.createTab(dsId, title);
    set((s) => ({ tabs: [...s.tabs, tab], tabContents: { ...s.tabContents, [tab.id]: "" } }));
    await get().selectTab(tab.id);
  },

  selectTab: async (tabId) => {
    const dsId = get().currentDataSourceId;
    if (!dsId) return;
    // 内容要在切 `activeTabId` 之前就拿到手——Monaco 的 `<Editor value=...>`
    // 是受控组件，先切过去再异步把 value 从 "" 补成真实内容，等于对刚创建的
    // 空白 model 做了一次 `setValue` 替换，会把光标/滚动位置重置到开头
    // （2026-09 用户反馈"点 TAB 切换切回来还是在当前行"没生效，根因就是这个
    // 时序：先切 tab 后拿内容）。先加载完内容再一次性切换，`<Editor>` 对这个
    // path 的第一次渲染就是最终内容，不会有"空内容 -> 真实内容"这次多余替换，
    // `saveViewState`+`keepCurrentModel` 才能正常保留视图状态。
    if (get().tabContents[tabId] === undefined) {
      try {
        const content = await sqlService.readTabContent(dsId, tabId);
        set((s) => ({ tabContents: { ...s.tabContents, [tabId]: content } }));
      } catch (e) {
        set({ error: formatError(e) });
        return;
      }
    }
    set({ activeTabId: tabId });
  },

  updateTabCursor: (tabId, cursorJson) => {
    const tab = get().tabs.find((t) => t.id === tabId);
    if (!tab) return;
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, cursor_json: cursorJson } : t)) }));
    clearTimeout(cursorSaveTimers.get(tabId));
    cursorSaveTimers.set(
      tabId,
      setTimeout(() => {
        sqlService
          .updateTabMeta(tabId, tab.title, tab.result_view_mode, cursorJson, tab.sort_order)
          .catch((e) => set({ error: formatError(e) }));
      }, 400),
    );
  },

  deleteTab: async (tabId) => {
    const dsId = get().currentDataSourceId;
    if (!dsId) return;
    const run = get().tabRuns[tabId];
    if (run?.running || run?.pendingWrite || run?.needsConfirmation) {
      set({ error: "请先停止查询或处理待确认操作，再关闭标签。" }); return;
    }
    await get().saveTabContent(tabId);
    if (get().saveStatus[tabId] === "error") return;
    set((s) => {
      const closed = s.tabs.find((t) => t.id === tabId);
      const tabs = s.tabs.filter((t) => t.id !== tabId);
      const activeTabId = s.activeTabId === tabId ? (tabs[0]?.id ?? null) : s.activeTabId;
      const { [tabId]: _removedContent, ...tabContents } = s.tabContents;
      const { [tabId]: _removedRun, ...tabRuns } = s.tabRuns;
      return { tabs, activeTabId, tabContents, tabRuns, closedTabs: closed ? [...s.closedTabs, closed] : s.closedTabs };
    });
    if (get().activeTabId) await get().selectTab(get().activeTabId!);
  },

  setTabContent: (tabId, content) => {
    set((s) => ({ tabContents: { ...s.tabContents, [tabId]: content }, saveStatus: { ...s.saveStatus, [tabId]: "saving" } }));
    clearTimeout(saveTimers.get(tabId));
    saveTimers.set(tabId, setTimeout(() => void get().saveTabContent(tabId), 500));
  },

  saveTabContent: async (tabId) => {
    const dsId = get().currentDataSourceId;
    if (!dsId || get().tabContents[tabId] === undefined) return;
    clearTimeout(saveTimers.get(tabId));
    saveTimers.delete(tabId);
    const content = get().tabContents[tabId];
    const task = (saveQueues.get(tabId) ?? Promise.resolve()).catch(() => {}).then(() => sqlService.writeTabContent(dsId, tabId, content));
    saveQueues.set(tabId, task);
    try {
      await task;
      if (get().tabContents[tabId] === content) set((s) => ({ saveStatus: { ...s.saveStatus, [tabId]: "saved" } }));
    } catch (e) {
      set((s) => ({ error: `草稿保存失败：${formatError(e)}`, saveStatus: { ...s.saveStatus, [tabId]: "error" } }));
    } finally {
      if (saveQueues.get(tabId) === task) saveQueues.delete(tabId);
    }
  },

  setResultViewMode: async (tabId, mode) => {
    const tab = get().tabs.find((t) => t.id === tabId);
    if (!tab) return;
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, result_view_mode: mode } : t)),
    }));
    try {
      await sqlService.updateTabMeta(tabId, tab.title, mode, tab.cursor_json, tab.sort_order);
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  runSql: async (tabId, sql, confirmed) => {
    const dsId = get().currentDataSourceId;
    if (!dsId || !sql.trim()) return;
    const previous = get().tabRuns[tabId];
    if (get().connectionStatus !== "connected" || previous?.running || previous?.pendingWrite || (previous?.needsConfirmation && !confirmed)) return;
    set((s) => ({ tabRuns: { ...s.tabRuns, [tabId]: { ...EMPTY_RUN_STATE, running: true, lastSql: sql } } }));
    try {
      const outcome = await sqlService.execute(dsId, sql, confirmed);
      if (outcome.kind === "Started") {
        set((s) => ({
          tabRuns: { ...s.tabRuns, [tabId]: { ...EMPTY_RUN_STATE, running: true, queryId: outcome.query_id, lastSql: sql } },
        }));
        await get().pollUntilDone(tabId, outcome.query_id);
      } else if (outcome.kind === "NeedsConfirmation") {
        set((s) => ({
          tabRuns: { ...s.tabRuns, [tabId]: { ...EMPTY_RUN_STATE, needsConfirmation: true, lastSql: sql } },
        }));
      } else {
        set((s) => ({
          tabRuns: { ...s.tabRuns, [tabId]: { ...EMPTY_RUN_STATE, pendingWrite: outcome, lastSql: sql } },
        }));
      }
      await get().loadHistory();
    } catch (e) {
      set((s) => ({ tabRuns: { ...s.tabRuns, [tabId]: { ...EMPTY_RUN_STATE, error: formatError(e), lastSql: sql } } }));
    }
  },

  pollUntilDone: async (tabId, queryId) => {
    // 简单轮询而不是事件推流（方案原设计是 `sql://query/{id}/chunk` 流式事件，
    // 这一版为了控制实现规模改成轮询：结果本来就按 1000 行截断，一次性拿回
    // 不会有体积问题，见 commands/sql.rs 里的说明）。
    for (;;) {
      const poll = await sqlService.pollQuery(queryId);
      if (poll.status === "running") {
        await new Promise((r) => setTimeout(r, 300));
        continue;
      }
      if (poll.status === "finished") {
        set((s) => ({
          tabRuns: { ...s.tabRuns, [tabId]: { ...EMPTY_RUN_STATE, result: poll.result } },
        }));
      } else if (poll.status === "cancelled") {
        set((s) => ({
          tabRuns: { ...s.tabRuns, [tabId]: { ...EMPTY_RUN_STATE, error: "查询已取消" } },
        }));
      } else {
        set((s) => ({
          tabRuns: { ...s.tabRuns, [tabId]: { ...EMPTY_RUN_STATE, error: poll.error ?? "执行失败" } },
        }));
      }
      return;
    }
  },

  cancelQuery: async (tabId) => {
    const dsId = get().currentDataSourceId;
    const run = get().tabRuns[tabId];
    if (!dsId || !run?.queryId) return;
    try {
      await sqlService.cancel(dsId, run.queryId);
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  confirmWrite: async (tabId) => {
    const dsId = get().currentDataSourceId;
    const run = get().tabRuns[tabId];
    if (!dsId || !run?.pendingWrite || run.running) return;
    set((s) => ({ tabRuns: { ...s.tabRuns, [tabId]: { ...run, running: true, error: null } } }));
    try {
      const result = await sqlService.confirmWrite(dsId, run.pendingWrite.pending_id);
      set((s) => ({ tabRuns: { ...s.tabRuns, [tabId]: { ...EMPTY_RUN_STATE, result } } }));
    } catch (e) {
      set((s) => ({ tabRuns: { ...s.tabRuns, [tabId]: { ...run, running: false, error: `提交结果未确认，请核实事务状态：${formatError(e)}` } } }));
    }
  },

  rollbackWrite: async (tabId) => {
    const dsId = get().currentDataSourceId;
    const run = get().tabRuns[tabId];
    if (!dsId || !run?.pendingWrite || run.running) return;
    set((s) => ({ tabRuns: { ...s.tabRuns, [tabId]: { ...run, running: true, error: null } } }));
    try {
      await sqlService.rollbackWrite(dsId, run.pendingWrite.pending_id);
      set((s) => ({ tabRuns: { ...s.tabRuns, [tabId]: EMPTY_RUN_STATE } }));
    } catch (e) {
      set((s) => ({ tabRuns: { ...s.tabRuns, [tabId]: { ...run, running: false, error: `回滚结果未确认，请重试或核实事务状态：${formatError(e)}` } } }));
    }
  },

  loadHistory: async () => {
    const dsId = get().currentDataSourceId;
    if (!dsId) return;
    try {
      const history = await sqlService.queryHistory(dsId, 50);
      set({ history });
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  applyFileChange: (change) => {
    set((s) => ({ changesById: { ...s.changesById, [change.id]: change } }));
  },

  acceptChange: async (changeId) => {
    const dsId = get().currentDataSourceId;
    if (!dsId) return;
    const sync = await sqlService.acceptChange(dsId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "applied" } },
    }));
    // 磁盘内容变了，刷新对应标签页的编辑器缓存（找到 file_path 匹配的 tab）。
    const tab = get().tabs.find((t) => pathMatchesTab(sync.path, t.file_path));
    if (tab) {
      set((s) => ({ tabContents: { ...s.tabContents, [tab.id]: sync.content } }));
    }
  },

  rejectChange: async (changeId) => {
    const dsId = get().currentDataSourceId;
    if (!dsId) return;
    await sqlService.rejectChange(dsId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "rejected" } },
    }));
  },

  undoChange: async (changeId) => {
    const dsId = get().currentDataSourceId;
    if (!dsId) return;
    const sync = await sqlService.undoChange(dsId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "undone" } },
    }));
    const tab = get().tabs.find((t) => pathMatchesTab(sync.path, t.file_path));
    if (tab) {
      set((s) => ({ tabContents: { ...s.tabContents, [tab.id]: sync.content } }));
    }
  },
}));
