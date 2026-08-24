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
