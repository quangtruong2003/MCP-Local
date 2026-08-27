import { useEffect, useState } from "react";
import { AlertTriangle, Loader2, ShieldAlert } from "lucide-react";
import { GeneralForm } from "@/components/features/settings/general-form";
import { AppearanceForm } from "@/components/features/settings/appearance-form";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { CopyButton } from "@/components/shared/copy-button";
import { api, parseErrorMessage } from "../api";
import type {
  AgentMode,
  AppSettings,
  BrowserRuntimeStatus,
  CaptureScope,
  ConnectionStatus,
} from "@/lib/types";

const APPROVED_COMMANDS =
  "git, cargo, rustc, npm, npx, node, pnpm, yarn, python, python3, pip, go, dotnet";

const EXECUTION_WARNING =
  "Approved Commands limits which executable ChatGPT may launch, but those tools can still execute project code such as npm scripts, Cargo build scripts, or Git hooks. Enable Execute only for trusted workspaces.";

const UNRESTRICTED_TITLE = "Enable unrestricted execution?";
const UNRESTRICTED_BODY =
  "Every workspace loses its guards: ANY file becomes readable and editable (including .env, *.pem, *.key), ANY installed program can run, full PowerShell is always available, and execution bounds are lifted (timeouts up to 24 hours, 32 MiB of output per stream). The folder boundary, the Pause kill switch and the audit log stay in force. Only enable this if you fully trust the conversations you use it in.";
const UNRESTRICTED_KEYWORD = "UNRESTRICTED";

const RESET_DATA_TITLE = "Reset browser data?";
const RESET_DATA_BODY =
  "This stops any running automation pages, then removes the browser profile, cookies, cache and artifacts stored inside the managed Local Workspaces Linux runtime. The runtime itself stays installed. Your personal browser and data are not touched.";
const REMOVE_RUNTIME_TITLE = "Remove the browser runtime?";
const REMOVE_RUNTIME_BODY =
  "This stops any running automation pages, unregisters the managed Local Workspaces Linux runtime from WSL, and deletes its app-owned files. Browser Automation stays switched on; the runtime will be set up again the next time a browser session starts.";

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

type AgentSettingsPatch = Partial<
  Pick<AppSettings, "agent_mode" | "screen_capture_enabled" | "capture_scope" | "visual_verification">
>;

