import { ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { StatCard } from "@/components/shared/stat-card";
import { formatRelative } from "@/lib/format";
import type { AuditEvent, ServerStatus } from "@/api";
import type { ViewId } from "@/lib/types";

const RECENT_ROWS = 3;

interface Props {
  server: ServerStatus | null;
  auditEvents: AuditEvent[];
  onNavigate: (page: ViewId) => void;
}

export function ServerInfoCard({ server, auditEvents, onNavigate }: Props) {
  return (
    <Card className="gap-4 p-6">
      <div className="grid gap-3 sm:grid-cols-3">
        <StatCard
          label="Local port"
          value={server ? String(server.port) : "—"}
        />
        <StatCard
          label="Server state"
          value={server?.paused ? "Paused" : "Running"}
          hint={server?.paused ? "All tool calls are blocked" : "Requests are being served"}
        />
        <StatCard label="Recent activity" value={String(auditEvents.length)} hint="Latest 100 events" />
      </div>

      <div className="flex flex-col gap-1">
        {auditEvents.slice(0, RECENT_ROWS).map((event) => (
          <div key={event.id} className="flex items-center justify-between gap-3 text-[13px]">
            <span className="truncate font-medium">{event.op}</span>
            <span
              className="whitespace-nowrap text-muted-foreground"
              title={new Date(event.ts).toLocaleString()}
            >
              {formatRelative(event.ts)}
            </span>
          </div>
        ))}
        {auditEvents.length === 0 ? (
          <p className="text-[13px] text-muted-foreground">No activity recorded yet.</p>
        ) : null}
      </div>

      <Button variant="ghost" className="justify-start px-0 text-[13px]" onClick={() => onNavigate("activity")}>
        View all activity
        <ChevronRight aria-hidden="true" />
      </Button>
    </Card>
  );
}
