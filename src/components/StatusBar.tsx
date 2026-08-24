import type { ConnectionSnapshot } from "../api";

/** @deprecated The main UI now uses ConnectionHero. Kept as a small compatibility component. */
export default function StatusBar({ connection }: { connection: ConnectionSnapshot }) {
  return (
    <div className="legacy-status-bar">
      <span className={`status-dot ${connection.state === "ready" ? "ready" : "offline"}`} />
      <strong>{connection.state === "ready" ? "Ready" : "Offline"}</strong>
      <code>{connection.local_mcp_url}</code>
    </div>
  );
}
