import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { PauseCircle } from "lucide-react";
import { Toaster } from "sonner";
import AppShell from "./components/AppShell";
import EmptyState from "./components/EmptyState";
import OnboardingFlow from "./components/OnboardingFlow";
import { ThemeProvider, useTheme } from "./hooks/use-theme";
import { useToast } from "./hooks/use-toast";
import ActivityPage from "./pages/ActivityPage";
import DashboardPage from "./pages/DashboardPage";
import SecurityPage from "./pages/SecurityPage";
import SettingsPage from "./pages/SettingsPage";
import WorkspacesPage from "./pages/WorkspacesPage";
import { api, parseErrorMessage } from "./api";
import type { AuditEvent, ServerStatus } from "./api";
import { VIEW_META } from "./types";
import type {
  AppSettings,
  ConnectionStatus,
  ThemePreference,
  ViewId,
  Workspace,
} from "./types";

export default function App() {
  // --- Shell state (plan Step 6): page, data load, toasts/errors, onboarding ---
  const [page, setPage] = useState<ViewId>("dashboard");
  const toast = useToast();
  // Null until settings load decides whether onboarding should run at all.
  const [onboardingVisible, setOnboardingVisible] = useState<boolean | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [initialLoadDone, setInitialLoadDone] = useState(false);
  const [connection, setConnection] = useState<ConnectionStatus | null>(null);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [auditEvents, setAuditEvents] = useState<AuditEvent[]>([]);
  const [server, setServer] = useState<ServerStatus | null>(null);

  const reportError = useCallback(
    (message: string) => {
      toast.error(message);
    },
    [toast],
  );

  // Event-driven audit refresh (plan Task 19 Step 5): called after mutations
  // and connection events; no polling.
  const refreshAudit = useCallback(async () => {
    try {
      setAuditEvents(await api.recentAudit());
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }, [reportError]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void (async () => {
      try {
        const [status, appSettings] = await Promise.all([
          api.connectionStatus(),
          api.getAppSettings(),
        ]);
        if (disposed) return;
        setConnection(status);
        setSettings(appSettings);
        setOnboardingVisible(!appSettings.onboarding_complete);
      } catch (err) {
        reportError(parseErrorMessage(err));
      }

      try {
        unlisten = await listen<ConnectionStatus>("connection-status", (e) => {
          setConnection(e.payload);
          void refreshAudit();
        });
      } catch (err) {
        reportError(parseErrorMessage(err));
      }
    })();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [reportError, refreshAudit]);

  // Initial data load: workspaces, audit trail and server state.
  useEffect(() => {
    void (async () => {
      try {
        const [wsList, events, status] = await Promise.all([
          api.listWorkspaces(),
          api.recentAudit(),
          api.serverStatus(),
        ]);
        setWorkspaces(wsList);
        setAuditEvents(events);
        setServer(status);
      } catch (err) {
        reportError(parseErrorMessage(err));
      } finally {
        setInitialLoadDone(true);
      }
    })();
  }, [reportError]);

  // Ready only when the funnel connection is fully online.
  const connectionReady = connection?.state.state === "online";

  // Onboarding completion persists ONLY via Finish at the Online stage;
  // dismissing the dialog never marks it complete (plan Task 17 Step 3).
  async function finishOnboarding() {
    if (!settings) return;
    try {
      const updated = await api.updateAppSettings({ ...settings, onboarding_complete: true });
      setSettings(updated);
    } catch (err) {
      reportError(parseErrorMessage(err));
      return;
    }
    setOnboardingVisible(false);
  }

  // Security page (plan Task 20): re-pull status after setPaused; rotate/reset
  // return the fresh ConnectionStatus directly.
  const refreshConnection = useCallback(async () => {
    try {
      setConnection(await api.connectionStatus());
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }, [reportError]);

  // Settings page (plan Task 21): persists then applies the returned value;
  // throws propagate to SettingsPage, which reports and rolls back.
  const saveSettings = useCallback(async (next: AppSettings) => {
    const updated = await api.updateAppSettings(next);
    setSettings(updated);
  }, []);

  const meta = VIEW_META[page];
  const paused = connection?.paused === true;
  const savedTheme: ThemePreference = settings?.theme ?? "system";

  const handleThemeChange = useCallback(
    async (t: ThemePreference) => {
      if (!settings) return;
      try {
        await saveSettings({ ...settings, theme: t });
      } catch (err) {
        reportError(parseErrorMessage(err));
      }
    },
    [settings, saveSettings, reportError],
  );

  return (
    <ThemeProvider
      initialTheme={savedTheme}
      onThemeChange={(t) => void handleThemeChange(t)}
    >
      <AppShell page={page} onNavigate={setPage} ready={connectionReady} title={meta.title} subtitle={meta.subtitle}>
      {onboardingVisible ? (
        <OnboardingFlow
          connection={connection}
          workspaces={workspaces}
          onWorkspacesChanged={async () => {
            const [wsList] = await Promise.all([api.listWorkspaces(), refreshAudit()]);
            setWorkspaces(wsList);
          }}
          onClose={() => setOnboardingVisible(false)}
          onFinish={() => finishOnboarding()}
          reportError={reportError}
        />
      ) : null}

      {/* Global paused strip (plan Task 20 Step 2): shown on every page while
          MCP access is paused, regardless of connection state. */}
      {paused ? (
        <div className="paused-strip" role="status">
          <PauseCircle aria-hidden="true" />
          <span>MCP access is paused. Connection may be online, but file tools are blocked.</span>
        </div>
      ) : null}

      {!initialLoadDone ? (
        <EmptyState title="Loading…" message="Reading local app state." />
      ) : page === "dashboard" ? (
        <DashboardPage
          connection={connection}
          workspaces={workspaces}
          auditEvents={auditEvents}
          server={server}
          onNavigate={setPage}
          reportError={reportError}
        />
      ) : page === "workspaces" ? (
        <WorkspacesPage
          workspaces={workspaces}
          onWorkspacesChanged={async () => {
            const [wsList] = await Promise.all([api.listWorkspaces(), refreshAudit()]);
            setWorkspaces(wsList);
          }}
          reportError={reportError}
        />
      ) : page === "activity" ? (
        <ActivityPage
          auditEvents={auditEvents}
          workspaces={workspaces}
          onRefresh={async () => setAuditEvents(await api.recentAudit())}
          reportError={reportError}
        />
      ) : page === "security" ? (
        <SecurityPage
          connection={connection}
          workspaces={workspaces}
          onPauseChanged={refreshConnection}
          onConnectionChanged={setConnection}
          reportError={reportError}
        />
      ) : page === "settings" ? (
        <SettingsPage
          settings={settings}
          connection={connection}
          onSaveSettings={saveSettings}
          reportError={reportError}
        />
      ) : (
        <EmptyState title={`${meta.title} coming soon`} message="This page is wired up by the next milestone." />
      )}

      </AppShell>
      <ThemedToaster />
    </ThemeProvider>
  );
}

/** Renders the themed toaster; lives inside ThemeProvider so it can read the
 * resolved light/dark value. */
function ThemedToaster() {
  const { resolvedTheme } = useTheme();
  return <Toaster position="bottom-right" theme={resolvedTheme} richColors closeButton />;
}
