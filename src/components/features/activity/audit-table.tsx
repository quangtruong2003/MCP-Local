import { formatRelative } from "@/lib/format";
import { CopyButton } from "@/components/shared/copy-button";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { AuditEvent } from "@/api";

/** "read_file" -> "Read file". */
export function humanizeOp(op: string): string {
  const spaced = op.replaceAll("_", " ");
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

/** mcp -> ChatGPT, ui -> App (raw source kept in the detail sheet). */
export function sourceLabel(source: string): string {
  if (source === "mcp") return "ChatGPT";
  if (source === "ui") return "App";
  return source;
}

export function badgeLabel(event: AuditEvent): string {
  if (event.decision === "allow") return "Allowed";
  if (event.decision === "deny") return "Blocked";
  return "Error";
}

function DecisionBadge({ event }: { event: AuditEvent }) {
  const ok = event.decision === "allow";
  return (
    <Badge variant="outline" className="gap-1.5">
      <span
        aria-hidden="true"
        className={cn("size-2 rounded-full", ok ? "bg-success" : "bg-destructive")}
      />
      {badgeLabel(event)}
    </Badge>
  );
}

interface RowProps {
  event: AuditEvent;
  workspaceName?: string;
  onOpen: (event: AuditEvent) => void;
}

function TableRowButton({ event, workspaceName, onOpen }: RowProps) {
  return (
    <TableRow
      tabIndex={0}
      role="button"
      aria-label={`Open details for ${humanizeOp(event.op)}`}
      onClick={() => onOpen(event)}
      onKeyDown={(e) => {
        if (e.key === "Enter") onOpen(event);
      }}
      className="cursor-pointer"
    >
      <TableCell className="whitespace-nowrap text-muted-foreground" title={new Date(event.ts).toLocaleString()}>
        {formatRelative(event.ts)}
      </TableCell>
      <TableCell>{sourceLabel(event.source)}</TableCell>
      <TableCell className="font-medium">
        {humanizeOp(event.op)}
        {workspaceName ? <span className="text-muted-foreground"> · {workspaceName}</span> : null}
      </TableCell>
      <TableCell>
        <DecisionBadge event={event} />
      </TableCell>
      <TableCell className="max-w-[16rem]">
        {event.path ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <code className="block truncate font-mono text-xs">{event.path}</code>
              </TooltipTrigger>
              <TooltipContent side="top" className="max-w-md break-all">
                {event.path}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          <span className="text-muted-foreground">—</span>
        )}
      </TableCell>
    </TableRow>
  );
}

interface Props {
  events: AuditEvent[];
  workspaceNames: Map<string, string>;
  onOpen: (event: AuditEvent) => void;
}

export function AuditTable({ events, workspaceNames, onOpen }: Props) {
  return (
    <div className="overflow-hidden rounded-md border">
      <Table>
        <TableHeader className="sticky top-0 z-[1] bg-card">
          <TableRow>
            <TableHead className="w-24">Time</TableHead>
            <TableHead className="w-24">Source</TableHead>
            <TableHead>Operation</TableHead>
            <TableHead className="w-32">Decision</TableHead>
            <TableHead>Path</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {events.map((event) => (
            <TableRowButton
              key={event.id}
              event={event}
              workspaceName={
                event.workspace_id ? workspaceNames.get(event.workspace_id) : undefined
              }
              onOpen={onOpen}
            />
          ))}
        </TableBody>
      </Table>
    </div>
  );
}

/** Narrow-window rendering of the same filtered array. */
export function AuditCardList({ events, workspaceNames, onOpen }: Props) {
  return (
    <div className="flex flex-col gap-2">
      {events.map((event) => (
        <button
          key={event.id}
          type="button"
          onClick={() => onOpen(event)}
          className="flex flex-col gap-1 rounded-md border p-3 text-left transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <div className="flex items-center justify-between gap-2">
            <span className="font-medium">{humanizeOp(event.op)}</span>
            <DecisionBadge event={event} />
          </div>
          <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
            <span>{sourceLabel(event.source)}</span>
            <span title={new Date(event.ts).toLocaleString()}>{formatRelative(event.ts)}</span>
          </div>
          {event.path ? (
            <div className="flex items-center gap-1">
              <code className="min-w-0 flex-1 truncate font-mono text-xs">{event.path}</code>
              <CopyButton value={event.path} label="Copy path" />
            </div>
          ) : null}
        </button>
      ))}
    </div>
  );
}
