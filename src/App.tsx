import { useCallback, useEffect, useState } from "react";
import { Toaster } from "sonner";
import OnboardingFlow from "./components/OnboardingFlow";
import EmptyState from "./components/EmptyState";
import { AppShell } from "./components/layout/app-shell";
import { ThemeProvider, useTheme } from "./hooks/use-theme";
import { useToast } from "./hooks/use-toast";
import { useAudit } from "./hooks/use-audit";
import { useConnection } from "./hooks/use-connection";
import { useWorkspaces } from "./hooks/use-workspaces";
import ActivityPage from "./pages/ActivityPage";
import DashboardPage from "./pages/dashboard-page";
import SecurityPage from "./pages/security-page";
import SettingsPage from "./pages/settings-page";
import WorkspacesPage from "./pages/workspaces-page";
import { api, parseErrorMessage } from "./api";
import type { ServerStatus } from "./api";
import { VIEW_META } from "@/lib/types";
import type { AppSettings, ThemePreference, ViewId } from "@/lib/types";

export default function App() {
  const [page, setPage] = useState<ViewId>("dashboard");
  const toast = useToast();
  // Null until settings load decides whether onboarding should run at all.
  const [onboardingVisible, setOnboardingVisible] = useState<boolean | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [initialLoadDone, setInitialLoadDone] = useState(false);
  const [server, setServer] = useState<ServerStatus | null>(null);

  const reportError = useCallback(
    (message: string) => {
      toast.error(message);
    },
    [toast],
  );

  // Event-driven audit refresh (plan Task 19 Step 5): called after mutations
  // and connection events; no polling.
  const { auditEvents, refreshAudit } = useAudit(reportError);
  const { workspaces, reloadWorkspaces } = useWorkspaces(reportError);
  const { connection, refreshConnection, setConnection } = useConnection(
    reportError,
    () => void refreshAudit(),
  );

  // Settings load decides whether onboarding runs at all.
  useEffect(() => {
    let disposed = false;
    void (async () => {
      try {
        const appSettings = await api.getAppSettings();
        if (!disposed) {
          setSettings(appSettings);
          setOnboardingVisible(!appSettings.onboarding_complete);
        }
      } catch (err) {
        reportError(parseErrorMessage(err));
      }
    })();
    return () => {
      disposed = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Initial data load: workspaces, audit trail and server state.
  useEffect(() => {
    let disposed = false;
    void (async () => {
      try {
        const [, , status] = await Promise.all([
          reloadWorkspaces(),
          refreshAudit(),
          api.serverStatus(),
        ]);
        if (!disposed) setServer(status);
      } catch (err) {
        if (!disposed) reportError(parseErrorMessage(err));
      } finally {
        if (!disposed) setInitialLoadDone(true);
      }
    })();
    return () => {
      disposed = true;
    };
  }, [reportError, reloadWorkspaces, refreshAudit]);

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
      <AppShell page={page} onNavigate={setPage} ready={connectionReady} paused={paused}>
      {onboardingVisible ? (
        <OnboardingFlow
          connection={connection}
          workspaces={workspaces}
          onWorkspacesChanged={async () => {
            await Promise.all([reloadWorkspaces(), refreshAudit()]);
          }}
          onClose={() => setOnboardingVisible(false)}
          onFinish={() => finishOnboarding()}
          reportError={reportError}
        />
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
            await Promise.all([reloadWorkspaces(), refreshAudit()]);
          }}
          reportError={reportError}
        />
      ) : page === "activity" ? (
        <ActivityPage
          auditEvents={auditEvents}
          workspaces={workspaces}
          onRefresh={refreshAudit}
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
