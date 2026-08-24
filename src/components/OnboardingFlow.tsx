import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  CheckCircle2,
  Copy,
  FolderPlus,
  LifeBuoy,
  Loader2,
  ShieldCheck,
} from "lucide-react";
import { api, parseErrorMessage } from "../api";
import type { ConnectionStatus, Workspace } from "../types";

/**
 * Guided first-run onboarding (plan Task 17):
 * Welcome -> Add workspace -> Review Read ON / Write OFF -> Start connection,
 * then live connection states take over: NeedsLogin (Sign in to Tailscale) ->
 * NeedsFunnelPermission (one-time approval explanation + Retry) ->
 * Online (Copy endpoint + Finish).
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
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    cardRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

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
  function startConnection() {
    setStarting(true);
    api.startConnection().catch((err) => {
      reportError(parseErrorMessage(err));
      setStarting(false);
    });
  }

  const step = SETUP_STEPS[stepIndex];
  const isLastSetupStep = stepIndex === SETUP_STEPS.length - 1;

  return (
    <div
      className="dialog-backdrop"
      role="presentation"
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={cardRef}
        className="dialog-card onboarding-card"
        role="dialog"
        aria-modal="true"
        aria-labelledby="onboarding-title"
        tabIndex={-1}
      >
        {/* Live-state stages replace the setup wizard once a run has started. */}
        <LiveStage
          connection={connection}
          starting={starting}
          onStart={startConnection}
          onFinish={onFinish}
          reportError={reportError}
        />
        {isLiveStage(connection) ? null : (
          <>
            <header className="onboarding-head">
              <span className="onboarding-step-count">
                Step {stepIndex + 1} of {SETUP_STEPS.length}
              </span>
              <h2 id="onboarding-title">{STEP_TITLES[step]}</h2>
            </header>

            {step === "welcome" ? (
              <div className="onboarding-body">
                <p>
                  This app gives ChatGPT safe access to folders you approve, over your own private
                  Tailscale network. Setup takes about two minutes.
                </p>
                <ul className="onboarding-points">
                  <li>Approve which folders the assistant may reach</li>
                  <li>Sign in to Tailscale once</li>
                  <li>Get a private web address only your devices can reach</li>
                </ul>
              </div>
            ) : null}

            {step === "add-workspace" ? (
              <div className="onboarding-body">
                <p>
                  Pick at least one folder ChatGPT may read. Nothing outside these boundaries is
                  ever touched.
                </p>
                {workspaces.length > 0 ? (
                  <ul className="onboarding-added-list" aria-label="Added workspaces">
                    {workspaces.map((ws) => (
                      <li key={ws.id}>
                        <FolderPlus aria-hidden="true" />
                        {ws.name}
                      </li>
                    ))}
                  </ul>
                ) : null}
                <div className="conn-actions">
                  <button className="btn btn-secondary" onClick={() => void addWorkspace()} disabled={adding}>
                    {adding ? "Adding…" : "Choose folder…"}
                  </button>
                </div>
              </div>
            ) : null}

            {step === "review-permissions" ? (
              <div className="onboarding-body">
                <p>Every new folder starts with safe defaults:</p>
                <ul className="onboarding-permissions">
                  <li>
                    <ShieldCheck aria-hidden="true" />
                    <div>
                      <strong>Read — on</strong>
                      <span>ChatGPT can list and read files inside the folder.</span>
                    </div>
                  </li>
                  <li>
                    <ShieldCheck aria-hidden="true" />
                    <div>
                      <strong>Write — off</strong>
                      <span>ChatGPT cannot create or change files until you allow it.</span>
                    </div>
                  </li>
                </ul>
                <p className="onboarding-note">You can change both later on the Workspaces page.</p>
              </div>
            ) : null}

            {step === "start-connection" ? (
              <div className="onboarding-body">
                <p>
                  Ready to connect. We will start the local MCP server, publish it through Tailscale
                  Funnel, and ask you to sign in if needed.
                </p>
              </div>
            ) : null}

            <footer className="dialog-actions onboarding-footer">
              {stepIndex > 0 ? (
                <button
                  className="btn btn-secondary"
                  onClick={() => setStepIndex((index) => index - 1)}
                  disabled={adding || starting}
                >
                  Back
                </button>
              ) : null}
              <button
                className="btn btn-primary"
                onClick={() => {
                  if (step === "add-workspace" && workspaces.length === 0) {
                    reportError("Add at least one workspace folder before continuing.");
                    return;
                  }
                  if (!isLastSetupStep) {
                    setStepIndex((index) => index + 1);
                  } else {
                    startConnection();
                  }
                }}
                disabled={adding || starting}
              >
                {starting ? <Loader2 aria-hidden="true" className="spinner" /> : null}
                {isLastSetupStep ? "Start connection" : "Continue"}
              </button>
            </footer>

            <button className="link-button onboarding-skip" onClick={onClose}>
              Set up later — you can finish from the Dashboard
            </button>
          </>
        )}
      </div>
    </div>
  );
}

