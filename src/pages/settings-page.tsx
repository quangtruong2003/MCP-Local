import { useEffect, useState } from "react";
import { AlertTriangle, Loader2 } from "lucide-react";
import { GeneralForm } from "@/components/features/settings/general-form";
import { AppearanceForm } from "@/components/features/settings/appearance-form";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { CopyButton } from "@/components/shared/copy-button";
import { api, parseErrorMessage } from "../api";
import type { AppSettings, ConnectionStatus } from "@/lib/types";

const APPROVED_COMMANDS =
  "git, cargo, rustc, npm, npx, node, pnpm, yarn, python, python3, pip, go, dotnet";

const EXECUTION_WARNING =
  "Approved Commands limits which executable ChatGPT may launch, but those tools can still execute project code such as npm scripts, Cargo build scripts, or Git hooks. Enable Execute only for trusted workspaces.";

/** Middle-truncates to roughly `max` characters for display. */
function elide(text: string, max = 40): string {
  if (text.length <= max) return text;
  const half = Math.floor((max - 1) / 2);
  return `${text.slice(0, half)}…${text.slice(text.length - half)}`;
}

interface Props {
  settings: AppSettings | null;
  connection: ConnectionStatus | null;
  /** Persists settings via api.updateAppSettings; App applies the returned value. */
  onSaveSettings: (settings: AppSettings) => Promise<void>;
  reportError: (message: string) => void;
}

export default function SettingsPage({ settings, connection, onSaveSettings, reportError }: Props) {
  const [hostname, setHostname] = useState(settings?.tailscale_hostname ?? "");
  const [stateDir, setStateDir] = useState<string | null>(null);
  const [copiedDiag, setCopiedDiag] = useState(false);
  const [savingHostname, setSavingHostname] = useState(false);

  // Re-seed when App loads/replaces settings.
  useEffect(() => {
    if (!settings) return;
    setHostname(settings.tailscale_hostname);
  }, [settings]);

  useEffect(() => {
    let disposed = false;
    api
      .tailscaleStateDir()
      .then((dir) => {
        if (!disposed) setStateDir(dir);
      })
      .catch((err) => reportError(parseErrorMessage(err)));
    return () => {
      disposed = true;
    };
  }, [reportError]);

  const identityExists = connection?.state.state !== undefined && connection.state.state !== "stopped";
  const hostnameChanged = hostname.trim() !== (settings?.tailscale_hostname ?? "");
  const hostnameValid = hostname.trim().length > 0 && !/\s/.test(hostname);

  async function saveHostname() {
    if (!settings || !hostnameChanged || !hostnameValid) return;
    setSavingHostname(true);
    try {
      await onSaveSettings({ ...settings, tailscale_hostname: hostname.trim() });
    } catch (err) {
      reportError(
        typeof err === "object" && err !== null && "message" in err
          ? String((err as { message: unknown }).message)
          : String(err),
      );
    } finally {
      setSavingHostname(false);
    }
  }

  async function copyDiagnostics() {
    try {
      const report = await api.connectionDiagnostics();
      await navigator.clipboard.writeText(JSON.stringify(report, null, 2));
      setCopiedDiag(true);
      window.setTimeout(() => setCopiedDiag(false), 1600);
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }

  const fqdn = connection?.state.state === "online" ? connection.state.fqdn : null;

  return (
    <div className="flex flex-col gap-4">
      <Card className="gap-4 p-6">
        <div>
          <h2 className="text-sm font-semibold">General</h2>
          <p className="text-[13px] text-muted-foreground">App launch and window behavior.</p>
        </div>
        <GeneralForm settings={settings} onSaveSettings={onSaveSettings} reportError={reportError} />
      </Card>

      <Card className="gap-4 p-6">
        <div>
          <h2 className="text-sm font-semibold">Appearance</h2>
          <p className="text-[13px] text-muted-foreground">Light, dark or follow the system.</p>
        </div>
        <AppearanceForm />
      </Card>

      <Card className="gap-4 p-6">
        <div>
          <h2 className="text-sm font-semibold">Connection</h2>
          <p className="text-[13px] text-muted-foreground">Tailscale Funnel identity and state.</p>
        </div>
        <div className="flex flex-col gap-1.5">
          <label htmlFor="settings-hostname" className="text-[13px] font-medium">
            Hostname
          </label>
          <div className="flex items-center gap-2">
            <Input
              id="settings-hostname"
              value={hostname}
              onChange={(event) => setHostname(event.target.value)}
              spellCheck={false}
              className="font-mono"
              aria-invalid={!hostnameValid}
            />
            <Button
              variant="secondary"
              onClick={() => void saveHostname()}
              disabled={!hostnameChanged || !hostnameValid || savingHostname}
            >
              {savingHostname ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
              Save
            </Button>
          </div>
          {!hostnameValid ? (
            <p className="text-[13px] text-warning-foreground" role="alert">
              Hostname must be non-empty, without spaces.
            </p>
          ) : null}
          {hostnameChanged && hostnameValid && identityExists ? (
            <p role="alert" className="flex items-start gap-1.5 text-[13px] text-warning-foreground">
              <AlertTriangle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-warning" />
              Changing the hostname requires a Tailscale reset and re-enrollment; your *.ts.net name
              may change. Use “Reset Tailscale identity…” on the Security page afterwards.
            </p>
          ) : null}
        </div>
        <ul className="flex flex-col divide-y">
          <li className="flex items-center justify-between gap-3 py-2 text-sm first:pt-0">
            <span>Current Tailscale FQDN</span>
            <strong className="text-right font-medium">{fqdn ?? "—"}</strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm last:pb-0">
            <span>Tailscale state location</span>
            <strong className="text-right font-medium">{stateDir ? elide(stateDir) : "—"}</strong>
          </li>
        </ul>
        {stateDir ? <CopyButton value={stateDir} label="Copy full path" /> : null}
      </Card>

      <Card className="gap-4 p-6">
        <div>
          <h2 className="text-sm font-semibold">Execution</h2>
          <p className="text-[13px] text-muted-foreground">
            What ChatGPT may run inside approved workspaces.
          </p>
        </div>
        <ul className="flex flex-col divide-y">
          <li className="flex items-center justify-between gap-3 py-2 text-sm first:pt-0">
            <span>Approved Commands</span>
            <strong className="text-right font-medium">{APPROVED_COMMANDS}</strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm">
            <span>Default timeout</span>
            <strong className="font-medium">60 seconds</strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm">
            <span>Maximum timeout</span>
            <strong className="font-medium">10 minutes</strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm last:pb-0">
            <span>Output limit</span>
            <strong className="font-medium">512 KiB per stdout/stderr stream</strong>
          </li>
        </ul>
        <p role="note" className="flex items-start gap-1.5 text-[13px] text-warning-foreground">
          <AlertTriangle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-warning" />
          {EXECUTION_WARNING}
        </p>
      </Card>

      <Card className="gap-4 p-6">
        <div>
          <h2 className="text-sm font-semibold">Diagnostics</h2>
          <p className="text-[13px] text-muted-foreground">
            Copy a redacted report for troubleshooting. It never contains secrets.
          </p>
        </div>
        <div>
          <Button variant="secondary" onClick={() => void copyDiagnostics()}>
            {copiedDiag ? "Copied" : "Copy diagnostics"}
          </Button>
        </div>
      </Card>
    </div>
  );
}
