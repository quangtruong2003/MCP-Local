import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  api,
  AuditEvent,
  parseErrorMessage,
  ServerStatus,
  tunnelStateFromJson,
  tunnelStateFromPayload,
  TunnelDisplay,
  Workspace,
} from "./api";
import StatusBar from "./components/StatusBar";
import WorkspaceList from "./components/WorkspaceList";
import PauseButton from "./components/PauseButton";
import AuditTable from "./components/AuditTable";
import SettingsDialog from "./components/SettingsDialog";

export default function App() {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [audit, setAudit] = useState<AuditEvent[]>([]);
  const [status, setStatus] = useState<ServerStatus | null>(null);
  const [paused, setPaused] = useState(false);
  const [tunnel, setTunnel] = useState<TunnelDisplay>({ state: "stopped" });
  const [showSettings, setShowSettings] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setWorkspaces(await api.listWorkspaces());
      setAudit(await api.recentAudit());
    } catch (err) {
      setError(parseErrorMessage(err));
    }
  }, []);

  useEffect(() => {
    (async () => {
      try {
        // Seed current tunnel state first so an already-connected sidecar
        // (auto-spawned at startup) is reflected even before any event.
        setTunnel(tunnelStateFromJson(await api.tunnelStatus()));
        const s = await api.serverStatus();
        setStatus(s);
        setPaused(s.paused);
        await refresh();
      } catch (err) {
        setError(parseErrorMessage(err));
      }
    })();
    const un1 = listen<unknown>("tunnel-status", (e) => setTunnel(tunnelStateFromPayload(e.payload)));
    const un2 = listen("access-paused", () => void refresh());
    return () => {
      un1.then((f) => f());
      un2.then((f) => f());
    };
  }, [refresh]);

  const togglePause = async () => {
    const next = !paused;
    setError(null);
    try {
      await api.setPaused(next);
      setPaused(next);
      await refresh();
    } catch (err) {
      setError(parseErrorMessage(err));
    }
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 16,
        padding: 16,
        maxWidth: 860,
        margin: "0 auto",
        minHeight: "100%",
      }}
    >
      <StatusBar port={status?.port} tunnel={tunnel} onOpenSettings={() => setShowSettings(true)} />

      {error && (
        <div
          role="alert"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            padding: "8px 12px",
            background: "var(--danger-bg)",
            border: "1px solid #fecaca",
            borderRadius: 8,
            color: "var(--danger)",
          }}
        >
          <span style={{ flex: 1 }}>{error}</span>
          <button onClick={() => setError(null)} aria-label="Dismiss error">
            ✕
          </button>
        </div>
      )}

      <WorkspaceList workspaces={workspaces} onChanged={refresh} onError={setError} />

      <PauseButton paused={paused} onToggle={() => void togglePause()} />

      <AuditTable events={audit} />

      {showSettings && <SettingsDialog onClose={() => setShowSettings(false)} onError={setError} />}
    </div>
  );
}
