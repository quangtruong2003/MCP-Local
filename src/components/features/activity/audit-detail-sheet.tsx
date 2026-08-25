import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { sourceLabel, humanizeOp, badgeLabel } from "./audit-table";
import { formatTime } from "@/lib/format";
import type { AuditEvent } from "@/api";

interface Props {
  event: AuditEvent | null;
  workspaceName?: string;
  onClose: () => void;
}

function DefinitionRow({ label, value }: { label: string; value: React.ReactNode }) {
  if (value === null || value === undefined || value === "") return null;
  return (
    <div className="flex flex-col gap-0.5 border-b py-2 last:border-b-0 sm:flex-row sm:items-start sm:justify-between">
      <dt className="text-[13px] text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-all text-right text-sm">{value}</dd>
    </div>
  );
}

export function AuditDetailSheet({ event, workspaceName, onClose }: Props) {
  return (
    <Sheet open={event !== null} onOpenChange={(o) => !o && onClose()}>
      <SheetContent side="right" className="w-full overflow-y-auto sm:max-w-md">
        <SheetHeader>
          <SheetTitle>{event ? humanizeOp(event.op) : "Activity detail"}</SheetTitle>
          <SheetDescription>
            Structured metadata recorded by the manager. Never contains file contents or script bodies.
          </SheetDescription>
        </SheetHeader>
        {event ? (
          <div className="flex flex-col gap-4 px-4 pb-6">
            <dl>
              <DefinitionRow label="Time" value={formatTime(event.ts)} />
              <DefinitionRow label="Source" value={sourceLabel(event.source)} />
              <DefinitionRow
                label="Workspace"
                value={
                  event.workspace_id
                    ? (workspaceName ?? event.workspace_id)
                    : null
                }
              />
              <DefinitionRow label="Tool" value={event.tool ?? null} />
              <DefinitionRow label="Decision" value={badgeLabel(event)} />
              <DefinitionRow label="Error code" value={event.error_code ?? null} />
              <DefinitionRow
                label="Path"
                value={
                  event.path ? <code className="font-mono text-xs break-all">{event.path}</code> : null
                }
              />
            </dl>

            {event.detail !== undefined && event.detail !== null ? (
              <div className="flex flex-col gap-1.5">
                <dt className="text-[13px] font-medium">Detail</dt>
                <pre className="overflow-x-auto rounded-md bg-muted p-3 font-mono text-xs">
                  {JSON.stringify(event.detail, null, 2)}
                </pre>
              </div>
            ) : null}
          </div>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}
