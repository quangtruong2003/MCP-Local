export type ViewId = "dashboard" | "workspaces" | "activity" | "security" | "settings";

export type ToastTone = "info" | "success" | "error";

export interface Toast {
  id: number;
  tone: ToastTone;
  message: string;
}

export interface ConnectionUiState {
  ready: boolean;
  label: string;
}

// --- Backend contracts (mirror crates/core/src/models.rs serde output) ---

export type ExecutionMode = "approved_commands" | "full_powershell";

export interface Workspace {
  id: string;
  name: string;
  root_path: string;
  can_read: boolean;
  can_write: boolean;
  can_execute: boolean;
  execution_mode: ExecutionMode;
  created_at: string;
  updated_at: string;
}

/**
 * Mirrors src-tauri/src/connection/state.rs `ConnectionState`
 * (serde tag = "state", snake_case variants).
 */
export type ConnectionState =
  | { state: "stopped" }
  | { state: "starting_server" }
  | { state: "starting_tunnel" }
  | { state: "needs_login"; login_url: string }
  | { state: "needs_funnel_permission"; message: string }
  | { state: "verifying"; fqdn: string }
  | { state: "online"; fqdn: string; endpoint: string }
  | { state: "degraded"; fqdn?: string | null; message: string }
  | { state: "stopping" }
  | { state: "error"; code: string; message: string };

/** Snapshot pushed on the `connection-status` event and returned by commands. */
export interface ConnectionStatus {
  state: ConnectionState;
  local_port: number;
  paused: boolean;
}

/** Versioned app settings persisted by the backend (`version: 2`). */
export interface AppSettings {
  version: number;
  start_on_launch: boolean;
  tailscale_hostname: string;
  onboarding_complete: boolean;
}

/** Structured shape backend errors are serialized into ({code,message,detail}). */
export interface AppError {
  code?: string;
  message: string;
  detail?: unknown;
}

export const VIEW_META: Record<ViewId, { title: string; subtitle: string }> = {
  dashboard: {
    title: "Dashboard",
    subtitle: "Start, stop and inspect your local MCP connection.",
  },
  workspaces: {
    title: "Workspaces",
    subtitle: "Control exactly which local folders the assistant can reach.",
  },
  activity: {
    title: "Activity",
    subtitle: "Review recent tool calls and manager decisions.",
  },
  security: {
    title: "Security",
    subtitle: "Understand the safeguards protecting your filesystem.",
  },
  settings: {
    title: "Settings",
    subtitle: "Configure the tunnel, credentials and app behavior.",
  },
};
