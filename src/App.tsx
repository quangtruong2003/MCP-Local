import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { CheckCircle2, Info, XOctagon } from "lucide-react";
import AppShell from "./components/AppShell";
import EmptyState from "./components/EmptyState";
import OnboardingFlow from "./components/OnboardingFlow";
import ActivityPage from "./pages/ActivityPage";
import DashboardPage from "./pages/DashboardPage";
import WorkspacesPage from "./pages/WorkspacesPage";
import { api, parseErrorMessage } from "./api";
import type { AuditEvent, ServerStatus } from "./api";
import { VIEW_META } from "./types";
import type { AppSettings, ConnectionStatus, Toast, ToastTone, ViewId, Workspace } from "./types";

const TOAST_TTL_MS = 5000;
const TOAST_ICONS: Record<ToastTone, typeof Info> = {
  info: Info,
  success: CheckCircle2,
  error: XOctagon,
};

export default function App() {
  // --- Shell state (plan Step 6): page, data load, toasts/errors, onboarding ---
  const [page, setPage] = useState<ViewId>("dashboard");
  const [toasts, setToasts] = useState<Toast[]>([]);
  // Null until settings load decides whether onboarding should run at all.
  const [onboardingVisible, setOnboardingVisible] = useState<boolean | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [initialLoadDone, setInitialLoadDone] = useState(false);
  const [connection, setConnection] = useState<ConnectionStatus | null>(null);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [auditEvents, setAuditEvents] = useState<AuditEvent[]>([]);
  const [server, setServer] = useState<ServerStatus | null>(null);
  const nextToastId = useRef(1);

  const pushToast = useCallback((tone: ToastTone, message: string) => {
    const id = nextToastId.current++;
    setToasts((current) => [...current, { id, tone, message }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, TOAST_TTL_MS);
  }, []);

  const dismissToast = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const reportError = useCallback(
    (message: string) => {
      pushToast("error", message);
    },
    [pushToast],
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

  const meta = VIEW_META[page];

  return (
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
      ) : (
        <EmptyState title={`${meta.title} coming soon`} message="This page is wired up by the next milestone." />
      )}

      <div className="toast-stack">
        {toasts.map((toast) => {
          const Icon = TOAST_ICONS[toast.tone];
          return (
            <div key={toast.id} className={`toast toast-${toast.tone}`} role={toast.tone === "error" ? "alert" : "status"}>
              <Icon aria-hidden="true" />
              <span>{toast.message}</span>
              <button className="icon-button" aria-label="Dismiss notification" onClick={() => dismissToast(toast.id)}>
                ×
              </button>
            </div>
          );
        })}
      </div>
    </AppShell>
  );
}
