import { useCallback, useEffect, useRef, useState } from "react";
import { CheckCircle2, Info, XOctagon } from "lucide-react";
import AppShell from "./components/AppShell";
import EmptyState from "./components/EmptyState";
import { VIEW_META } from "./types";
import type { Toast, ToastTone, ViewId } from "./types";

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
  const [onboardingVisible, setOnboardingVisible] = useState(true);
  const [initialLoadDone, setInitialLoadDone] = useState(false);
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

  // Placeholder for Task 15: real initial data load + connection-status subscription.
  useEffect(() => {
    void (async () => {
      try {
        // TODO(Task 15): api.connectionStatus(), api.serverStatus(), workspaces, audit.
        await Promise.resolve();
      } catch (err) {
        reportError(err instanceof Error ? err.message : String(err));
      } finally {
        setInitialLoadDone(true);
      }
    })();
  }, [reportError]);

  // Placeholder connection readiness until Task 15 wires the live subscription.
  const connectionReady = false;

  const meta = VIEW_META[page];

  return (
    <AppShell page={page} onNavigate={setPage} ready={connectionReady} title={meta.title} subtitle={meta.subtitle}>
      {onboardingVisible ? (
        <div className="callout" role="note">
          <div>
            <strong>Welcome</strong>
            <span>Approve at least one workspace folder, then press Start on the Dashboard.</span>
          </div>
          <button className="btn btn-secondary" onClick={() => setOnboardingVisible(false)}>
            Got it
          </button>
        </div>
      ) : null}

      {!initialLoadDone ? (
        <EmptyState title="Loading…" message="Reading local app state." />
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
