import { invoke } from "@tauri-apps/api/core";
import type { McpServer, McpServerInput } from "../types/bindings";

/** IPC 边界：MCP 服务器管理（本地 stdio / 远程 HTTP，REQUIREMENTS.md §3.7）。 */
export const mcpServerService = {
  list(): Promise<McpServer[]> {
    return invoke("mcp_server_list");
  },
  create(input: McpServerInput): Promise<McpServer> {
    return invoke("mcp_server_create", { input });
  },
  update(id: string, input: McpServerInput): Promise<McpServer> {
    return invoke("mcp_server_update", { id, input });
  },
  delete(id: string): Promise<void> {
    return invoke("mcp_server_delete", { id });
  },
};
