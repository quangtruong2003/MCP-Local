import type { TunnelDisplay } from "../api";

interface Props {
  port?: number;
  tunnel: TunnelDisplay;
  /** Configured tunnel id (display only — last 8 chars shown). */
  tunnelId?: string | null;
  busy?: boolean;
  onOpenSettings: () => void;
  onDisconnect: () => void;
}

const TUNNEL_LABEL: Record<TunnelDisplay["state"], string> = {
  stopped: "Stopped",
  connecting: "Connecting…",
  connected: "Connected",
  error: "Error",
};

export default function StatusBar({ port, tunnel, tunnelId, busy, onOpenSettings, onDisconnect }: Props) {
  const suffix = tunnelId ? tunnelId.slice(-8) : undefined;
  const color =
    tunnel.state === "connected"
      ? "var(--ok)"
      : tunnel.state === "error"
        ? "var(--danger)"
        : "var(--text-muted)";
  const title =
    tunnel.state === "error" && tunnel.detail
      ? `Tunnel error: ${tunnel.detail}`
      : undefined;
  return (
    <div
      style={{
        display: "flex",
        gap: 16,
        alignItems: "center",
        padding: "10px 14px",
        background: "var(--surface)",
        border: "1px solid var(--border)",
        borderRadius: 8,
      }}
    >
      <span>
        Local server:{" "}
        <code style={{ background: "var(--bg)", padding: "2px 6px", borderRadius: 4 }}>
          127.0.0.1:{port ?? "…"}/mcp
        </code>
      </span>
      <span style={{ color, fontWeight: 600 }} title={title}>
        Tunnel: {TUNNEL_LABEL[tunnel.state]}
        {suffix && tunnel.state === "connected" ? ` · ${suffix}` : ""}
      </span>
      <button onClick={onDisconnect} disabled={busy || tunnel.state === "stopped"}>
        Disconnect
      </button>
      <button onClick={onOpenSettings} style={{ marginLeft: "auto" }}>
        Tunnel settings…
      </button>
    </div>
  );
}
