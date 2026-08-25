import { useMemo, useState } from "react";
import { RefreshCw, ScrollText, Search } from "lucide-react";
import ActivityDetail from "../components/ActivityDetail";
import EmptyState from "../components/EmptyState";
import type { AuditEvent } from "../api";
import type { Workspace } from "@/lib/types";

/**
 * Activity history (plan Task 19): filterable list of the latest audit events
 * with a detail dialog showing the SAFE structured metadata the backend records.
 * Audit detail never contains file contents or script bodies; the UI only
 * renders what the backend stored as plain JSON.
 */

const MAX_EVENTS = 100;

type FilterTab = "all" | "allowed" | "blocked" | "errors";

const FILTER_TABS: ReadonlyArray<{ id: FilterTab; label: string }> = [
  { id: "all", label: "All" },
  { id: "allowed", label: "Allowed" },
  { id: "blocked", label: "Blocked" },
  { id: "errors", label: "Errors" },
];

/** mcp -> ChatGPT, ui -> App (plan Task 19 Step 1); raw source kept in detail. */
function sourceLabel(source: string): string {
  if (source === "mcp") return "ChatGPT";
  if (source === "ui") return "App";
  return source;
}

/** "read_file" -> "Read file". */
function humanizeOp(op: string): string {
  const spaced = op.replaceAll("_", " ");
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

function matchesFilter(event: AuditEvent, tab: FilterTab): boolean {
  switch (tab) {
    case "all":
      return true;
    case "allowed":
      return event.decision === "allow";
    case "blocked":
      return event.decision === "deny";
    case "errors":
      return event.error_code !== null && event.error_code !== undefined;
  }
}

interface Props {
  auditEvents: AuditEvent[];
  workspaces: Workspace[];
  /** Explicit, event-driven refresh of the audit trail (no polling). */
  onRefresh: () => Promise<void>;
  reportError: (message: string) => void;
}

export default function ActivityPage({ auditEvents, workspaces, onRefresh, reportError }: Props) {
  const [tab, setTab] = useState<FilterTab>("all");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<AuditEvent | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const workspaceNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const ws of workspaces) map.set(ws.id, ws.name);
    return map;
  }, [workspaces]);

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return auditEvents
      .slice(0, MAX_EVENTS)
      .filter((event) => matchesFilter(event, tab))
      .filter((event) => {
        if (!needle) return true;
        const haystack = [event.op, event.tool ?? "", event.path ?? "", event.error_code ?? ""]
          .join(" ")
          .toLowerCase();
        return haystack.includes(needle);
      });
  }, [auditEvents, tab, query]);

  async function refresh() {
    setRefreshing(true);
    try {
      await onRefresh();
    } catch (err) {
      reportError(err instanceof Error ? err.message : String(err));
    } finally {
      setRefreshing(false);
    }
  }

  return (
    <div className="activity-page">
      <section className="ws-toolbar">
        <div className="ws-toolbar-copy">
          <h2>Recent tool calls</h2>
          <p>The last {MAX_EVENTS} recorded decisions, newest first.</p>
        </div>
        <button className="btn btn-secondary" onClick={() => void refresh()} disabled={refreshing}>
          <RefreshCw aria-hidden="true" className={refreshing ? "spinner" : undefined} />
          Refresh
        </button>
      </section>

      <div className="activity-controls">
        <div className="activity-tabs" role="tablist" aria-label="Filter activity">
          {FILTER_TABS.map(({ id, label }) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              className={`filter-tab ${tab === id ? "selected" : ""}`}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </div>
        <label className="activity-search">
          <Search aria-hidden="true" />
          <input
            type="search"
            placeholder="Search operation, tool, path or error…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
      </div>

      {visible.length === 0 ? (
        <EmptyState
          icon={ScrollText}
          title={auditEvents.length === 0 ? "No activity yet" : "Nothing matches"}
          message={
            auditEvents.length === 0
              ? "Tool calls will appear here once ChatGPT connects."
              : "Try another filter or clear the search."
          }
        />
      ) : (
        <ul className="activity-list">
          {visible.map((event) => (
            <li key={event.id}>
              <button type="button" className="activity-row" onClick={() => setSelected(event)}>
                <span className="activity-time">{formatTime(event.ts)}</span>
                <span className="activity-source">{sourceLabel(event.source)}</span>
                <span className="activity-op">
                  {humanizeOp(event.op)}
                  {event.workspace_id && workspaceNames.has(event.workspace_id) ? (
                    <span className="activity-ws"> · {workspaceNames.get(event.workspace_id)}</span>
                  ) : null}
                </span>
                {event.path ? <code className="activity-path">{event.path}</code> : null}
                <span className={`decision-badge decision-${event.decision}`}>
                  {badgeLabel(event)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}

      <ActivityDetail event={selected} onClose={() => setSelected(null)} />
    </div>
  );
}

function badgeLabel(event: AuditEvent): string {
  if (event.decision === "allow") return "Allowed";
  if (event.decision === "deny") return "Blocked";
  return "Error";
}

/** Compact local time; falls back to the raw timestamp when unparsable. */
function formatTime(ts: string): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}
