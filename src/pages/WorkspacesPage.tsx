import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, Loader2, TerminalSquare } from "lucide-react";
import ConfirmDialog from "../components/ConfirmDialog";
import EmptyState from "../components/EmptyState";
import { api, parseErrorMessage } from "../api";
import type { ExecutionMode, Workspace } from "@/lib/types";

/**
 * Workspaces permission management (plan Task 18):
 * one card per workspace exposing exactly three top-level capabilities
 * (Read / Write / Execute), with the execution mode nested under Execute.
 * Backend remains authoritative; the UI merely keeps its own toggles coherent.
 */

const FULL_POWERSHELL_TITLE = "Enable Full PowerShell?";
const FULL_POWERSHELL_BODY =
  "PowerShell will start in this workspace, but the working directory is not a security sandbox. Scripts may access files, processes, the network, and other resources allowed to your Windows account. Enable this only for workspaces you trust.";
const REMOVE_TITLE = "Remove workspace?";
const REMOVE_BODY = "ChatGPT will lose access to this folder. Files on disk will not be deleted.";

/** Concise list shown under Approved Commands mode (mirrors backend policy defaults). */
const APPROVED_COMMANDS = [
  "git",
  "cargo",
  "rustc",
  "npm",
  "npx",
  "node",
  "pnpm",
  "yarn",
  "python",
  "pip",
  "go",
  "dotnet",
];

type PendingConfirm =
  | { kind: "full-powershell"; workspace: Workspace }
  | { kind: "remove"; workspace: Workspace }
  | null;

interface Props {
  workspaces: Workspace[];
  /** Refreshes the workspace list from the backend after any mutation. */
  onWorkspacesChanged: () => Promise<void>;
  reportError: (message: string) => void;
}

