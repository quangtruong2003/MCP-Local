import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import type {
  AppError,
  AppSettings,
  BrowserRuntimeStatus,
  ConnectionStatus,
  DiagnosticsReport,
  ExecutionMode,
  Workspace,
} from "@/lib/types";

export type {
  AppError,
  AppSettings,
  BrowserRuntimeCapabilities,
  BrowserRuntimeProvisionState,
  BrowserRuntimeStatus,
  ConnectionStatus,
  ConnectionState,
  DiagnosticsReport,
  ExecutionMode,
  ThemePreference,
  ViewId,
  Workspace,
} from "@/lib/types";

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

/**
 * Parses a backend error (a JSON string shaped `{code?, message, detail?}`,
 * or any thrown value) into a structured {@link AppError} with stable codes.
 */
export function parseError(err: unknown): AppError {
  if (typeof err === "string") {
    try {
      const parsed = JSON.parse(err) as Partial<AppError>;
      if (parsed && typeof parsed === "object" && typeof parsed.message === "string") {
        return { code: parsed.code, message: parsed.message, detail: parsed.detail };
      }
    } catch {
      // plain string error
    }
    return { message: err };
  }
  if (err instanceof Error) return { message: err.message };
  return { message: String(err) };
}

/** Convenience for UI messages that only need the human-readable text. */
export function parseErrorMessage(err: unknown): string {
  return parseError(err).message;
}

export const api = {
  // Workspaces & audit
  addWorkspace: (rootPath: string, name?: string) =>
    invoke<Workspace>("add_workspace", { rootPath, name: name ?? null }),
  listDrives: () => invoke<string[]>("list_drives"),
  listWorkspaces: () => invoke<Workspace[]>("list_workspaces"),
  setCapabilities: (id: string, canRead: boolean, canWrite: boolean) =>
    invoke<void>("set_capabilities", { id, canRead, canWrite }),
  setExecutionAccess: (id: string, canExecute: boolean, executionMode: ExecutionMode) =>
    invoke<void>("set_execution_access", { id, canExecute, executionMode }),
  removeWorkspace: (id: string) => invoke<void>("remove_workspace", { id }),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  recentAudit: () => invoke<AuditEvent[]>("recent_audit"),
  serverStatus: () => invoke<ServerStatus>("server_status"),

  // Connection lifecycle
  connectionStatus: () => invoke<ConnectionStatus>("connection_status"),
  startConnection: () => invoke<ConnectionStatus>("start_connection"),
  stopConnection: () => invoke<ConnectionStatus>("stop_connection"),
  retryConnection: () => invoke<ConnectionStatus>("retry_connection"),
  rotateConnectionUrl: () => invoke<ConnectionStatus>("rotate_connection_url"),
  resetTailscaleIdentity: (removeStateDir: boolean, restart: boolean) =>
    invoke<ConnectionStatus>("reset_tailscale_identity", { removeStateDir, restart }),
  connectionDiagnostics: () => invoke<DiagnosticsReport>("connection_diagnostics"),
  tailscaleStateDir: () => invoke<string>("tailscale_state_dir"),

  // Settings
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  updateAppSettings: (settings: AppSettings) =>
    invoke<AppSettings>("update_app_settings", { settings }),

  // Browser automation diagnostics & maintenance (desktop-only; never exposed
  // as MCP tools). Status reads are side-effect free: opening Settings never
  // starts a download or provisioning.
  browserRuntimeStatus: () => invoke<BrowserRuntimeStatus>("browser_runtime_status"),
  setupBrowserRuntime: () => invoke<BrowserRuntimeStatus>("setup_browser_runtime"),
  repairBrowserRuntime: () => invoke<BrowserRuntimeStatus>("repair_browser_runtime"),
  resetBrowserData: () => invoke<BrowserRuntimeStatus>("reset_browser_data"),
  removeBrowserRuntime: () => invoke<BrowserRuntimeStatus>("remove_browser_runtime"),

  /**
   * Opens a URL in the system browser. The Tauri opener capability restricts
   * this to Tailscale domains only; anything else is rejected by the backend.
   */
  openExternalUrl: (url: string) => openUrl(url),
};
