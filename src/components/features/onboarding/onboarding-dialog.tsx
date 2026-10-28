import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  CheckCircle2,
  FolderPlus,
  LifeBuoy,
  Loader2,
  ShieldCheck,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import { EndpointField } from "@/components/shared/endpoint-field";
import { api, parseErrorMessage } from "@/api";
import type { ConnectionStatus, Workspace } from "@/lib/types";

/**
 * Guided first-run onboarding:
 * Welcome -> Add workspace -> Review full access -> Start connection,
 * then live connection states take over: NeedsLogin ->
 * NeedsFunnelPermission/HttpsSetup -> Verifying -> Online (Copy endpoint +
 * Finish).
 */

type SetupStep = "welcome" | "add-workspace" | "review-permissions" | "start-connection";

const SETUP_STEPS: SetupStep[] = [
  "welcome",
  "add-workspace",
  "review-permissions",
  "start-connection",
];

const STEP_TITLES: Record<SetupStep, string> = {
  welcome: "Welcome",
  "add-workspace": "Add a workspace",
  "review-permissions": "Review permissions",
  "start-connection": "Start your connection",
};

interface Props {
  connection: ConnectionStatus | null;
  workspaces: Workspace[];
  /** Refreshes the workspace list after a folder is added in-step. */
  onWorkspacesChanged: () => Promise<void>;
  onClose: () => void;
  onFinish: () => void | Promise<void>;
  reportError: (message: string) => void;
}

