import { useState } from "react";
import { AlertTriangle, ExternalLink, LifeBuoy, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { CopyButton } from "@/components/shared/copy-button";
import { StatusBadge } from "@/components/shared/status-badge";
import type { ConnectionState } from "@/lib/types";

export interface ConnectionPresentation {
  label: string;
  copy: string;
  /** Backend is mid-transition; all actions must be inert. */
  inProgress: boolean;
  primary: { key: "start" | "retry" | "signin" | "stop" | "wait"; text: string };
  secondary: { key: "stop" | "help"; text: string } | null;
}

/**
 * Single source of truth mapping a ConnectionState to everything the card
 * renders (status label, body copy, available actions).
 */
function presentConnection(state: ConnectionState): ConnectionPresentation {
  switch (state.state) {
    case "stopped":
      return {
        label: "Offline",
        copy: "Your local MCP connection is offline.",
        inProgress: false,
        primary: { key: "start", text: "Start" },
        secondary: null,
      };
    case "starting_server":
      return {
        label: "Starting",
        copy: "Starting local MCP server…",
        inProgress: true,
        primary: { key: "wait", text: "Starting…" },
        secondary: null,
      };
    case "starting_tunnel":
      return {
        label: "Connecting",
        copy: "Starting secure Tailscale connection…",
        inProgress: true,
        primary: { key: "wait", text: "Connecting…" },
        secondary: null,
      };
    case "verifying":
      return {
        label: "Verifying",
        copy: "Verifying the public MCP endpoint…",
        inProgress: true,
        primary: { key: "wait", text: "Verifying…" },
        secondary: null,
      };
    case "stopping":
      return {
        label: "Stopping",
        copy: "Closing public access…",
        inProgress: true,
        primary: { key: "wait", text: "Stopping…" },
        secondary: null,
      };
    case "needs_login":
      return {
        label: "Sign-in required",
        copy: "Sign in to Tailscale to connect this app.",
        inProgress: false,
        primary: { key: "signin", text: "Sign in to Tailscale" },
        secondary: { key: "stop", text: "Stop" },
      };
    case "needs_funnel_permission":
      return {
        label: "Setup required",
        copy: "Funnel needs one-time approval for this tailnet.",
        inProgress: false,
        primary: { key: "retry", text: "Retry" },
        secondary: { key: "help", text: "Setup help" },
      };
    case "needs_https_setup":
      return {
        label: "Setup required",
        copy: "Funnel needs HTTPS certificates enabled for your tailnet.",
        inProgress: false,
        primary: { key: "retry", text: "Retry" },
        secondary: { key: "help", text: "Setup help" },
      };
    case "online":
      return {
        label: "Online",
        copy: "Ready for ChatGPT.",
        inProgress: false,
        primary: { key: "stop", text: "Stop" },
        secondary: null,
      };
    case "degraded":
      return {
        label: "Degraded",
        copy: "Connection started, but end-to-end verification failed.",
        inProgress: false,
        primary: { key: "retry", text: "Retry" },
        secondary: null,
      };
    case "error":
      return {
        label: "Error",
        copy: state.message,
        inProgress: false,
        primary: { key: "retry", text: "Retry" },
        secondary: null,
      };
  }
}

const ENDPOINT_WARNING =
  "Anyone with this URL may attempt to use this MCP connection. Keep it private.";

interface Props {
  state: ConnectionState;
  paused?: boolean;
  /** True while a lifecycle request (start/stop/retry) is in flight. */
  pending: boolean;
  onStart: () => void;
  onStop: () => void;
  onRetry: () => void;
  onSignIn: () => void;
  /** Opens a Tailscale admin console page in the system browser. */
  onOpenAdmin: (url: string) => void;
}

export function ConnectionCard({
  state,
  paused = false,
  pending,
  onStart,
  onStop,
  onRetry,
  onSignIn,
  onOpenAdmin,
}: Props) {
  const p = presentConnection(state);
  const [helpVisible, setHelpVisible] = useState(false);
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

  return (
    <Card className="gap-4 p-6">
      <div className="flex items-center justify-between gap-2">
        <StatusBadge state={state} paused={paused} />
        <span className="text-[13px] font-medium text-muted-foreground">{p.label}</span>
      </div>
      <p aria-live="polite" className="text-sm text-muted-foreground">
        {p.copy}
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <Button size="lg" onClick={runPrimary} disabled={busy || p.primary.key === "wait"}>
          {busy ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
          {p.primary.text}
        </Button>
        {p.secondary ? (
          p.secondary.key === "help" ? (
            <Button
              variant="secondary"
              onClick={() => setHelpVisible((visible) => !visible)}
              aria-expanded={helpVisible}
              disabled={busy}
            >
              <LifeBuoy aria-hidden="true" />
              {p.secondary.text}
            </Button>
          ) : (
            <Button variant="outline" onClick={onStop} disabled={busy}>
              {p.secondary.text}
            </Button>
          )
        ) : null}
      </div>

      {helpVisible ? (
        <div className="rounded-md border bg-muted/50 p-4 text-sm">
          {state.state === "needs_https_setup" ? (
            <>
              <p>
                Publishing over Funnel requires HTTPS certificates, which are currently disabled
                for your tailnet. A tailnet admin can enable them once in the Tailscale admin
                console. Once enabled, press Retry.
              </p>
              <Button
                variant="link"
                className="mt-1 h-auto px-0"
                onClick={() => onOpenAdmin("https://login.tailscale.com/admin/settings/dns")}
              >
                Open HTTPS settings →
              </Button>
            </>
          ) : (
            <>
              <p>
                Funnel publishing must be approved once for your tailnet. A tailnet admin can
                enable it in the Tailscale admin console under Access controls (add the funnel
                node attribute). Once approved, press Retry.
              </p>
              <Button
                variant="link"
                className="mt-1 h-auto px-0"
                onClick={() => onOpenAdmin("https://login.tailscale.com/admin/acls")}
              >
                Open access controls →
              </Button>
            </>
          )}
        </div>
      ) : null}

      {endpoint ? (
        <div className="flex flex-col gap-2 rounded-md border bg-muted/50 p-4">
          <label htmlFor="public-endpoint" className="text-[13px] font-medium">
            Public MCP endpoint
          </label>
          <div className="flex items-center gap-2">
            <code
              id="public-endpoint"
              className="min-w-0 flex-1 truncate rounded-sm bg-background px-2 py-1.5 font-mono text-xs"
              title={endpoint}
            >
              {endpoint}
            </code>
            <CopyButton value={endpoint} label="Copy endpoint URL" />
            <Button
              variant="ghost"
              size="icon"
              aria-label="Open endpoint in browser"
              onClick={() => onOpenAdmin(endpoint)}
            >
              <ExternalLink aria-hidden="true" />
            </Button>
          </div>
          <p className="flex items-start gap-1.5 text-[13px] text-warning-foreground">
            <AlertTriangle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-warning" />
            {ENDPOINT_WARNING}
          </p>
        </div>
      ) : null}
    </Card>
  );
}
