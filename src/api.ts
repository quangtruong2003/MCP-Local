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

/** Normalized tunnel status for display. */
export interface TunnelDisplay {
  state: "stopped" | "connecting" | "connected" | "error";
  /** Present only when state is "error". */
  detail?: string;
}

/**
 * Narrow a tunnel-status payload to a display shape. The backend serializes
 * its `TunnelStatus` enum as lowercase strings ("stopped" | "connecting" |
 * "connected") or an object {"error": "<detail>"}.
 */
export function tunnelStateFromPayload(payload: unknown): TunnelDisplay {
  if (typeof payload === "string") {
    if (payload === "connecting") return { state: "connecting" };
    if (payload === "connected") return { state: "connected" };
    return { state: "stopped" };
  }
  if (payload && typeof payload === "object" && "error" in payload) {
    const raw = (payload as { error?: unknown }).error;
    return { state: "error", detail: typeof raw === "string" ? raw : String(raw) };
  }
  return { state: "stopped" };
}

/** Same as {@link tunnelStateFromPayload} for the raw JSON returned by `tunnel_status`. */
export function tunnelStateFromJson(raw: string): TunnelDisplay {
  try {
    return tunnelStateFromPayload(JSON.parse(raw));
  } catch {
    return tunnelStateFromPayload(raw);
  }
}

/**
 * Commands reject with a string containing CoreError JSON
 * ({code, message, detail}); extract a human-readable message.
 */
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
  configureTunnel: (tunnelId: string, apiKey: string) =>
    invoke<void>("configure_tunnel", { tunnelId, apiKey }),
  stopTunnel: () => invoke<void>("stop_tunnel"),
  tunnelStatus: () => invoke<string>("tunnel_status"),
};
