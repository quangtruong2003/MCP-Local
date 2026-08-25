import { AlertTriangle, Loader2, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { ExecutionModePicker } from "./execution-mode-picker";
import type { ExecutionMode, Workspace } from "@/lib/types";

/** Concise list shown under Approved Commands mode (mirrors backend policy defaults). */
const APPROVED_COMMANDS = [
  "git",
  "cargo",
  "rustc",
  "npm",
  "npx",
  "node",
  "pnpm",
  "yarn",
  "python",
  "pip",
  "go",
  "dotnet",
];

interface Props {
  ws: Workspace;
  busy: boolean;
  registerRef?: (node: HTMLElement | null) => void;
  onToggleCapability: (ws: Workspace, cap: "read" | "write") => void;
  onToggleExecute: (ws: Workspace) => void;
  onChangeMode: (ws: Workspace, mode: ExecutionMode) => void;
  onRemove: (ws: Workspace) => void;
}

function PermissionRow({
  id,
  label,
  description,
  checked,
  disabled,
  onToggle,
}: {
  id: string;
  label: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <Label htmlFor={id} className="flex-col items-start gap-0.5 font-normal">
        <span className="text-sm font-medium">{label}</span>
        <span className="text-[13px] text-muted-foreground">{description}</span>
      </Label>
      <Switch id={id} checked={checked} disabled={disabled} onCheckedChange={onToggle} />
    </div>
  );
}

export function WorkspaceCard({
  ws,
  busy,
  registerRef,
  onToggleCapability,
  onToggleExecute,
  onChangeMode,
  onRemove,
}: Props) {
  return (
    // Ref wrapper: vendored Card is a plain function component (React 18),
    // so the scroll/focus target must be an outer DOM element.
    <div
      ref={registerRef}
      tabIndex={-1}
      aria-busy={busy}
      className="rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
    <Card className="flex flex-col gap-4 p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold">{ws.name}</div>
          <code className="block truncate text-xs text-muted-foreground" title={ws.root_path}>
            {ws.root_path}
          </code>
        </div>
        {busy ? <Loader2 aria-hidden="true" className="size-4 animate-spin text-muted-foreground" /> : null}
      </div>

      <div role="group" aria-label={`Permissions for ${ws.name}`} className="flex flex-col gap-3 rounded-md border p-3">
        <PermissionRow
          id={`ws-read-${ws.id}`}
          label="Read"
          description="List, search and inspect files"
          checked={ws.can_read}
          disabled={busy}
          onToggle={() => onToggleCapability(ws, "read")}
        />
        <PermissionRow
          id={`ws-write-${ws.id}`}
          label="Write"
          description="Create files and apply anchored patches"
          checked={ws.can_write}
          disabled={busy}
          onToggle={() => onToggleCapability(ws, "write")}
        />
        <PermissionRow
          id={`ws-exec-${ws.id}`}
          label="Execute"
          description="Run approved programs inside this folder"
          checked={ws.can_execute}
          disabled={busy}
          onToggle={() => onToggleExecute(ws)}
        />
      </div>

      {ws.can_execute ? (
        <div className="flex flex-col gap-2">
          <ExecutionModePicker value={ws.execution_mode} disabled={busy} onChange={(mode) => onChangeMode(ws, mode)} />
          {ws.execution_mode === "approved_commands" ? (
            <div className="flex flex-wrap items-center gap-1.5" aria-label="Approved executables">
              <span className="text-[13px] text-muted-foreground">Allowed executables:</span>
              {APPROVED_COMMANDS.map((cmd) => (
                <Badge key={cmd} variant="secondary" className="font-mono text-xs font-normal">
                  {cmd}
                </Badge>
              ))}
            </div>
          ) : (
            <p className="flex items-start gap-1.5 text-[13px] text-warning">
              <AlertTriangle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-warning" />
              High risk: unrestricted PowerShell scripts run with your Windows account permissions.
            </p>
          )}
        </div>
      ) : null}

      <div className="flex justify-end border-t pt-3">
        <Button
          variant="ghost"
          className="text-destructive hover:bg-destructive/10 hover:text-destructive"
          onClick={() => onRemove(ws)}
          disabled={busy}
        >
          <Trash2 aria-hidden="true" />
          Remove
        </Button>
      </div>
    </Card>
    </div>
  );
}
