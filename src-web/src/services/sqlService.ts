import { invoke } from "@tauri-apps/api/core";
import type {
  AlterOp,
  DataSourceInput,
  DataSourceProfile,
  DbInfo,
  ExecuteOutcome,
  ExecuteResult,
  FileChange,
  FileSyncInfo,
  NamedCell,
  ObjectDefinition,
  ObjectPage,
  ObjectRef,
  QueryHistoryEntry,
  QueryPoll,
  TransferFormat,
  TransferProgress,
  WorkspaceTab,
} from "../types/bindings";

/** IPC 边界（docs/SQL_DESKTOP_PLAN.md）：SQL 桌面模块的数据源/会话/执行/AI。 */
export const sqlService = {
  previewTemplate(dataSourceId: string, object: ObjectRef): Promise<string> {
    return invoke("sql_preview_template", { dataSourceId, object });
  },
  listDataSources(): Promise<DataSourceProfile[]> {
    return invoke("sql_list_data_sources");
  },
  saveDataSource(id: string | null, input: DataSourceInput): Promise<DataSourceProfile> {
    return invoke("sql_save_data_source", { id, input });
  },
  deleteDataSource(id: string): Promise<void> {
    return invoke("sql_delete_data_source", { id });
  },
  testConnection(id: string | null, input: DataSourceInput): Promise<DbInfo> {
    return invoke("sql_test_connection", { id, input });
  },

  openSession(dataSourceId: string): Promise<void> {
    return invoke("sql_open_session", { dataSourceId });
  },
  closeSession(dataSourceId: string): Promise<void> {
    return invoke("sql_close_session", { dataSourceId });
  },
  listDatabases(dataSourceId: string): Promise<string[]> {
    return invoke("sql_list_databases", { dataSourceId });
  },
  currentDatabase(dataSourceId: string): Promise<string | null> {
    return invoke("sql_current_database", { dataSourceId });
  },
  switchDatabase(dataSourceId: string, databaseName: string): Promise<void> {
    return invoke("sql_switch_database", { dataSourceId, databaseName });
  },
  listObjects(dataSourceId: string): Promise<ObjectPage> {
    return invoke("sql_list_objects", { dataSourceId });
  },
  describeObject(dataSourceId: string, object: ObjectRef): Promise<ObjectDefinition> {
    return invoke("sql_describe_object", { dataSourceId, object });
  },

  execute(dataSourceId: string, sql: string, confirmed: boolean): Promise<ExecuteOutcome> {
    return invoke("sql_execute", { dataSourceId, sql, confirmed });
  },
  pollQuery(queryId: string): Promise<QueryPoll> {
    return invoke("sql_poll_query", { queryId });
  },
  cancel(dataSourceId: string, queryId: string): Promise<void> {
    return invoke("sql_cancel", { dataSourceId, queryId });
  },
  confirmWrite(dataSourceId: string, pendingId: string): Promise<ExecuteResult> {
    return invoke("sql_confirm_write", { dataSourceId, pendingId });
  },
  rollbackWrite(dataSourceId: string, pendingId: string): Promise<void> {
    return invoke("sql_rollback_write", { dataSourceId, pendingId });
  },
  explain(dataSourceId: string, sql: string): Promise<ExecuteResult> {
    return invoke("sql_explain", { dataSourceId, sql });
  },
  queryHistory(dataSourceId: string, limit: number): Promise<QueryHistoryEntry[]> {
    return invoke("sql_query_history", { dataSourceId, limit });
  },

  listTabs(dataSourceId: string): Promise<WorkspaceTab[]> {
    return invoke("sql_workspace_tabs_list", { dataSourceId });
  },
  createTab(dataSourceId: string, title: string): Promise<WorkspaceTab> {
    return invoke("sql_workspace_tab_create", { dataSourceId, title });
  },
  deleteTab(dataSourceId: string, tabId: string): Promise<void> {
    return invoke("sql_workspace_tab_delete", { dataSourceId, tabId });
  },
  updateTabMeta(
    tabId: string,
    title: string,
    resultViewMode: string,
    cursorJson: string | null,
    sortOrder: number,
  ): Promise<void> {
    return invoke("sql_workspace_tab_update_meta", { tabId, title, resultViewMode, cursorJson, sortOrder });
  },
  readTabContent(dataSourceId: string, tabId: string): Promise<string> {
    return invoke("sql_tab_read_content", { dataSourceId, tabId });
  },
  writeTabContent(dataSourceId: string, tabId: string, content: string): Promise<void> {
    return invoke("sql_tab_write_content", { dataSourceId, tabId, content });
  },

  aiGenerate(
    dataSourceId: string,
    tabId: string,
    providerId: string,
    instruction: string,
    schemaContext: string,
    currentSql: string,
  ): Promise<FileChange> {
    return invoke("sql_ai_generate", { dataSourceId, tabId, providerId, instruction, schemaContext, currentSql });
  },
  aiExplain(providerId: string, sql: string, schemaContext: string): Promise<string> {
    return invoke("sql_ai_explain", { providerId, sql, schemaContext });
  },
  aiOptimize(
    dataSourceId: string,
    tabId: string,
    providerId: string,
    sql: string,
    explainOutput: string | null,
    schemaContext: string,
  ): Promise<FileChange> {
    return invoke("sql_ai_optimize", { dataSourceId, tabId, providerId, sql, explainOutput, schemaContext });
  },
  aiFixError(
    dataSourceId: string,
    tabId: string,
    providerId: string,
    sql: string,
    errorMessage: string,
    schemaContext: string,
  ): Promise<FileChange> {
    return invoke("sql_ai_fix_error", { dataSourceId, tabId, providerId, sql, errorMessage, schemaContext });
  },

  acceptChange(dataSourceId: string, changeId: string): Promise<FileSyncInfo> {
    return invoke("sql_accept_change", { dataSourceId, changeId });
  },
  rejectChange(dataSourceId: string, changeId: string): Promise<void> {
    return invoke("sql_reject_change", { dataSourceId, changeId });
  },
  undoChange(dataSourceId: string, changeId: string): Promise<FileSyncInfo> {
    return invoke("sql_undo_change", { dataSourceId, changeId });
  },
  revertTurn(dataSourceId: string, turnId: string): Promise<FileSyncInfo[]> {
    return invoke("sql_revert_turn", { dataSourceId, turnId });
  },

  tablePage(dataSourceId: string, object: ObjectRef, limit: number, offset: number): Promise<ExecuteResult> {
    return invoke("sql_table_page", { dataSourceId, object, limit, offset });
  },
  tableRowCount(dataSourceId: string, object: ObjectRef): Promise<number> {
    return invoke("sql_table_row_count", { dataSourceId, object });
  },
  tableUpdateCell(
    dataSourceId: string,
    object: ObjectRef,
    pk: NamedCell[],
    column: string,
    value: { is_null: boolean; text: string },
  ): Promise<void> {
    return invoke("sql_table_update_cell", { dataSourceId, object, pk, column, value });
  },
  tableDeleteRow(dataSourceId: string, object: ObjectRef, pk: NamedCell[]): Promise<void> {
    return invoke("sql_table_delete_row", { dataSourceId, object, pk });
  },
  tableInsertRow(dataSourceId: string, object: ObjectRef, values: NamedCell[]): Promise<void> {
    return invoke("sql_table_insert_row", { dataSourceId, object, values });
  },
  generateAlterTable(dataSourceId: string, object: ObjectRef, ops: AlterOp[]): Promise<string[]> {
    return invoke("sql_generate_alter_table", { dataSourceId, object, ops });
  },

  exportStart(
    dataSourceId: string,
    object: ObjectRef,
    format: TransferFormat,
    filePath: string,
    resume: boolean,
  ): Promise<string> {
    return invoke("sql_export_start", { dataSourceId, object, format, filePath, resume });
  },
  exportPoll(exportId: string): Promise<TransferProgress> {
    return invoke("sql_export_poll", { exportId });
  },
  exportCancel(exportId: string): Promise<void> {
    return invoke("sql_export_cancel", { exportId });
  },
  importStart(
    dataSourceId: string,
    object: ObjectRef,
    filePath: string,
    hasHeader: boolean,
    resume: boolean,
  ): Promise<string> {
    return invoke("sql_import_start", { dataSourceId, object, filePath, hasHeader, resume });
  },
  importPoll(importId: string): Promise<TransferProgress> {
    return invoke("sql_import_poll", { importId });
  },
  importCancel(importId: string): Promise<void> {
    return invoke("sql_import_cancel", { importId });
  },
  writeTextFile(path: string, content: string): Promise<void> {
    return invoke("sql_write_text_file", { path, content });
  },
};
