import { useEffect, useState } from "react";
import { AlertTriangle, Copy } from "lucide-react";
import { api, parseErrorMessage } from "../api";
import type { AppSettings, ConnectionStatus } from "@/lib/types";

/**
 * Settings page (plan Task 21): general auto-start toggle, connection
 * settings with a hostname-change reset warning, read-only execution bounds,
 * and a redacted "Copy diagnostics" action.
 */

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
  const [autoStart, setAutoStart] = useState(settings?.start_on_launch ?? false);
  const [hostname, setHostname] = useState(settings?.tailscale_hostname ?? "");
  const [stateDir, setStateDir] = useState<string | null>(null);
  const [copiedPath, setCopiedPath] = useState(false);
  const [copiedDiag, setCopiedDiag] = useState(false);
  const [savingHostname, setSavingHostname] = useState(false);

  // Re-seed when App loads/replaces settings.
  useEffect(() => {
    if (!settings) return;
    setAutoStart(settings.start_on_launch);
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
  const hostnameChanged = hostname !== (settings?.tailscale_hostname ?? "");

  async function toggleAutoStart() {
    const previous = autoStart;
    const next = !previous;
    setAutoStart(next);
    if (!settings) return;
    try {
      await onSaveSettings({ ...settings, start_on_launch: next });
    } catch (err) {
      reportError(parseErrorMessage(err));
      setAutoStart(previous); // roll back on failure
    }
  }

  async function saveHostname() {
    if (!settings || !hostnameChanged) return;
    setSavingHostname(true);
    try {
      await onSaveSettings({ ...settings, tailscale_hostname: hostname });
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setSavingHostname(false);
    }
  }

  async function copyStateDir() {
    if (!stateDir) return;
    try {
      await navigator.clipboard.writeText(stateDir);
      setCopiedPath(true);
      window.setTimeout(() => setCopiedPath(false), 1600);
    } catch {
      reportError("Could not access the clipboard to copy the path.");
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
    <div className="settings-page">
      <section className="sec-card">
        <div className="ws-toolbar-copy">
          <h2>General</h2>
          <p>App launch behavior.</p>
        </div>
        <label className="permission-toggle">
          <span className="permission-copy">
            <strong>Start connection automatically on app launch</strong>
          </span>
          <input type="checkbox" checked={autoStart} onChange={() => void toggleAutoStart()} />
        </label>
      </section>

      <section className="sec-card">
        <div className="ws-toolbar-copy">
          <h2>Connection</h2>
          <p>Tailscale Funnel identity and state.</p>
        </div>
        <div className="settings-field">
          <label htmlFor="settings-hostname">Hostname</label>
          <div className="endpoint-row">
            <input
              id="settings-hostname"
              type="text"
              value={hostname}
              onChange={(event) => setHostname(event.target.value)}
              spellCheck={false}
            />
            <button
              className="btn btn-secondary"
              onClick={() => void saveHostname()}
              disabled={!hostnameChanged || savingHostname}
            >
              Save
            </button>
          </div>
          {hostnameChanged && identityExists ? (
            <p className="endpoint-warning" role="alert">
              <AlertTriangle aria-hidden="true" />
              Changing the hostname requires a Tailscale reset and re-enrollment; your *.ts.net name
              may change. Use “Reset Tailscale identity…” on the Security page afterwards.
            </p>
          ) : null}
        </div>
        <ul className="protection-list">
          <li className="protection-row">
            <span>Current Tailscale FQDN</span>
            <strong>{fqdn ?? "—"}</strong>
          </li>
          <li className="protection-row">
            <span>Tailscale state location</span>
            <strong>{stateDir ? elide(stateDir) : "—"}</strong>
          </li>
        </ul>
        {stateDir ? (
          <div className="conn-actions">
            <button className="btn btn-secondary" onClick={() => void copyStateDir()}>
              <Copy aria-hidden="true" />
              {copiedPath ? "Copied" : "Copy full path"}
            </button>
          </div>
        ) : null}
      </section>

      <section className="sec-card">
        <div className="ws-toolbar-copy">
          <h2>Execution</h2>
          <p>What ChatGPT may run inside approved workspaces.</p>
        </div>
        <ul className="protection-list">
          <li className="protection-row settings-exec-row">
            <span>Approved Commands</span>
            <strong>{APPROVED_COMMANDS}</strong>
          </li>
          <li className="protection-row">
            <span>Default timeout</span>
            <strong>60 seconds</strong>
          </li>
          <li className="protection-row">
            <span>Maximum timeout</span>
            <strong>10 minutes</strong>
          </li>
          <li className="protection-row">
            <span>Output limit</span>
            <strong>512 KiB per stdout/stderr stream</strong>
          </li>
        </ul>
        <p className="security-callout" role="note">
          <AlertTriangle aria-hidden="true" />
          {EXECUTION_WARNING}
        </p>
      </section>

      <section className="sec-card">
        <div className="ws-toolbar-copy">
          <h2>Diagnostics</h2>
          <p>Copy a redacted report for troubleshooting. It never contains secrets.</p>
        </div>
        <div className="conn-actions">
          <button className="btn btn-secondary" onClick={() => void copyDiagnostics()}>
            <Copy aria-hidden="true" />
            {copiedDiag ? "Copied" : "Copy diagnostics"}
          </button>
        </div>
      </section>
    </div>
  );
}
