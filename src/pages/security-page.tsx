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
import { EndpointField } from "@/components/shared/endpoint-field";
import { api, parseErrorMessage } from "../api";
import type { AppSettings, ConnectionStatus, Workspace } from "@/lib/types";

const ROTATE_TITLE = "Rotate connection URL?";
const ROTATE_BODY =
  "The current URL will stop working and you will need to update the MCP endpoint in ChatGPT. Your Tailscale hostname will stay the same.";
const RESET_TITLE = "Reset Tailscale identity?";
const RESET_BODY =
  "This app will need to sign in to Tailscale again and its *.ts.net hostname may change.";
const RESET_KEYWORD = "RESET";
const UNRESTRICTED_TITLE = "Enable unrestricted access?";
const UNRESTRICTED_BODY =
  "ChatGPT will be able to read, create and edit ANY file in your workspaces — including credentials (.env, *.pem, *.key) — run ANY installed program, and run full PowerShell in every workspace. Execution bounds are lifted too: timeouts up to 24 hours and 32 MiB of output per stream. The folder boundary, the Pause kill switch and the audit log stay in force. Only enable this if you fully trust the conversations you use it in.";
const UNRESTRICTED_KEYWORD = "UNRESTRICTED";

interface Props {
  connection: ConnectionStatus | null;
  workspaces: Workspace[];
  settings: AppSettings | null;
  /** Persists settings via api.updateAppSettings; App applies the returned value. */
  onSaveSettings: (settings: AppSettings) => Promise<void>;
  /** Re-pulls connection status after setPaused (App owns shell state). */
  onPauseChanged: () => Promise<void>;
  /** Applies a new ConnectionStatus returned by rotate/reset to App state. */
  onConnectionChanged: (status: ConnectionStatus) => void;
  reportError: (message: string) => void;
}

export default function SecurityPage({
  connection,
  workspaces,
  settings,
  onSaveSettings,
  onPauseChanged,
  onConnectionChanged,
  reportError,
}: Props) {
  const [pending, setPending] = useState(false);
  const [rotateOpen, setRotateOpen] = useState(false);
  const [rotatedEndpoint, setRotatedEndpoint] = useState<string | null>(null);
  const [resetOpen, setResetOpen] = useState(false);
  const [resetText, setResetText] = useState("");
  const [unrestrictedOpen, setUnrestrictedOpen] = useState(false);
  const [unrestrictedText, setUnrestrictedText] = useState("");
  const unrestrictedOn = settings?.unrestricted_access === true;

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

  async function setUnrestricted(next: boolean) {
    if (!settings) return;
    setPending(true);
    try {
      await onSaveSettings({ ...settings, unrestricted_access: next });
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setPending(false);
    }
  }

  function requestUnrestrictedToggle(next: boolean) {
    if (next) {
      setUnrestrictedText("");
      setUnrestrictedOpen(true);
    } else {
      // Disabling the dangerous mode needs no confirmation.
      void setUnrestricted(false);
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
          <p role="alert" className="flex items-start gap-2 text-[13px] text-warning">
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
          <EndpointField
            url={rotatedEndpoint}
            label="New public MCP endpoint"
            hint="Update the endpoint in ChatGPT with this new URL — the old one stopped working."
          />
        ) : null}
        <div>
          <Button variant="secondary" onClick={() => setRotateOpen(true)} disabled={pending}>
            {pending ? <Loader2 aria-hidden="true" className="animate-spin" /> : <KeyRound aria-hidden="true" />}
            Rotate connection URL…
          </Button>
        </div>
      </Card>

      <Card className="gap-3 border-destructive/40 p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <Label htmlFor="unrestricted-mcp" className="text-sm font-semibold">
              Unrestricted access
            </Label>
            <p className="text-[13px] text-muted-foreground">
              Bypass capability gates, the sensitive-file deny list, the approved-command list, the
              Full-PowerShell gate and the execution bounds (timeout, output) in every workspace.
              The folder boundary, Pause and the audit log stay in force.
            </p>
          </div>
          <Switch
            id="unrestricted-mcp"
            checked={unrestrictedOn}
            disabled={pending || !settings}
            onCheckedChange={requestUnrestrictedToggle}
          />
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
      {/* Typed-keyword confirm for unrestricted access (enable only). */}
      <AlertDialog
        open={unrestrictedOpen}
        onOpenChange={(o) => {
          if (!o && !pending) setUnrestrictedOpen(false);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle className="flex items-center gap-2">
              <ShieldAlert aria-hidden="true" className="size-5 text-destructive" />
              {UNRESTRICTED_TITLE}
            </AlertDialogTitle>
            <AlertDialogDescription>{UNRESTRICTED_BODY}</AlertDialogDescription>
          </AlertDialogHeader>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="unrestricted-confirm-input">
              Type {UNRESTRICTED_KEYWORD} to confirm
            </Label>
            <Input
              id="unrestricted-confirm-input"
              value={unrestrictedText}
              autoFocus
              disabled={pending}
              onChange={(event) => setUnrestrictedText(event.target.value)}
              placeholder={UNRESTRICTED_KEYWORD}
              spellCheck={false}
            />
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={() => setUnrestrictedOpen(false)} disabled={pending}>
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setUnrestrictedOpen(false);
                void setUnrestricted(true);
              }}
              disabled={pending || unrestrictedText !== UNRESTRICTED_KEYWORD}
              className="bg-destructive text-white hover:bg-destructive/90"
            >
              Enable unrestricted access
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