export default function OnboardingFlow({
  connection,
  workspaces,
  onWorkspacesChanged,
  onClose,
  onFinish,
  reportError,
}: Props) {
  const [stepIndex, setStepIndex] = useState(0);
  const [adding, setAdding] = useState(false);
  const [starting, setStarting] = useState(false);

  async function addWorkspace() {
    setAdding(true);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Choose a workspace folder" });
      if (typeof dir === "string") {
        await api.addWorkspace(dir);
        await onWorkspacesChanged();
      }
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setAdding(false);
    }
  }

  // Drives the backend; the `connection-status` event then moves the flow
  // through NeedsLogin / NeedsFunnelPermission / Online automatically.
  async function startConnection() {
    setStarting(true);
    try {
      await api.startConnection();
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setStarting(false);
    }
  }

  // Retry must tear down the previous run first. ConnectionManager::start()
  // is intentionally idempotent for every non-Stopped state, so calling it
  // from NeedsLogin/NeedsFunnelPermission/NeedsHttpsSetup would otherwise be
  // a no-op after the sidecar has exited or needs a clean restart.
  async function retryConnection() {
    setStarting(true);
    try {
      await api.retryConnection();
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setStarting(false);
    }
  }

  const step = SETUP_STEPS[stepIndex];
  const isLastSetupStep = stepIndex === SETUP_STEPS.length - 1;

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        {/* Live-state stages replace the setup wizard once a run has started. */}
        <LiveStage
          connection={connection}
          starting={starting}
          onRetry={retryConnection}
          onFinish={onFinish}
          reportError={reportError}
        />
        {isLiveStage(connection) ? null : (
          <>
            <DialogHeader>
              <div className="flex items-center gap-2">
                <Badge variant="secondary">
                  Step {stepIndex + 1} of {SETUP_STEPS.length}
                </Badge>
              </div>
              <DialogTitle>{STEP_TITLES[step]}</DialogTitle>
            </DialogHeader>

            {step === "welcome" ? (
              <div className="flex flex-col gap-3 text-sm">
                <p className="text-muted-foreground">
                  This app gives ChatGPT safe access to folders you approve, published over
                  Tailscale Funnel and protected by a secret capability URL. Setup takes about two
                  minutes.
                </p>
                <ul className="flex flex-col gap-1.5">
                  {[
                    "Approve which folders the assistant may reach",
                    "Sign in to Tailscale once",
                    "Get a secret capability URL for ChatGPT — keep it private",
                  ].map((point) => (
                    <li key={point} className="flex items-start gap-2">
                      <CheckCircle2 aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-success" />
                      {point}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}

            {step === "add-workspace" ? (
              <div className="flex flex-col gap-3 text-sm">
                <p className="text-muted-foreground">
                  Pick at least one folder ChatGPT may use. File tools stay within workspace
                  boundaries, but Full PowerShell scripts can reach other locations allowed by your
                  Windows account.
                </p>
                {workspaces.length > 0 ? (
                  <ul className="flex flex-col gap-1" aria-label="Added workspaces">
                    {workspaces.map((ws) => (
                      <li key={ws.id} className="flex items-center gap-2 text-[13px]">
                        <FolderPlus aria-hidden="true" className="size-4 text-muted-foreground" />
                        {ws.name}
                      </li>
                    ))}
                  </ul>
                ) : null}
                <div>
                  <Button variant="secondary" onClick={() => void addWorkspace()} disabled={adding}>
                    {adding ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
                    Choose folder…
                  </Button>
                </div>
              </div>
            ) : null}

            {step === "review-permissions" ? (
              <div className="flex flex-col gap-3 text-sm">
                <p className="text-muted-foreground">Every new folder starts with full access:</p>
                <ul className="flex flex-col gap-2">
                  <li className="flex items-start gap-2">
                    <ShieldCheck aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-primary" />
                    <div>
                      <strong>Read — on</strong>
                      <p className="text-muted-foreground">
                        ChatGPT can list and read files inside the folder.
                      </p>
                    </div>
                  </li>
                  <li className="flex items-start gap-2">
                    <ShieldCheck aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-primary" />
                    <div>
                      <strong>Write — on</strong>
                      <p className="text-muted-foreground">
                        ChatGPT can create files and apply patches.
                      </p>
                    </div>
                  </li>
                  <li className="flex items-start gap-2">
                    <AlertTriangle aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-warning" />
                    <div>
                      <strong>Execute — on (Full PowerShell)</strong>
                      <p className="text-muted-foreground">
                        Scripts run with your Windows account permissions. Toggle anything off on
                        the Workspaces page.
                      </p>
                    </div>
                  </li>
                </ul>
              </div>
            ) : null}

            {step === "start-connection" ? (
              <p className="text-sm text-muted-foreground">
                Ready to connect. We will start the local MCP server, publish it through Tailscale
                Funnel, and ask you to sign in if needed.
              </p>
            ) : null}

            <DialogFooter className="items-center sm:justify-between">
              <Button variant="ghost" onClick={onClose} className="mr-auto">
                Set up later
              </Button>
              <div className="flex items-center gap-2">
                {stepIndex > 0 ? (
                  <Button
                    variant="outline"
                    onClick={() => setStepIndex((index) => index - 1)}
                    disabled={adding || starting}
                  >
                    Back
                  </Button>
                ) : null}
                <Button
                  onClick={() => {
                    if (step === "add-workspace" && workspaces.length === 0) {
                      reportError("Add at least one workspace folder before continuing.");
                      return;
                    }
                    if (!isLastSetupStep) {
                      setStepIndex((index) => index + 1);
                    } else {
                      void startConnection();
                    }
                  }}
                  disabled={adding || starting}
                >
                  {starting ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
                  {isLastSetupStep ? "Start connection" : "Continue"}
                </Button>
              </div>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

function isLiveStage(
  connection: ConnectionStatus | null,
): connection is ConnectionStatus & {
  state: Extract<
    ConnectionStatus["state"],
    {
      state:
        | "needs_login"
        | "needs_funnel_permission"
        | "needs_https_setup"
        | "verifying"
        | "online"
        | "error";
    }
  >;
} {
  const state = connection?.state.state;
  return (
    state === "needs_login" ||
    state === "needs_funnel_permission" ||
    state === "needs_https_setup" ||
    state === "verifying" ||
    state === "online" ||
    state === "error"
  );
}

interface LiveStageProps {
  connection: ConnectionStatus | null;
  starting: boolean;
  onRetry: () => void;
  onFinish: () => void | Promise<void>;
  reportError: (message: string) => void;
}

function LiveStage({ connection, starting, onRetry, onFinish, reportError }: LiveStageProps) {
  const [helpVisible, setHelpVisible] = useState(false);

  if (!connection || !isLiveStage(connection)) return null;
  const state = connection.state;

  if (state.state === "needs_login") {
    return (
      <>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <LifeBuoy aria-hidden="true" className="size-5 text-warning" />
            Sign in to Tailscale
          </DialogTitle>
          <DialogDescription aria-live="polite">
            One more step: sign in to your Tailscale account so this app can publish its endpoint.
            A browser window will open at login.tailscale.com.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            onClick={() => {
              api.openExternalUrl(state.login_url).catch((err: unknown) => reportError(parseErrorMessage(err)));
            }}
            disabled={starting}
          >
            Sign in to Tailscale
          </Button>
          <Button variant="outline" onClick={onRetry} disabled={starting}>
            {starting ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
            I signed in — retry
          </Button>
        </div>
        <p className="text-[13px] text-muted-foreground">
          After signing in, this dialog updates automatically.
        </p>
      </>
    );
  }

  if (state.state === "needs_funnel_permission" || state.state === "needs_https_setup") {
    const https = state.state === "needs_https_setup";
    return (
      <>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <AlertTriangle aria-hidden="true" className="size-5 text-warning" />
            One-time approval needed
          </DialogTitle>
          <DialogDescription aria-live="polite">
            {https
              ? "Publishing over Funnel requires HTTPS certificates, which are currently disabled for your tailnet. A tailnet admin can enable them once in the Tailscale admin console under DNS. Once enabled, press Retry."
              : "Before this app can publish its public address, your tailnet needs Funnel enabled. A tailnet admin can turn it on in the Tailscale admin console under Access controls. Once approved, press Retry."}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-wrap items-center gap-2">
          <Button onClick={onRetry} disabled={starting}>
            {starting ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
            Retry
          </Button>
          <Button
            variant="outline"
            onClick={() => {
              api
                .openExternalUrl(
                  https
                    ? "https://login.tailscale.com/admin/settings/dns"
                    : "https://login.tailscale.com/admin/acls",
                )
                .catch((err: unknown) => reportError(parseErrorMessage(err)));
            }}
          >
            Open admin console
          </Button>
          <Button variant="ghost" onClick={() => setHelpVisible((v) => !v)} aria-expanded={helpVisible}>
            <LifeBuoy aria-hidden="true" />
            Setup help
          </Button>
        </div>
        {helpVisible ? (
          <div className="rounded-md border bg-muted/50 p-3 text-[13px] text-muted-foreground">
            {state.message}
          </div>
        ) : null}
      </>
    );
  }

  if (state.state === "verifying") {
    // Mid-transition: the run is live and the public endpoint is being
    // health-checked. Stay in the live stage with progress instead of
    // flickering back to the wizard, mirroring the dashboard
    // ConnectionCard's inert "Verifying…" presentation.
    return (
      <>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Loader2 aria-hidden="true" className="size-5 animate-spin text-primary" />
            Almost there
          </DialogTitle>
          <DialogDescription aria-live="polite">
            Verifying the public MCP endpoint… this dialog updates automatically.
          </DialogDescription>
        </DialogHeader>
        <p className="min-w-0 truncate font-mono text-xs text-muted-foreground" title={state.fqdn}>
          {state.fqdn}
        </p>
      </>
    );
  }

  if (state.state === "error") {
    // Retry (stop then start) is the only action that can recover a run in
    // the Error state: ConnectionManager::start() is intentionally a no-op
    // for every non-Stopped state, so the wizard's "Start connection" button
    // would do nothing here. Transient public-health failures may also clear
    // on their own while the sidecar re-verifies.
    return (
      <>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <AlertTriangle aria-hidden="true" className="size-5 text-destructive" />
            Connection failed
          </DialogTitle>
          <DialogDescription aria-live="polite">{state.message}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-wrap items-center gap-2">
          <Button onClick={onRetry} disabled={starting}>
            {starting ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
            Retry
          </Button>
        </div>
        <p className="text-[13px] text-muted-foreground">
          Retry stops the connection and starts it again. Transient public-health failures may
          also clear on their own while the app keeps checking.
        </p>
      </>
    );
  }

  // Online: Copy endpoint + Finish.
  return (
    <>
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <CheckCircle2 aria-hidden="true" className="size-5 text-success" />
          You are connected
        </DialogTitle>
        <DialogDescription aria-live="polite">
          Copy this secret capability URL into ChatGPT to finish setup. The URL is public on the
          internet — anyone who has it can call this MCP connection, so share it only with your own
          ChatGPT.
        </DialogDescription>
      </DialogHeader>
      <EndpointField
        url={state.endpoint}
        label="Public MCP endpoint (secret capability URL)"
        hint="Anyone who has this URL can call this MCP connection — share it only with your own ChatGPT."
      />
      <DialogFooter>
        <Button onClick={() => void onFinish()}>Finish</Button>
      </DialogFooter>
    </>
  );
}
