import { useState } from "react";
import type { ReactNode } from "react";
import StatCard from "../components/StatCard";
import ConnectionCard from "../components/ConnectionCard";
import EmptyState from "../components/EmptyState";
import { api, parseErrorMessage } from "../api";
import type { AuditEvent, ServerStatus } from "../api";
import type { ConnectionStatus, ViewId, Workspace } from "../types";

const RECENT_ACTIVITY_ROWS = 5;

interface Props {
  connection: ConnectionStatus | null;
  workspaces: Workspace[];
  auditEvents: AuditEvent[];
  server: ServerStatus | null;
  onNavigate: (page: ViewId) => void;
  reportError: (message: string) => void;
}

/** Formats an audit timestamp as a compact local time (HH:MM). */
function formatTime(ts: string): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export default function DashboardPage({
  connection,
  workspaces,
  auditEvents,
  server,
  onNavigate,
  reportError,
}: Props) {
  // One in-flight lifecycle request at a time; buttons disable while true.
  const [pending, setPending] = useState(false);

  async function runLifecycle(action: () => Promise<unknown>) {
    if (pending) return;
    setPending(true);
    try {
      await action();
      // The live `connection-status` event drives the UI from here.
    } catch (err) {
      reportError(
        typeof err === "object" && err !== null && "message" in err
          ? String((err as { message: unknown }).message)
          : String(err),
      );
    } finally {
      setPending(false);
    }
  }

  const writeCount = workspaces.filter((ws) => ws.can_write).length;
  const executeWorkspaces = workspaces.filter((ws) => ws.can_execute);
  const fullPowershellCount = executeWorkspaces.filter(
    (ws) => ws.execution_mode === "full_powershell",
  ).length;

  let summary: ReactNode;
  if (!connection) {
    summary = <EmptyState title="No connection data" message="Waiting for the local backend…" />;
  } else {
    summary = (
      <ConnectionCard
        state={connection.state}
        pending={pending}
        onStart={() => void runLifecycle(api.startConnection)}
        onStop={() => void runLifecycle(api.stopConnection)}
        onRetry={() => void runLifecycle(api.retryConnection)}
        onSignIn={() => {
          if (connection.state.state !== "needs_login") return;
          void api
            .openExternalUrl(connection.state.login_url)
            .catch((err) => reportError(parseErrorMessage(err)));
        }}
        onOpenAdmin={(url) => {
          void api.openExternalUrl(url).catch((err) => reportError(parseErrorMessage(err)));
        }}
        onError={reportError}
      />
    );
  }

  return (
    <div className="dashboard">
      {summary}

      <section className="dash-summary" aria-label="Overview">
        <StatCard
          label="Workspaces"
          value={workspaces.length}
          hint={
            workspaces.length === 0
              ? "None approved yet"
              : `${writeCount} allow ${writeCount === 1 ? "write" : "writes"}`
          }
        />
        <StatCard
          label="Execution"
          value={`${executeWorkspaces.length} / ${workspaces.length}`}
          hint={
            fullPowershellCount > 0
              ? `${fullPowershellCount} in Full PowerShell mode`
              : "Approved commands only"
          }
        />
        <StatCard
          label="Global access"
          value={server?.paused ? "Paused" : "Active"}
          hint={server?.paused ? "All tool calls are blocked" : "Requests are being served"}
        />
        <button className="stat-link" onClick={() => onNavigate("workspaces")}>
          Manage workspaces →
        </button>
      </section>

      <section className="dash-recent" aria-label="Recent activity">
        <div className="dash-recent-head">
          <h2>Recent activity</h2>
          <button className="link-button" onClick={() => onNavigate("activity")}>
            View all →
          </button>
        </div>
        {auditEvents.length === 0 ? (
          <EmptyState title="Nothing yet" message="Tool calls will appear here once ChatGPT connects." />
        ) : (
          <ul className="recent-list">
            {auditEvents.slice(0, RECENT_ACTIVITY_ROWS).map((event) => (
              <li key={event.id}>
                <span className="recent-time">{formatTime(event.ts)}</span>
                <span className="recent-source">{event.source}</span>
                <span className="recent-op">{event.op}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
