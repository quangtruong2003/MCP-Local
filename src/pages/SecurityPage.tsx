import { useState } from "react";
import { Copy, KeyRound, Loader2, Pause, Play, ShieldAlert } from "lucide-react";
import ConfirmDialog from "../components/ConfirmDialog";
import { api, parseErrorMessage } from "../api";
import type { ConnectionStatus, Workspace } from "@/lib/types";

/**
 * Security overview (plan Task 20): protection summary rows derived from the
 * workspace list plus the global pause control, connection URL rotation and
 * the destructive Tailscale identity reset behind typed confirmation.
 * The capability token itself is never rendered; only the endpoint URL,
 * which embeds the token by design and is meant to be pasted into ChatGPT.
 */

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
  const [copied, setCopied] = useState(false);
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

  async function togglePaused() {
    setPending(true);
    try {
      await api.setPaused(!paused);
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
      setCopied(false);
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

  async function copyEndpoint(endpoint: string) {
    try {
      await navigator.clipboard.writeText(endpoint);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      reportError("Could not access the clipboard to copy the endpoint URL.");
    }
  }

  return (
    <div className="security-page">
      <section className="sec-card">
        <div className="ws-toolbar-copy">
          <h2>Protection summary</h2>
          <p>What stands between ChatGPT and your filesystem right now.</p>
        </div>
        <ul className="protection-list">
          {rows.map((row) => (
            <li key={row.label} className="protection-row">
              <span>{row.label}</span>
              <strong>{row.value}</strong>
            </li>
          ))}
        </ul>
        {fullPowershellCount > 0 ? (
          <p className="security-callout" role="alert">
            <ShieldAlert aria-hidden="true" />
            Full PowerShell is enabled for one or more workspaces. Working-directory confinement is
            not a Windows security sandbox; scripts may access other resources available to your
            account.
          </p>
        ) : null}
      </section>

      <section className="sec-card">
        <div className="ws-toolbar-copy">
          <h2>Global access</h2>
          <p>Pause all tool calls without tearing down the connection.</p>
        </div>
        <div className="conn-actions">
          <button className="btn btn-secondary" onClick={() => void togglePaused()} disabled={pending}>
            {pending ? <Loader2 aria-hidden="true" className="spinner" /> : paused ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}
            {paused ? "Resume MCP access" : "Pause MCP access"}
          </button>
          <span className="sec-state-label">{paused ? "Paused" : "Active"}</span>
        </div>
      </section>

      <section className="sec-card">
        <div className="ws-toolbar-copy">
          <h2>Connection URL</h2>
          <p>Issue a fresh capability URL if you think the current one has leaked.</p>
        </div>
        {rotatedEndpoint ? (
          <div className="endpoint-panel">
            <label htmlFor="security-endpoint">New public MCP endpoint</label>
            <div className="endpoint-row">
              <input id="security-endpoint" type="text" readOnly value={rotatedEndpoint} />
              <button className="btn btn-secondary" onClick={() => void copyEndpoint(rotatedEndpoint)}>
                <Copy aria-hidden="true" />
                {copied ? "Copied" : "Copy"}
              </button>
            </div>
          </div>
        ) : null}
        <div className="conn-actions">
          <button className="btn btn-secondary" onClick={() => setRotateOpen(true)} disabled={pending}>
            <KeyRound aria-hidden="true" />
            Rotate connection URL…
          </button>
        </div>
      </section>

      <section className="sec-card sec-card-danger">
        <div className="ws-toolbar-copy">
          <h2>Advanced</h2>
          <p>Last resort: forget this machine's Tailscale identity and re-authenticate.</p>
        </div>
        <div className="conn-actions">
          <button
            className="btn btn-danger"
            onClick={() => {
              setResetText("");
              setResetOpen(true);
            }}
            disabled={pending}
          >
            Reset Tailscale identity…
          </button>
        </div>
      </section>

      <ConfirmDialog
        open={rotateOpen}
        title={ROTATE_TITLE}
        message={ROTATE_BODY}
        tone="warning"
        confirmLabel="Rotate URL"
        onCancel={() => setRotateOpen(false)}
        onConfirm={() => void rotateUrl()}
      />

      {resetOpen ? (
        <div
          className="dialog-backdrop"
          role="presentation"
          onClick={(event) => {
            if (event.target === event.currentTarget && !pending) setResetOpen(false);
          }}
        >
          <div className="dialog-card" role="alertdialog" aria-modal="true" aria-labelledby="reset-dialog-title">
            <div className="dialog-title-row">
              <span className="dialog-icon dialog-icon-danger" aria-hidden="true">
                <ShieldAlert />
              </span>
              <h2 id="reset-dialog-title">{RESET_TITLE}</h2>
            </div>
            <p className="dialog-message">{RESET_BODY}</p>
            <label className="reset-confirm">
              <span>Type {RESET_KEYWORD} to confirm</span>
              <input
                type="text"
                value={resetText}
                autoFocus
                disabled={pending}
                onChange={(event) => setResetText(event.target.value)}
                placeholder={RESET_KEYWORD}
                spellCheck={false}
              />
            </label>
            <div className="dialog-actions">
              <button className="btn btn-secondary" onClick={() => setResetOpen(false)} disabled={pending}>
                Cancel
              </button>
              <button
                className="btn btn-danger"
                onClick={() => void resetIdentity()}
                disabled={pending || resetText !== RESET_KEYWORD}
              >
                {pending ? <Loader2 aria-hidden="true" className="spinner" /> : null}
                Reset
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
