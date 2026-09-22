import { invoke } from "@tauri-apps/api/core";
import type {
  EnvironmentDef,
  EnvVar,
  HttpCollectionMeta,
  HttpCollectionSummary,
  HttpExecuteResult,
  HttpRequestHistoryDetail,
  HttpRequestHistoryEntry,
  HttpWorkspaceTab,
  RequestDef,
  RequestSummary,
} from "../types/bindings";

/** IPC 边界（CODE_DESIGN.md §一分层原则）——HTTP 桌面所有 command 都要求目标
 * 工作区已经通过 `workspaceService.openLocal`/`openRemote` 打开过（见
 * docs/HTTP_DESKTOP_PLAN.md §3.1），这里不重复封装"打开工作区"。 */
export const httpDeskService = {
  listCollections(workspaceId: string): Promise<HttpCollectionSummary[]> {
    return invoke("http_list_collections", { workspaceId });
  },
  createCollection(workspaceId: string, name: string): Promise<HttpCollectionSummary> {
    return invoke("http_create_collection", { workspaceId, name });
  },
  renameCollection(workspaceId: string, slug: string, name: string): Promise<void> {
    return invoke("http_rename_collection", { workspaceId, slug, name });
  },
  deleteCollection(workspaceId: string, slug: string): Promise<void> {
    return invoke("http_delete_collection", { workspaceId, slug });
  },
  getCollectionMeta(workspaceId: string, slug: string): Promise<HttpCollectionMeta> {
    return invoke("http_get_collection_meta", { workspaceId, slug });
  },
  saveCollectionMeta(workspaceId: string, slug: string, meta: HttpCollectionMeta): Promise<void> {
    return invoke("http_save_collection_meta", { workspaceId, slug, meta });
  },

  listRequests(workspaceId: string, slug: string): Promise<RequestSummary[]> {
    return invoke("http_list_requests", { workspaceId, slug });
  },
  getRequest(workspaceId: string, slug: string, id: string): Promise<RequestDef> {
    return invoke("http_get_request", { workspaceId, slug, id });
  },
  createRequest(workspaceId: string, slug: string, name: string, folder: string[] = []): Promise<RequestDef> {
    return invoke("http_create_request", { workspaceId, slug, name, folder });
  },
  saveRequest(workspaceId: string, slug: string, request: RequestDef): Promise<void> {
    return invoke("http_save_request", { workspaceId, slug, request });
  },
  deleteRequest(workspaceId: string, slug: string, id: string): Promise<void> {
    return invoke("http_delete_request", { workspaceId, slug, id });
  },

  listEnvironments(workspaceId: string, slug: string): Promise<EnvironmentDef[]> {
    return invoke("http_list_environments", { workspaceId, slug });
  },
  saveEnvironment(workspaceId: string, slug: string, environment: EnvironmentDef): Promise<EnvironmentDef> {
    return invoke("http_save_environment", { workspaceId, slug, environment });
  },
  deleteEnvironment(workspaceId: string, slug: string, envId: string): Promise<void> {
    return invoke("http_delete_environment", { workspaceId, slug, envId });
  },
  getGlobalVariables(workspaceId: string): Promise<EnvVar[]> {
    return invoke("http_get_global_variables", { workspaceId });
  },
  saveGlobalVariables(workspaceId: string, variables: EnvVar[]): Promise<void> {
    return invoke("http_save_global_variables", { workspaceId, variables });
  },

  sendRequest(
    workspaceId: string,
    slug: string,
    request: RequestDef,
    environmentId: string | null,
  ): Promise<HttpExecuteResult> {
    return invoke("http_send_request", { workspaceId, slug, request, environmentId });
  },
  importCurl(command: string): Promise<RequestDef> {
    return invoke("http_import_curl", { command });
  },
  importPostman(workspaceId: string, content: string): Promise<HttpCollectionSummary> {
    return invoke("http_import_postman", { workspaceId, content });
  },
  importOpenApi(workspaceId: string, content: string): Promise<HttpCollectionSummary> {
    return invoke("http_import_openapi", { workspaceId, content });
  },
  exportPostman(workspaceId: string, slug: string): Promise<string> {
    return invoke("http_export_postman", { workspaceId, slug });
  },

  listHistory(workspaceId: string, limit = 200): Promise<HttpRequestHistoryEntry[]> {
    return invoke("http_list_history", { workspaceId, limit });
  },
  getHistoryDetail(id: string): Promise<HttpRequestHistoryDetail | null> {
    return invoke("http_get_history_detail", { id });
  },
  deleteHistory(id: string): Promise<void> {
    return invoke("http_delete_history", { id });
  },
  clearHistory(workspaceId: string): Promise<void> {
    return invoke("http_clear_history", { workspaceId });
  },

  listTabs(workspaceId: string): Promise<HttpWorkspaceTab[]> {
    return invoke("http_list_tabs", { workspaceId });
  },
  openTab(workspaceId: string, slug: string, requestId: string, title: string): Promise<HttpWorkspaceTab> {
    return invoke("http_open_tab", { workspaceId, slug, requestId, title });
  },
  closeTab(id: string): Promise<void> {
    return invoke("http_close_tab", { id });
  },
};