export default function WorkspacesPage({ workspaces, onWorkspacesChanged, reportError }: Props) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [pendingConfirm, setPendingConfirm] = useState<PendingConfirm>(null);
  const lastAddedRef = useRef<HTMLElement | null>(null);
  const lastAddedId = useRef<string | null>(null);

  // Focus/scroll to the freshly added workspace once it appears in the list.
  useEffect(() => {
    if (!lastAddedId.current) return;
    lastAddedRef.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    lastAddedRef.current?.focus();
    lastAddedId.current = null;
  }, [workspaces]);

  async function addWorkspace() {
    setAdding(true);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Choose a workspace folder" });
      if (typeof dir === "string") {
        const created = await api.addWorkspace(dir);
        lastAddedId.current = created.id;
        await onWorkspacesChanged();
      }
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setAdding(false);
    }
  }

  async function setCapabilities(id: string, canRead: boolean, canWrite: boolean) {
    setBusyId(id);
    try {
      await api.setCapabilities(id, canRead, canWrite);
      await onWorkspacesChanged();
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setBusyId(null);
    }
  }

  /**
   * Coherent Read/Write toggle rules (plan Task 18 Step 2):
   * - disabling Read also disables Write,
   * - enabling Write while Read is off also enables Read.
   */
  function toggleCapability(ws: Workspace, cap: "read" | "write") {
    if (busyId) return;
    if (cap === "read") {
      if (ws.can_read) {
        void setCapabilities(ws.id, false, false);
      } else {
        void setCapabilities(ws.id, true, ws.can_write);
      }
    } else {
      const nextWrite = !ws.can_write;
      if (nextWrite && !ws.can_read) {
        void setCapabilities(ws.id, true, true);
      } else {
        void setCapabilities(ws.id, ws.can_read, nextWrite);
      }
    }
  }

  async function applyExecutionAccess(ws: Workspace, canExecute: boolean, mode: ExecutionMode) {
    setBusyId(ws.id);
    try {
      await api.setExecutionAccess(ws.id, canExecute, mode);
      await onWorkspacesChanged();
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setBusyId(null);
    }
  }

  function toggleExecute(ws: Workspace) {
    if (busyId) return;
    if (ws.can_execute) {
      // Turning Execute off keeps the stored mode so re-enabling is explicit.
      void applyExecutionAccess(ws, false, ws.execution_mode);
    } else if (ws.execution_mode === "full_powershell") {
      // Re-enabling straight into Full PowerShell still requires confirmation.
      setPendingConfirm({ kind: "full-powershell", workspace: ws });
    } else {
      void applyExecutionAccess(ws, true, "approved_commands");
    }
  }

  function changeMode(ws: Workspace, mode: ExecutionMode) {
    if (busyId || !ws.can_execute || mode === ws.execution_mode) return;
    if (mode === "full_powershell") {
      setPendingConfirm({ kind: "full-powershell", workspace: ws });
    } else {
      void applyExecutionAccess(ws, true, mode);
    }
  }

  function confirmPending() {
    if (!pendingConfirm) return;
    const { kind, workspace } = pendingConfirm;
    setPendingConfirm(null);
    if (kind === "full-powershell") {
      void applyExecutionAccess(workspace, true, "full_powershell");
    } else {
      setBusyId(workspace.id);
      void (async () => {
        try {
          await api.removeWorkspace(workspace.id);
          await onWorkspacesChanged();
        } catch (err) {
          reportError(parseErrorMessage(err));
        } finally {
          setBusyId(null);
        }
      })();
    }
  }

  return (
    <div className="workspaces-page">
      <section className="ws-toolbar">
        <div className="ws-toolbar-copy">
          <h2>Approved folders</h2>
          <p>New folders start with Read enabled, Write and Execute disabled.</p>
        </div>
        <button className="btn btn-primary" onClick={() => void addWorkspace()} disabled={adding}>
          {adding ? <Loader2 aria-hidden="true" className="spinner" /> : <FolderPlus aria-hidden="true" />}
          Add folder…
        </button>
      </section>

      {workspaces.length === 0 ? (
        <EmptyState
          icon={FolderPlus}
          title="No workspaces yet"
          message="Add a folder to let ChatGPT list, search and read files inside that boundary."
        />
      ) : (
        <div className="workspace-list">
          {workspaces.map((ws) => {
            const busy = busyId === ws.id;
            const isNew = lastAddedId.current === ws.id;
            return (
              <article
                key={ws.id}
                ref={isNew ? (node) => (lastAddedRef.current = node) : undefined}
                tabIndex={-1}
                className={`workspace-card ${busy ? "busy" : ""}`}
                aria-busy={busy}
              >
                <div className="ws-card-head">
                  <div>
                    <strong className="ws-name">{ws.name}</strong>
                    <code className="ws-path">{ws.root_path}</code>
                  </div>
                  {busy ? <Loader2 aria-hidden="true" className="spinner ws-spinner" /> : null}
                </div>

                <div className="permission-grid" role="group" aria-label={`Permissions for ${ws.name}`}>
                  <label className="permission-toggle">
                    <span className="permission-copy">
                      <strong>Read</strong>
                      <span>List, search and inspect files</span>
                    </span>
                    <input
                      type="checkbox"
                      checked={ws.can_read}
                      disabled={busy}
                      onChange={() => toggleCapability(ws, "read")}
                    />
                  </label>
                  <label className="permission-toggle">
                    <span className="permission-copy">
                      <strong>Write</strong>
                      <span>Create files and apply anchored patches</span>
                    </span>
                    <input
                      type="checkbox"
                      checked={ws.can_write}
                      disabled={busy}
                      onChange={() => toggleCapability(ws, "write")}
                    />
                  </label>
                  <label className="permission-toggle">
                    <span className="permission-copy">
                      <strong>Execute</strong>
                      <span>Run approved programs inside this folder</span>
                    </span>
                    <input
                      type="checkbox"
                      checked={ws.can_execute}
                      disabled={busy}
                      onChange={() => toggleExecute(ws)}
                    />
                  </label>
                </div>

                {ws.can_execute ? (
                  <div className="exec-detail">
                    <div className="mode-row" role="radiogroup" aria-label="Execution mode">
                      <button
                        type="button"
                        role="radio"
                        aria-checked={ws.execution_mode === "approved_commands"}
                        className={`mode-option ${ws.execution_mode === "approved_commands" ? "selected" : ""}`}
                        disabled={busy}
                        onClick={() => changeMode(ws, "approved_commands")}
                      >
                        Approved commands
                      </button>
                      <button
                        type="button"
                        role="radio"
                        aria-checked={ws.execution_mode === "full_powershell"}
                        className={`mode-option ${ws.execution_mode === "full_powershell" ? "selected" : ""}`}
                        disabled={busy}
                        onClick={() => changeMode(ws, "full_powershell")}
                      >
                        <TerminalSquare aria-hidden="true" />
                        Full PowerShell
                      </button>
                    </div>

                    {ws.execution_mode === "approved_commands" ? (
                      <div className="exec-note">
                        <span>Allowed executables:</span>
                        <ul className="approved-list" aria-label="Approved executables">
                          {APPROVED_COMMANDS.map((cmd) => (
                            <li key={cmd}>{cmd}</li>
                          ))}
                        </ul>
                      </div>
                    ) : (
                      <p className="exec-warning">High risk: unrestricted PowerShell scripts run with your Windows account permissions.</p>
                    )}
                  </div>
                ) : null}

                <div className="ws-card-actions">
                  <button className="btn btn-danger-ghost" onClick={() => setPendingConfirm({ kind: "remove", workspace: ws })} disabled={busy}>
                    Remove
                  </button>
                </div>
              </article>
            );
          })}
        </div>
      )}

      <ConfirmDialog
        open={pendingConfirm?.kind === "full-powershell"}
        title={FULL_POWERSHELL_TITLE}
        message={FULL_POWERSHELL_BODY}
        tone="warning"
        confirmLabel="Enable Full PowerShell"
        onCancel={() => setPendingConfirm(null)}
        onConfirm={confirmPending}
      />

      <ConfirmDialog
        open={pendingConfirm?.kind === "remove"}
        title={REMOVE_TITLE}
        message={REMOVE_BODY}
        tone="danger"
        confirmLabel="Remove"
        onCancel={() => setPendingConfirm(null)}
        onConfirm={confirmPending}
      />
    </div>
  );
}
