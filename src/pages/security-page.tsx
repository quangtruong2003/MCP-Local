import { useState } from "react";
import { Loader2, KeyRound, ShieldAlert } from "lucide-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { ConfirmDialog } from "@/components/shared/confirm-dialog";
import { CopyButton } from "@/components/shared/copy-button";
import { api, parseErrorMessage } from "../api";
import type { ConnectionStatus, Workspace } from "@/lib/types";

const ROTATE_TITLE = "Rotate connection URL?";
const ROTATE_BODY =
  "The current URL will stop working and you will need to update the MCP endpoint in ChatGPT. Your Tailscale hostname will stay the same.";
const RESET_TITLE = "Reset Tailscale identity?";
const RESET_BODY =
  "This app will need to sign in to Tailscale again and its *.ts.net hostname may change.";
const RESET_KEYWORD = "RESET";

interface Props {
  connection: ConnectionStatus | null;
  workspaces: Workspace[];
  /** Re-pulls connection status after setPaused (App owns shell state). */
  onPauseChanged: () => Promise<void>;
  /** Applies a new ConnectionStatus returned by rotate/reset to App state. */
  onConnectionChanged: (status: ConnectionStatus) => void;
  reportError: (message: string) => void;
}

export default function SecurityPage({
  connection,
  workspaces,
  onPauseChanged,
  onConnectionChanged,
  reportError,
}: Props) {
  const [pending, setPending] = useState(false);
  const [rotateOpen, setRotateOpen] = useState(false);
  const [rotatedEndpoint, setRotatedEndpoint] = useState<string | null>(null);
  const [resetOpen, setResetOpen] = useState(false);
  const [resetText, setResetText] = useState("");

  const paused = connection?.paused === true;
  const writeCount = workspaces.filter((ws) => ws.can_write).length;
  const executeCount = workspaces.filter((ws) => ws.can_execute).length;
  const fullPowershellCount = workspaces.filter(
    (ws) => ws.can_execute && ws.execution_mode === "full_powershell",
  ).length;

  const rows: ReadonlyArray<{ label: string; value: string }> = [
    { label: "Public endpoint protection", value: "Capability URL · Active" },
    { label: "Sensitive files", value: "Protected" },
    { label: "Path confinement", value: "Active" },
    { label: "Concurrent edit protection", value: "Active" },
    { label: "Write-enabled workspaces", value: String(writeCount) },
    { label: "Execute-enabled workspaces", value: String(executeCount) },
    { label: "Full PowerShell workspaces", value: `${fullPowershellCount} · High risk when > 0` },
    { label: "Global MCP access", value: paused ? "Paused" : "Active" },
  ];

  async function setPaused(next: boolean) {
    setPending(true);
    try {
      await api.setPaused(next);
      await onPauseChanged();
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setPending(false);
    }
  }

  async function rotateUrl() {
    setRotateOpen(false);
    setPending(true);
    try {
      const status = await api.rotateConnectionUrl();
      onConnectionChanged(status);
      setRotatedEndpoint(status.state.state === "online" ? status.state.endpoint : null);
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setPending(false);
    }
  }

  async function resetIdentity() {
    setPending(true);
    try {
      const status = await api.resetTailscaleIdentity(true, true);
      onConnectionChanged(status);
      setRotatedEndpoint(null);
      setResetOpen(false);
      setResetText("");
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <Card className="gap-4 p-6">
        <div>
          <h2 className="text-sm font-semibold">Protection summary</h2>
          <p className="text-[13px] text-muted-foreground">
            What stands between ChatGPT and your filesystem right now.
          </p>
        </div>
        <ul className="flex flex-col divide-y">
          {rows.map((row) => (
            <li key={row.label} className="flex items-center justify-between gap-3 py-2 text-sm first:pt-0 last:pb-0">
              <span>{row.label}</span>
              <strong className="text-right font-medium">{row.value}</strong>
            </li>
          ))}
        </ul>
        {fullPowershellCount > 0 ? (
          <p role="alert" className="flex items-start gap-2 text-[13px] text-warning-foreground">
            <ShieldAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-warning" />
            Full PowerShell is enabled for one or more workspaces. Working-directory confinement is
            not a Windows security sandbox; scripts may access other resources available to your
            account.
          </p>
        ) : null}
      </Card>

      <Card className="gap-3 p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <Label htmlFor="pause-mcp" className="text-sm font-semibold">
              Pause MCP access
            </Label>
            <p className="text-[13px] text-muted-foreground">
              Pause all tool calls without tearing down the connection.
            </p>
          </div>
          <Switch
            id="pause-mcp"
            checked={paused}
            disabled={pending}
            onCheckedChange={(next) => void setPaused(next)}
          />
        </div>
      </Card>

      <Card className="gap-3 p-6">
        <div>
          <h2 className="text-sm font-semibold">Connection URL</h2>
          <p className="text-[13px] text-muted-foreground">
            Issue a fresh capability URL if you think the current one has leaked.
          </p>
        </div>
        {rotatedEndpoint ? (
          <div className="flex items-center gap-2">
            <code
              className="min-w-0 flex-1 truncate rounded-sm bg-muted px-2 py-1.5 font-mono text-xs"
              title={rotatedEndpoint}
            >
              {rotatedEndpoint}
            </code>
            <CopyButton value={rotatedEndpoint} label="Copy new endpoint URL" />
          </div>
        ) : null}
        <div>
          <Button variant="secondary" onClick={() => setRotateOpen(true)} disabled={pending}>
            {pending ? <Loader2 aria-hidden="true" className="animate-spin" /> : <KeyRound aria-hidden="true" />}
            Rotate connection URL…
          </Button>
        </div>
      </Card>

      <Card className="gap-3 border-destructive/40 p-6">
        <div>
          <h2 className="text-sm font-semibold">Advanced</h2>
          <p className="text-[13px] text-muted-foreground">
            Last resort: forget this machine's Tailscale identity and re-authenticate.
          </p>
        </div>
        <div>
          <Button
            variant="outline"
            className="border-destructive/50 text-destructive hover:bg-destructive/10 hover:text-destructive"
            onClick={() => {
              setResetText("");
              setResetOpen(true);
            }}
            disabled={pending}
          >
            Reset Tailscale identity…
          </Button>
        </div>
      </Card>

      <ConfirmDialog
        open={rotateOpen}
        title={ROTATE_TITLE}
        message={ROTATE_BODY}
        tone="warning"
        confirmLabel="Rotate URL"
        onCancel={() => setRotateOpen(false)}
        onConfirm={() => void rotateUrl()}
      />

      {/* Typed-keyword confirm: kept as a bespoke AlertDialog because the
          shared ConfirmDialog cannot host the RESET input. */}
      <AlertDialog
        open={resetOpen}
        onOpenChange={(o) => {
          if (!o && !pending) setResetOpen(false);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle className="flex items-center gap-2">
              <ShieldAlert aria-hidden="true" className="size-5 text-destructive" />
              {RESET_TITLE}
            </AlertDialogTitle>
            <AlertDialogDescription>{RESET_BODY}</AlertDialogDescription>
          </AlertDialogHeader>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="reset-confirm-input">Type {RESET_KEYWORD} to confirm</Label>
            <Input
              id="reset-confirm-input"
              value={resetText}
              autoFocus
              disabled={pending}
              onChange={(event) => setResetText(event.target.value)}
              placeholder={RESET_KEYWORD}
              spellCheck={false}
            />
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={() => setResetOpen(false)} disabled={pending}>
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => void resetIdentity()}
              disabled={pending || resetText !== RESET_KEYWORD}
              className="bg-destructive text-white hover:bg-destructive/90"
            >
              {pending ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
              Reset
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
