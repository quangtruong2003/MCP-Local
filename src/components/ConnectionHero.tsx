import type { ConnectionSnapshot } from "../api";

interface Props {
  connection: ConnectionSnapshot;
  busy: boolean;
  onStart: () => void;
  onStop: () => void;
  onOpenSettings: () => void;
}

const STATE_LABEL: Record<ConnectionSnapshot["state"], string> = {
  stopped: "MCP Offline",
  starting_server: "Preparing MCP",
  validating_tunnel: "Validating tunnel",
  provisioning_tunnel: "Creating secure tunnel",
  connecting: "Connecting",
  checking_health: "Checking readiness",
  ready: "MCP Ready",
  stopping: "Stopping",
  needs_setup: "Setup required",
  error: "Connection issue",
};

const pendingStates = new Set<ConnectionSnapshot["state"]>([
  "starting_server",
  "validating_tunnel",
  "provisioning_tunnel",
  "connecting",
  "checking_health",
  "stopping",
]);

function copyText(text: string) {
  void navigator.clipboard.writeText(text);
}

export default function ConnectionHero({ connection, busy, onStart, onStop, onOpenSettings }: Props) {
  const ready = connection.state === "ready";
  const pending = pendingStates.has(connection.state);
  const needsAttention = connection.state === "needs_setup" || connection.state === "error";

  return (
    <section className={`connection-hero ${ready ? "is-ready" : needsAttention ? "is-warning" : ""}`}>
      <div className="connection-hero-main">
        <div className={`hero-icon ${ready ? "ready" : pending ? "working" : needsAttention ? "warning" : "offline"}`}>
          {ready ? "✓" : pending ? "↻" : needsAttention ? "!" : "▶"}
        </div>
        <div className="hero-copy">
          <div className="eyebrow">Secure MCP Tunnel</div>
          <h2>{STATE_LABEL[connection.state]}</h2>
          <p>{connection.message ?? "Start the local MCP connection when you are ready."}</p>
          {connection.error_code ? <span className="error-code">{connection.error_code}</span> : null}
        </div>
      </div>

      <div className="hero-actions">
        {ready || connection.state === "stopping" ? (
          <button className="btn btn-danger btn-lg" onClick={onStop} disabled={busy || pending}>
            {connection.state === "stopping" ? "Stopping…" : "Stop"}
          </button>
        ) : needsAttention ? (
          <>
            <button className="btn btn-primary btn-lg" onClick={onStart} disabled={busy || pending}>
              Retry
            </button>
            <button className="btn btn-secondary" onClick={onOpenSettings}>Open settings</button>
          </>
        ) : (
          <button className="btn btn-primary btn-lg" onClick={onStart} disabled={busy || pending}>
            {pending ? "Starting…" : "Start"}
          </button>
        )}
      </div>

      {(connection.tunnel_id || ready) && (
        <div className="connection-details">
          <div className="detail-row">
            <span>Tunnel ID</span>
            <div className="detail-value">
              <code>{connection.tunnel_id ?? "—"}</code>
              {connection.tunnel_id ? (
                <button className="copy-button" onClick={() => copyText(connection.tunnel_id!)}>Copy</button>
              ) : null}
            </div>
          </div>
          <div className="detail-row">
            <span>Local MCP</span>
            <div className="detail-value">
              <code>{connection.local_mcp_url}</code>
              <button className="copy-button" onClick={() => copyText(connection.local_mcp_url)}>Copy</button>
            </div>
          </div>
          {connection.health_url ? (
            <div className="detail-row subtle-row">
              <span>Runtime health</span>
              <code>{connection.health_url}</code>
            </div>
          ) : null}
        </div>
      )}

      {connection.error_detail && needsAttention ? (
        <details className="technical-details">
          <summary>Technical details</summary>
          <pre>{connection.error_detail}</pre>
        </details>
      ) : null}
    </section>
  );
}
