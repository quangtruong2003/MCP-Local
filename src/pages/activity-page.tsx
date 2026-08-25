import { useMemo, useState } from "react";
import { Loader2, RefreshCw, ScrollText, Search } from "lucide-react";
import { AuditCardList, AuditTable } from "@/components/features/activity/audit-table";
import { AuditDetailSheet } from "@/components/features/activity/audit-detail-sheet";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { EmptyState } from "@/components/shared/empty-state";
import { cn } from "@/lib/utils";
import type { AuditEvent } from "../api";
import type { Workspace } from "@/lib/types";

const MAX_EVENTS = 100;

type FilterTab = "all" | "allowed" | "blocked" | "errors";

const FILTER_TABS: ReadonlyArray<{ id: FilterTab; label: string }> = [
  { id: "all", label: "All" },
  { id: "allowed", label: "Allowed" },
  { id: "blocked", label: "Blocked" },
  { id: "errors", label: "Errors" },
];

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
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-1" role="tablist" aria-label="Filter activity">
          {FILTER_TABS.map(({ id, label }) => (
            <Button
              key={id}
              variant={tab === id ? "secondary" : "ghost"}
              size="sm"
              role="tab"
              aria-selected={tab === id}
              onClick={() => setTab(id)}
            >
              {label}
            </Button>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <div className="relative">
            <Search
              aria-hidden="true"
              className="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
            />
            <Input
              type="search"
              placeholder="Search operation, tool, path or error…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              className={cn("w-64 pl-8")}
              aria-label="Search activity"
            />
          </div>
          <Button variant="secondary" size="icon" onClick={() => void refresh()} disabled={refreshing} aria-label="Refresh activity">
            {refreshing ? <Loader2 aria-hidden="true" className="animate-spin" /> : <RefreshCw aria-hidden="true" />}
          </Button>
        </div>
      </div>

      <p className="-mt-2 text-[13px] text-muted-foreground">
        The last {MAX_EVENTS} recorded decisions, newest first.
      </p>

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
        <>
          {/* Desktop/table: real table. Narrow: stacked cards from the same array. */}
          <div className="max-md:hidden">
            <AuditTable events={visible} workspaceNames={workspaceNames} onOpen={setSelected} />
          </div>
          <div className="hidden max-md:block">
            <AuditCardList events={visible} workspaceNames={workspaceNames} onOpen={setSelected} />
          </div>
        </>
      )}

      <AuditDetailSheet
        event={selected}
        workspaceName={selected?.workspace_id ? workspaceNames.get(selected.workspace_id) : undefined}
        onClose={() => setSelected(null)}
      />
    </div>
  );
}