function isLiveStage(
  connection: ConnectionStatus | null,
): connection is ConnectionStatus & {
  state: Extract<ConnectionStatus["state"], { state: "needs_login" | "needs_funnel_permission" | "online" }>;
} {
  const state = connection?.state.state;
  return state === "needs_login" || state === "needs_funnel_permission" || state === "online";
}

interface LiveStageProps {
  connection: ConnectionStatus | null;
  starting: boolean;
  onStart: () => void;
  onFinish: () => void | Promise<void>;
  reportError: (message: string) => void;
}

function LiveStage({ connection, starting, onStart, onFinish, reportError }: LiveStageProps) {
  const [copied, setCopied] = useState(false);
  const [helpVisible, setHelpVisible] = useState(false);

  if (!connection || !isLiveStage(connection)) return null;
  const state = connection.state;

  async function copyEndpoint(endpoint: string) {
    try {
      await navigator.clipboard.writeText(endpoint);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      reportError("Could not access the clipboard to copy the endpoint URL.");
    }
  }

  if (state.state === "needs_login") {
    return (
      <div className="onboarding-stage">
        <span className="conn-state conn-state-warning">
          <LifeBuoy aria-hidden="true" />
          Sign in to Tailscale
        </span>
        <p className="conn-copy" aria-live="polite">
          One more step: sign in to your Tailscale account so this app can join your private
          network. A browser window will open at login.tailscale.com.
        </p>
        <div className="conn-actions">
          <button
            className="btn btn-primary"
            onClick={() => {
              api.openExternalUrl(state.login_url).catch((err) => reportError(parseErrorMessage(err)));
            }}
            disabled={starting}
          >
            Sign in to Tailscale
          </button>
          <button className="btn btn-secondary" onClick={onStart} disabled={starting}>
            {starting ? <Loader2 aria-hidden="true" className="spinner" /> : null}
            I signed in — retry
          </button>
        </div>
        <p className="onboarding-note">After signing in, this dialog updates automatically.</p>
      </div>
    );
  }

  if (state.state === "needs_funnel_permission") {
    return (
      <div className="onboarding-stage">
        <span className="conn-state conn-state-warning">
          <AlertTriangle aria-hidden="true" />
          One-time approval needed
        </span>
        <p className="conn-copy" aria-live="polite">
          Before this app can publish its public address, your tailnet needs Funnel enabled. A
          tailnet admin can turn it on in the Tailscale admin console under Access controls. Once
          approved, press Retry.
        </p>
        <div className="conn-actions">
          <button className="btn btn-primary" onClick={onStart} disabled={starting}>
            {starting ? <Loader2 aria-hidden="true" className="spinner" /> : null}
            Retry
          </button>
          <button className="btn btn-secondary" onClick={() => setHelpVisible((v) => !v)} aria-expanded={helpVisible}>
            <LifeBuoy aria-hidden="true" />
            Setup help
          </button>
        </div>
        {helpVisible ? (
          <details className="onboarding-diagnostics">
            <summary>Technical details</summary>
            <p>{state.message}</p>
          </details>
        ) : null}
      </div>
    );
  }

  // Online: Copy endpoint + Finish.
  return (
    <div className="onboarding-stage">
      <span className="conn-state conn-state-ready">
        <CheckCircle2 aria-hidden="true" />
        You are connected
      </span>
      <p className="conn-copy" aria-live="polite">
        Copy this private web address into ChatGPT to finish setup.
      </p>
      <div className="endpoint-panel">
        <label htmlFor="onboarding-endpoint">Public MCP endpoint</label>
        <div className="endpoint-row">
          <input id="onboarding-endpoint" type="text" readOnly value={state.endpoint} />
          <button className="btn btn-secondary" onClick={() => void copyEndpoint(state.endpoint)}>
            <Copy aria-hidden="true" />
            {copied ? "Copied" : "Copy endpoint"}
          </button>
        </div>
      </div>
      <div className="dialog-actions">
        <button className="btn btn-primary" onClick={() => void onFinish()}>
          Finish
        </button>
      </div>
    </div>
  );
}