export default function SettingsPage({ settings, connection, onSaveSettings, reportError }: Props) {
  const [hostname, setHostname] = useState(settings?.tailscale_hostname ?? "");
  const [stateDir, setStateDir] = useState<string | null>(null);
  const [copiedDiag, setCopiedDiag] = useState(false);
  const [savingHostname, setSavingHostname] = useState(false);
  const [unrestrictedOpen, setUnrestrictedOpen] = useState(false);
  const [unrestrictedText, setUnrestrictedText] = useState("");
  const unrestrictedOn = settings?.unrestricted_access === true;

  function requestUnrestrictedToggle(next: boolean) {
    if (next) {
      setUnrestrictedText("");
      setUnrestrictedOpen(true);
    } else {
      // Disabling the dangerous mode needs no confirmation.
      void setUnrestricted(false);
    }
  }

  async function setUnrestricted(next: boolean) {
    if (!settings) return;
    try {
      await onSaveSettings({ ...settings, unrestricted_access: next });
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }

  async function updateAgentSettings(patch: AgentSettingsPatch) {
    if (!settings) return;
    try {
      await onSaveSettings({ ...settings, ...patch });
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }

  const [browserStatus, setBrowserStatus] = useState<BrowserRuntimeStatus | null>(null);
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const [resetDataOpen, setResetDataOpen] = useState(false);
  const [resettingData, setResettingData] = useState(false);
  const [removeRuntimeOpen, setRemoveRuntimeOpen] = useState(false);
  const [removingRuntime, setRemovingRuntime] = useState(false);

  async function updateBrowserSettings(
    patch: Partial<Pick<AppSettings, "browser_automation_enabled">>,
  ) {
    if (!settings) return;
    try {
      await onSaveSettings({ ...settings, ...patch });
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }

  async function runRuntimeAction(action: () => Promise<BrowserRuntimeStatus>) {
    try {
      setBrowserStatus(await action());
    } catch (err) {
      reportError(parseErrorMessage(err));
      // Refresh status so the card reflects the resulting provision state.
      api
        .browserRuntimeStatus()
        .then(setBrowserStatus)
        .catch(() => {});
    }
  }

  async function setupOrRepairRuntime() {
    if (runtimeBusy) return;
    setRuntimeBusy(true);
    try {
      await runRuntimeAction(
        browserStatus?.distro_ready ? api.repairBrowserRuntime : api.setupBrowserRuntime,
      );
    } finally {
      setRuntimeBusy(false);
    }
  }

  async function confirmResetData() {
    if (resettingData) return;
    setResettingData(true);
    try {
      await runRuntimeAction(api.resetBrowserData);
      setResetDataOpen(false);
    } finally {
      setResettingData(false);
    }
  }

  async function confirmRemoveRuntime() {
    if (removingRuntime) return;
    setRemovingRuntime(true);
    try {
      await runRuntimeAction(api.removeBrowserRuntime);
      setRemoveRuntimeOpen(false);
    } finally {
      setRemovingRuntime(false);
    }
  }

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

  // Browser runtime diagnostics: availability flags + bundled versions only.
  useEffect(() => {
    let disposed = false;
    api
      .browserRuntimeStatus()
      .then((status) => {
        if (!disposed) setBrowserStatus(status);
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

  // Runtime provision label from redacted diagnostics; `null` while loading.
  function runtimeStateLabel(status: BrowserRuntimeStatus): string {
    switch (status.runtime_state) {
      case "ready":
        return status.running ? "Ready · running" : "Ready";
      case "checking_host":
        return "Checking…";
      case "wsl_install_required":
        return "WSL install required";
      case "installing_wsl":
        return "Installing WSL…";
      case "reboot_required":
        return "Windows restart required";
      case "distro_install_required":
        return "Linux runtime install required";
      case "installing_distro":
        return "Installing Linux runtime…";
      case "lightpanda_install_required":
        return "Lightpanda install required";
      case "installing_lightpanda":
        return "Installing Lightpanda…";
      case "verifying_runtime":
        return "Verifying runtime…";
      case "repair_required":
        return "Repair required";
      case "unsupported_host":
        return "Unsupported host";
      default:
        return "Setup required";
    }
  }

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
          <h2 className="text-sm font-semibold">Agent</h2>
          <p className="text-[13px] text-muted-foreground">
            Autonomy and visual verification behavior for connected agent workflows.
          </p>
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="setting-agent-mode">Agent mode</Label>
            <Select
              value={settings?.agent_mode ?? "autonomous"}
              disabled={!settings}
              onValueChange={(value) =>
                void updateAgentSettings({ agent_mode: value as AgentMode })
              }
            >
              <SelectTrigger id="setting-agent-mode" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="autonomous">Autonomous</SelectItem>
                <SelectItem value="conservative">Conservative</SelectItem>
              </SelectContent>
            </Select>
            <p className="text-[13px] text-muted-foreground">
              Autonomous proceeds from clear intent; Conservative asks before materially
              consequential choices that intent does not resolve.
            </p>
          </div>

          <div className="flex items-start justify-between gap-4 rounded-md border p-3">
            <div>
              <Label htmlFor="setting-screen-capture">Screen capture</Label>
              <p className="mt-1 text-[13px] text-muted-foreground">
                Allow single-shot screenshots for model vision when they are relevant.
              </p>
            </div>
            <Switch
              id="setting-screen-capture"
              checked={settings?.screen_capture_enabled ?? false}
              disabled={!settings}
              onCheckedChange={(checked) =>
                void updateAgentSettings({ screen_capture_enabled: checked })
              }
              aria-label="Screen capture"
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="setting-capture-scope">Capture scope</Label>
            <Select
              value={settings?.capture_scope ?? "windows_and_displays"}
              disabled={!settings || !settings.screen_capture_enabled}
              onValueChange={(value) =>
                void updateAgentSettings({ capture_scope: value as CaptureScope })
              }
            >
              <SelectTrigger id="setting-capture-scope" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="windows_and_displays">Windows and displays</SelectItem>
                <SelectItem value="windows_only">Windows only</SelectItem>
              </SelectContent>
            </Select>
            <p className="text-[13px] text-muted-foreground">
              Windows only is the narrower privacy option and excludes full-display targets.
            </p>
          </div>

          <div className="flex items-start justify-between gap-4 rounded-md border p-3">
            <div>
              <Label htmlFor="setting-visual-verification">Visual verification</Label>
              <p className="mt-1 text-[13px] text-muted-foreground">
                Prefer visual evidence when it materially helps verify UI or rendering work.
              </p>
            </div>
            <Switch
              id="setting-visual-verification"
              checked={settings?.visual_verification ?? true}
              disabled={!settings}
              onCheckedChange={(checked) =>
                void updateAgentSettings({ visual_verification: checked })
              }
              aria-label="Visual verification"
            />
          </div>
        </div>
        <p role="note" className="flex items-start gap-1.5 text-[13px] text-warning">
          <AlertTriangle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
          Screen capture can expose content outside approved workspace folders. It is off by
          default, unrestricted execution does not enable it, and Pause always blocks capture.
        </p>
      </Card>

      <Card className="gap-4 p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h2 className="text-sm font-semibold">Browser Automation</h2>
            <p className="text-[13px] text-muted-foreground">
              Background browser automation for agents, running on Lightpanda inside a managed
              WSL2 runtime.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-[13px] text-muted-foreground">
              {settings?.browser_automation_enabled ? "On" : "Off"}
            </span>
            <Switch
              id="setting-browser-automation"
              checked={settings?.browser_automation_enabled ?? false}
              disabled={!settings}
              onCheckedChange={(checked) =>
                void updateBrowserSettings({ browser_automation_enabled: checked })
              }
              aria-label="Browser Automation"
            />
          </div>
        </div>
        <ul className="flex flex-col divide-y">
          <li className="flex items-center justify-between gap-3 py-2 text-sm first:pt-0">
            <span>Backend</span>
            <strong className="text-right font-medium">Lightpanda (WSL2)</strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm">
            <span>Mode</span>
            <strong className="text-right font-medium">Background</strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm">
            <span>Runtime status</span>
            <strong className="text-right font-medium">
              {browserStatus ? runtimeStateLabel(browserStatus) : "Checking…"}
            </strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm last:pb-0">
            <span>Lightpanda</span>
            <strong className="text-right font-medium">
              {browserStatus?.lightpanda_version ?? "—"}
            </strong>
          </li>
        </ul>
        <div className="rounded-md border p-3">
          <Label>Capabilities</Label>
          <ul className="mt-1 flex flex-col gap-1 text-[13px] text-muted-foreground">
            <li>Semantic browser automation over a private native MCP channel</li>
            <li>No visual rendering or screenshots</li>
            <li>No file upload, managed downloads, dialogs, or device emulation</li>
          </ul>
        </div>
        {browserStatus?.reboot_required ? (
          <p role="alert" className="flex items-start gap-1.5 text-[13px] text-warning">
            <AlertTriangle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
            Windows Subsystem for Linux was installed. Restart Windows, then return here and
            choose Set up runtime again to finish.
          </p>
        ) : null}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="secondary"
            disabled={!settings || runtimeBusy || !browserStatus}
            onClick={() => void setupOrRepairRuntime()}
          >
            {runtimeBusy ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
            {browserStatus?.distro_ready ? "Repair runtime" : "Set up runtime"}
          </Button>
          <Button
            variant="secondary"
            disabled={!settings || runtimeBusy || resettingData || !browserStatus?.distro_ready}
            onClick={() => setResetDataOpen(true)}
          >
            Reset browser data…
          </Button>
          <Button
            variant="secondary"
            disabled={
              !settings || runtimeBusy || removingRuntime || !browserStatus?.distro_ready
            }
            onClick={() => setRemoveRuntimeOpen(true)}
          >
            Remove browser runtime…
          </Button>
        </div>
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
            <p className="text-[13px] text-warning" role="alert">
              Hostname must be non-empty, without spaces.
            </p>
          ) : null}
          {hostnameChanged && hostnameValid && identityExists ? (
            <p role="alert" className="flex items-start gap-1.5 text-[13px] text-warning">
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

      <Card className={unrestrictedOn ? "gap-4 border-destructive/40 p-6" : "gap-4 p-6"}>
        <div className="flex items-center justify-between gap-4">
          <div>
            <h2 className="text-sm font-semibold">Execution</h2>
            <p className="text-[13px] text-muted-foreground">
              What ChatGPT may run inside approved workspaces.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-[13px] text-muted-foreground">Full access</span>
            <Switch
              id="setting-unrestricted-execution"
              checked={unrestrictedOn}
              disabled={!settings}
              onCheckedChange={requestUnrestrictedToggle}
              aria-label="Unrestricted execution"
            />
          </div>
        </div>
        <ul className="flex flex-col divide-y">
          <li className="flex items-center justify-between gap-3 py-2 text-sm first:pt-0">
            <span>Approved Commands</span>
            <strong
              className={
                unrestrictedOn
                  ? "text-right font-medium text-destructive"
                  : "text-right font-medium"
              }
            >
              {unrestrictedOn ? "All programs allowed" : APPROVED_COMMANDS}
            </strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm">
            <span>Default timeout</span>
            <strong className="font-medium">60 seconds</strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm">
            <span>Maximum timeout</span>
            <strong
              className={
                unrestrictedOn
                  ? "font-medium text-destructive"
                  : "font-medium"
              }
            >
              {unrestrictedOn ? "24 hours" : "10 minutes"}
            </strong>
          </li>
          <li className="flex items-center justify-between gap-3 py-2 text-sm last:pb-0">
            <span>Output limit</span>
            <strong
              className={
                unrestrictedOn
                  ? "font-medium text-destructive"
                  : "font-medium"
              }
            >
              {unrestrictedOn
                ? "32 MiB per stdout/stderr stream"
                : "512 KiB per stdout/stderr stream"}
            </strong>
          </li>
        </ul>
        {unrestrictedOn ? (
          <p role="alert" className="flex items-start gap-1.5 text-[13px] text-destructive">
            <ShieldAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
            Unrestricted execution is ON — the guards above are bypassed in every workspace. The
            red banner across the app and the audit trail mark everything done in this mode.
          </p>
        ) : (
          <p role="note" className="flex items-start gap-1.5 text-[13px] text-warning">
            <AlertTriangle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
            {EXECUTION_WARNING}
          </p>
        )}
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

      {/* Typed-keyword confirm for unrestricted execution (enable only). */}
      <AlertDialog
        open={unrestrictedOpen}
        onOpenChange={(o) => {
          if (!o) setUnrestrictedOpen(false);
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
            <Label htmlFor="unrestricted-execution-input">
              Type {UNRESTRICTED_KEYWORD} to confirm
            </Label>
            <Input
              id="unrestricted-execution-input"
              value={unrestrictedText}
              autoFocus
              onChange={(event) => setUnrestrictedText(event.target.value)}
              placeholder={UNRESTRICTED_KEYWORD}
              spellCheck={false}
            />
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={() => setUnrestrictedOpen(false)}>
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setUnrestrictedOpen(false);
                void setUnrestricted(true);
              }}
              disabled={unrestrictedText !== UNRESTRICTED_KEYWORD}
              className="bg-destructive text-white hover:bg-destructive/90"
            >
              Enable unrestricted access
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Confirm before wiping browser data inside the managed runtime. */}
      <AlertDialog
        open={resetDataOpen}
        onOpenChange={(o) => {
          if (!o && !resettingData) setResetDataOpen(false);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{RESET_DATA_TITLE}</AlertDialogTitle>
            <AlertDialogDescription>{RESET_DATA_BODY}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={resettingData} onClick={() => setResetDataOpen(false)}>
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={(event) => {
                event.preventDefault();
                void confirmResetData();
              }}
              disabled={resettingData}
              className="bg-destructive text-white hover:bg-destructive/90"
            >
              {resettingData ? "Resetting…" : "Reset browser data"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Confirm before unregistering the managed WSL runtime. */}
      <AlertDialog
        open={removeRuntimeOpen}
        onOpenChange={(o) => {
          if (!o && !removingRuntime) setRemoveRuntimeOpen(false);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{REMOVE_RUNTIME_TITLE}</AlertDialogTitle>
            <AlertDialogDescription>{REMOVE_RUNTIME_BODY}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              disabled={removingRuntime}
              onClick={() => setRemoveRuntimeOpen(false)}
            >
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={(event) => {
                event.preventDefault();
                void confirmRemoveRuntime();
              }}
              disabled={removingRuntime}
              className="bg-destructive text-white hover:bg-destructive/90"
            >
              {removingRuntime ? "Removing…" : "Remove browser runtime"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
