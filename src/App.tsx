import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  api,
  type AuditEvent,
  type ConnectionSettingsSummary,
  type ConnectionSnapshot,
  parseErrorMessage,
  type SaveConnectionSettingsInput,
  type ServerStatus,
  type Workspace,
} from "./api";
import AppShell from "./components/AppShell";
import ActivityPage from "./components/ActivityPage";
import ConnectionHero from "./components/ConnectionHero";
import SecurityPage from "./components/SecurityPage";
import type { AppPage } from "./components/Sidebar";
import SettingsPage from "./components/SettingsPage";
import StatCard from "./components/StatCard";
import WorkspaceList from "./components/WorkspaceList";

const INITIAL_CONNECTION: ConnectionSnapshot = {
  state: "stopped",
  local_mcp_url: "http://127.0.0.1:…/mcp",
  message: "Loading connection state…",
};

const PAGE_COPY: Record<AppPage, { title: string; subtitle: string }> = {
  dashboard: { title: "Dashboard", subtitle: "Start, stop and inspect your local ChatGPT MCP connection." },
  workspaces: { title: "Workspaces", subtitle: "Control exactly which local folders ChatGPT can read or modify." },
  activity: { title: "Activity", subtitle: "Review recent MCP decisions and manager actions." },
  security: { title: "Security", subtitle: "Understand and control the safeguards protecting your filesystem." },
  settings: { title: "Settings", subtitle: "Configure the reusable Secure MCP Tunnel and protected credentials." },
};

export default function App() {
  const [page, setPage] = useState<AppPage>("dashboard");
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [audit, setAudit] = useState<AuditEvent[]>([]);
  const [status, setStatus] = useState<ServerStatus | null>(null);
  const [paused, setPaused] = useState(false);
  const [connection, setConnection] = useState<ConnectionSnapshot>(INITIAL_CONNECTION);
  const [settings, setSettings] = useState<ConnectionSettingsSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshData = useCallback(async () => {
    const [workspaceRows, events] = await Promise.all([api.listWorkspaces(), api.recentAudit()]);
    setWorkspaces(workspaceRows);
    setAudit(events);
  }, []);

  const refreshSettings = useCallback(async () => {
    setSettings(await api.connectionSettingsSummary());
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        const [connectionState, serverState, workspaceRows, events, settingsState] = await Promise.all([
          api.connectionStatus(),
          api.serverStatus(),
          api.listWorkspaces(),
          api.recentAudit(),
          api.connectionSettingsSummary(),
        ]);
        setConnection(connectionState);
        setStatus(serverState);
        setPaused(serverState.paused);
        setWorkspaces(workspaceRows);
        setAudit(events);
        setSettings(settingsState);
      } catch (err) {
        setError(parseErrorMessage(err));
      }
    })();

    const unConnection = listen<ConnectionSnapshot>("connection-status", (event) => {
      setConnection(event.payload);
      if (event.payload.tunnel_id) void refreshSettings().catch(() => undefined);
    });
    const unPause = listen<boolean>("access-paused", (event) => {
      if (typeof event.payload === "boolean") setPaused(event.payload);
      void refreshData().catch(() => undefined);
    });
    return () => {
      unConnection.then((fn) => fn());
      unPause.then((fn) => fn());
    };
  }, [refreshData, refreshSettings]);

  const runAction = async (action: () => Promise<void>): Promise<boolean> => {
    setError(null);
    setBusy(true);
    try {
      await action();
      return true;
    } catch (err) {
      setError(parseErrorMessage(err));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const start = () => runAction(async () => {
    setConnection(await api.startConnection());
    await Promise.all([refreshSettings(), refreshData()]);
  });

  const stop = () => runAction(async () => {
    setConnection(await api.stopConnection());
    await refreshData();
  });

  const togglePause = () => runAction(async () => {
    const next = !paused;
    await api.setPaused(next);
    setPaused(next);
    await refreshData();
  });

  const saveSettings = (input: SaveConnectionSettingsInput) => runAction(async () => {
    setSettings(await api.saveConnectionSettings(input));
  });

  const regenerate = () => runAction(async () => {
    setConnection(await api.regenerateTunnel());
    await refreshSettings();
  });

  const title = PAGE_COPY[page];
  const ready = connection.state === "ready";
  const requestCount = audit.filter((item) => item.source === "mcp").length;
  const writeEnabled = workspaces.filter((workspace) => workspace.can_write).length;
  const toolCount = 8;

  const pageBody = useMemo(() => {
    switch (page) {
      case "workspaces":
        return <WorkspaceList workspaces={workspaces} onChanged={refreshData} onError={setError} />;
      case "activity":
        return <ActivityPage events={audit} workspaces={workspaces} />;
      case "security":
        return <SecurityPage paused={paused} busy={busy} onTogglePause={() => void togglePause()} />;
      case "settings":
        return (
          <SettingsPage
            settings={settings}
            busy={busy}
            onSave={saveSettings}
            onRegenerate={regenerate}
          />
        );
      default:
        return (
          <div className="stack-lg">
            <ConnectionHero
              connection={connection}
              busy={busy}
              onStart={() => void start()}
              onStop={() => void stop()}
              onOpenSettings={() => setPage("settings")}
            />

            <div className="stats-grid">
              <StatCard label="Workspaces" value={workspaces.length} hint={`${writeEnabled} with write access`} />
              <StatCard label="MCP tools" value={toolCount} hint="Workspace-scoped" />
              <StatCard label="Recent requests" value={requestCount} hint="Latest audit window" />
              <StatCard label="Local server" value={status?.port ?? "—"} hint="Loopback only" />
            </div>

            <section className="panel setup-panel">
              <div className="section-heading compact">
                <div>
                  <span className="eyebrow">How it works</span>
                  <h2>One persistent tunnel, explicit local access</h2>
                </div>
                <span className={`badge ${paused ? "badge-deny" : "badge-success"}`}>
                  {paused ? "Access paused" : "Policy active"}
                </span>
              </div>
              <div className="flow-steps">
                <div className="flow-step"><span>1</span><div><strong>Approve folders</strong><p>Only registered workspaces can reach the filesystem policy layer.</p></div></div>
                <div className="flow-step"><span>2</span><div><strong>Press Start</strong><p>The app reuses your saved Tunnel ID or creates it once on first setup.</p></div></div>
                <div className="flow-step"><span>3</span><div><strong>Ready means ready</strong><p>The UI waits for tunnel-client /readyz instead of trusting process spawn.</p></div></div>
              </div>
            </section>
          </div>
        );
    }
  }, [page, workspaces, refreshData, audit, paused, busy, settings, connection, status, writeEnabled, requestCount]);

  return (
    <AppShell
      page={page}
      onNavigate={setPage}
      ready={ready}
      title={title.title}
      subtitle={title.subtitle}
    >
      {error ? (
        <div className="global-alert" role="alert">
          <div><strong>Action failed</strong><span>{error}</span></div>
          <button className="icon-button" onClick={() => setError(null)} aria-label="Dismiss error">×</button>
        </div>
      ) : null}
      {pageBody}
    </AppShell>
  );
}
