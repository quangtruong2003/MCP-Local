import { useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  CircleSlash,
  Copy,
  LifeBuoy,
  Loader2,
  XOctagon,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { ConnectionState } from "../types";

export type ConnectionTone = "ready" | "offline" | "warning" | "error";

export interface ConnectionPresentation {
  label: string;
  copy: string;
  tone: ConnectionTone;
  /** Backend is mid-transition; all actions must be inert. */
  inProgress: boolean;
  primary: { key: "start" | "retry" | "signin" | "stop" | "wait"; text: string };
  secondary: { key: "stop" | "help"; text: string } | null;
}

/**
 * Single source of truth mapping a ConnectionState to everything the card
 * renders (status label, body copy, severity tone, available actions).
 */
export function presentConnection(state: ConnectionState): ConnectionPresentation {
  switch (state.state) {
    case "stopped":
      return {
        label: "Offline",
        copy: "Your local MCP connection is offline.",
        tone: "offline",
        inProgress: false,
        primary: { key: "start", text: "Start" },
        secondary: null,
      };
    case "starting_server":
      return {
        label: "Starting",
        copy: "Starting local MCP server…",
        tone: "warning",
        inProgress: true,
        primary: { key: "wait", text: "Starting…" },
        secondary: null,
      };
    case "starting_tunnel":
      return {
        label: "Connecting",
        copy: "Starting secure Tailscale connection…",
        tone: "warning",
        inProgress: true,
        primary: { key: "wait", text: "Connecting…" },
        secondary: null,
      };
    case "verifying":
      return {
        label: "Verifying",
        copy: "Verifying the public MCP endpoint…",
        tone: "warning",
        inProgress: true,
        primary: { key: "wait", text: "Verifying…" },
        secondary: null,
      };
    case "stopping":
      return {
        label: "Stopping",
        copy: "Closing public access…",
        tone: "warning",
        inProgress: true,
        primary: { key: "wait", text: "Stopping…" },
        secondary: null,
      };
    case "needs_login":
      return {
        label: "Sign-in required",
        copy: "Sign in to Tailscale to connect this app.",
        tone: "warning",
        inProgress: false,
        primary: { key: "signin", text: "Sign in to Tailscale" },
        secondary: { key: "stop", text: "Stop" },
      };
    case "needs_funnel_permission":
      return {
        label: "Setup required",
        copy: "Funnel needs one-time approval for this tailnet.",
        tone: "warning",
        inProgress: false,
        primary: { key: "retry", text: "Retry" },
        secondary: { key: "help", text: "Setup help" },
      };
    case "needs_https_setup":
      return {
        label: "Setup required",
        copy: "Funnel needs HTTPS certificates enabled for your tailnet.",
        tone: "warning",
        inProgress: false,
        primary: { key: "retry", text: "Retry" },
        secondary: { key: "help", text: "Setup help" },
      };
    case "online":
      return {
        label: "Online",
        copy: "Ready for ChatGPT.",
        tone: "ready",
        inProgress: false,
        primary: { key: "stop", text: "Stop" },
        secondary: null,
      };
    case "degraded":
      return {
        label: "Degraded",
        copy: "Connection started, but end-to-end verification failed.",
        tone: "warning",
        inProgress: false,
        primary: { key: "retry", text: "Retry" },
        secondary: null,
      };
    case "error":
      return {
        label: "Error",
        copy: state.message,
        tone: "error",
        inProgress: false,
        primary: { key: "retry", text: "Retry" },
        secondary: null,
      };
  }
}

const TONE_ICONS: Record<ConnectionTone, LucideIcon> = {
  ready: CheckCircle2,
  offline: CircleSlash,
  warning: AlertTriangle,
  error: XOctagon,
};

const ENDPOINT_WARNING =
  "Anyone with this URL may attempt to use this MCP connection. Keep it private.";

interface Props {
  state: ConnectionState;
  /** True while a lifecycle request (start/stop/retry) is in flight. */
  pending: boolean;
  onStart: () => void;
  onStop: () => void;
  onRetry: () => void;
  onSignIn: () => void;
  /** Opens a Tailscale admin console page in the system browser. */
  onOpenAdmin: (url: string) => void;
  /** Surface non-lifecycle failures (e.g. clipboard denial) as toasts. */
  onError: (message: string) => void;
}

export default function ConnectionCard({
  state,
  pending,
  onStart,
  onStop,
  onRetry,
  onSignIn,
  onOpenAdmin,
  onError,
}: Props) {
  const p = presentConnection(state);
  const [helpVisible, setHelpVisible] = useState(false);
  const [copied, setCopied] = useState(false);
  const endpoint = state.state === "online" ? state.endpoint : null;

  const busy = pending || p.inProgress;

  function runPrimary() {
    switch (p.primary.key) {
      case "start":
        onStart();
        break;
      case "retry":
        onRetry();
        break;
      case "signin":
        onSignIn();
        break;
      case "stop":
        onStop();
        break;
      case "wait":
        break;
    }
  }

  async function copyEndpoint() {
    if (!endpoint) return;
    try {
      await navigator.clipboard.writeText(endpoint);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      onError("Could not access the clipboard to copy the endpoint URL.");
    }
  }

  const ToneIcon = p.inProgress ? Loader2 : TONE_ICONS[p.tone];

  return (
    <section className="conn-card">
      <div className="conn-card-top">
        <span className={`conn-state conn-state-${p.tone}`}>
          <ToneIcon aria-hidden="true" className={p.inProgress ? "spinner" : undefined} />
          {p.label}
        </span>
      </div>
      <p className="conn-copy" aria-live="polite">
        {p.copy}
      </p>

      <div className="conn-actions">
        <button
          className="btn btn-primary"
          onClick={runPrimary}
          disabled={busy || p.primary.key === "wait"}
        >
          {p.primary.key === "wait" ? (
            <Loader2 aria-hidden="true" className="spinner" />
          ) : null}
          {p.primary.text}
        </button>
        {p.secondary ? (
          p.secondary.key === "help" ? (
            <button
              className="btn btn-secondary"
              onClick={() => setHelpVisible((visible) => !visible)}
              aria-expanded={helpVisible}
              disabled={busy}
            >
              <LifeBuoy aria-hidden="true" />
              {p.secondary.text}
            </button>
          ) : (
            <button className="btn btn-danger" onClick={onStop} disabled={busy}>
              {p.secondary.text}
            </button>
          )
        ) : null}
      </div>

      {helpVisible ? (
        <div className="setup-help">
          {state.state === "needs_https_setup" ? (
            <>
              <p>
                Publishing over Funnel requires HTTPS certificates, which are currently disabled
                for your tailnet. A tailnet admin can enable them once in the Tailscale admin
                console. Once enabled, press Retry.
              </p>
              <button
                className="link-button"
                onClick={() => onOpenAdmin("https://login.tailscale.com/admin/settings/dns")}
              >
                Open HTTPS settings →
              </button>
            </>
          ) : (
            <>
              <p>
                Funnel publishing must be approved once for your tailnet. A tailnet admin can
                enable it in the Tailscale admin console under Access controls (add the funnel
                node attribute). Once approved, press Retry.
              </p>
              <button
                className="link-button"
                onClick={() => onOpenAdmin("https://login.tailscale.com/admin/acls")}
              >
                Open access controls →
              </button>
            </>
          )}
        </div>
      ) : null}

      {endpoint ? (
        <div className="endpoint-panel">
          <label htmlFor="public-endpoint">Public MCP endpoint</label>
          <div className="endpoint-row">
            <input id="public-endpoint" type="text" readOnly value={endpoint} />
            <button className="btn btn-secondary" onClick={() => void copyEndpoint()}>
              <Copy aria-hidden="true" />
              {copied ? "Copied" : "Copy"}
            </button>
          </div>
          <p className="endpoint-warning">
            <AlertTriangle aria-hidden="true" />
            {ENDPOINT_WARNING}
          </p>
        </div>
      ) : null}
    </section>
  );
}
