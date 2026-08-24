import { invoke } from "@tauri-apps/api/core";

export interface Workspace {
  id: string;
  name: string;
  root_path: string;
  can_read: boolean;
  can_write: boolean;
  created_at: string;
  updated_at: string;
}

export interface AuditEvent {
  id: number;
  ts: string;
  source: string;
  tool?: string | null;
  workspace_id?: string | null;
  op: string;
  decision: string;
  error_code?: string | null;
  path?: string | null;
  detail?: unknown;
}

export interface ServerStatus {
  port: number;
  paused: boolean;
}

export type ConnectionState =
  | "stopped"
  | "starting_server"
  | "validating_tunnel"
  | "provisioning_tunnel"
  | "connecting"
  | "checking_health"
  | "ready"
  | "stopping"
  | "needs_setup"
  | "error";

export interface ConnectionSnapshot {
  state: ConnectionState;
  tunnel_id?: string | null;
  local_mcp_url: string;
  health_url?: string | null;
  started_at?: string | null;
  ready_at?: string | null;
  message?: string | null;
  error_code?: string | null;
  error_detail?: string | null;
}

export interface ConnectionSettingsSummary {
  tunnel_id: string;
  organization_id?: string | null;
  workspace_id?: string | null;
  tunnel_name: string;
  auto_start: boolean;
  has_runtime_key: boolean;
  has_admin_key: boolean;
}

export interface SaveConnectionSettingsInput {
  organizationId?: string | null;
  workspaceId?: string | null;
  tunnelName: string;
  autoStart: boolean;
  runtimeApiKey?: string | null;
  adminApiKey?: string | null;
}

export function parseErrorMessage(err: unknown): string {
  if (typeof err === "string") {
    try {
      const parsed = JSON.parse(err) as { code?: unknown; message?: unknown };
      if (parsed && typeof parsed.message === "string") return parsed.message;
    } catch {
      // plain string error
    }
    return err;
  }
  if (err instanceof Error) return err.message;
  return String(err);
}

export const api = {
  addWorkspace: (rootPath: string, name?: string) =>
    invoke<Workspace>("add_workspace", { rootPath, name: name ?? null }),
  listWorkspaces: () => invoke<Workspace[]>("list_workspaces"),
  setCapabilities: (id: string, canRead: boolean, canWrite: boolean) =>
    invoke<void>("set_capabilities", { id, canRead, canWrite }),
  removeWorkspace: (id: string) => invoke<void>("remove_workspace", { id }),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  recentAudit: () => invoke<AuditEvent[]>("recent_audit"),
  serverStatus: () => invoke<ServerStatus>("server_status"),
  connectionStatus: () => invoke<ConnectionSnapshot>("connection_status"),
  startConnection: () => invoke<ConnectionSnapshot>("start_connection"),
  stopConnection: () => invoke<ConnectionSnapshot>("stop_connection"),
  regenerateTunnel: () => invoke<ConnectionSnapshot>("regenerate_tunnel"),
  connectionSettingsSummary: () =>
    invoke<ConnectionSettingsSummary>("connection_settings_summary"),
  saveConnectionSettings: (input: SaveConnectionSettingsInput) =>
    invoke<ConnectionSettingsSummary>("save_connection_settings", { input }),
};
