import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, Loader2 } from "lucide-react";
import { WorkspaceCard } from "@/components/features/workspaces/workspace-card";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/shared/confirm-dialog";
import { EmptyState } from "@/components/shared/empty-state";
import { api, parseErrorMessage } from "../api";
import type { ExecutionMode, Workspace } from "@/lib/types";

/**
 * Workspaces permission management:
 * one card per workspace exposing exactly three top-level capabilities
 * (Read / Write / Execute), with the execution mode nested under Execute.
 * Backend remains authoritative; the UI merely keeps its own toggles coherent.
 */

const FULL_POWERSHELL_TITLE = "Enable Full PowerShell?";
const FULL_POWERSHELL_BODY =
  "PowerShell will start in this workspace, but the working directory is not a security sandbox. Scripts may access files, processes, the network, and other resources allowed to your Windows account. Enable this only for workspaces you trust.";
const REMOVE_TITLE = "Remove workspace?";
const REMOVE_BODY = "ChatGPT will lose access to this folder. Files on disk will not be deleted.";

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
   * Coherent Read/Write toggle rules:
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
    <div className="flex flex-col gap-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-sm font-semibold">Approved folders</h2>
          <p className="text-[13px] text-muted-foreground">
            New folders start with Read enabled, Write and Execute disabled.
          </p>
        </div>
        <Button onClick={() => void addWorkspace()} disabled={adding}>
          {adding ? <Loader2 aria-hidden="true" className="animate-spin" /> : <FolderPlus aria-hidden="true" />}
          Add folder…
        </Button>
      </div>

      {workspaces.length === 0 ? (
        <EmptyState
          icon={FolderPlus}
          title="No workspaces yet"
          message="Add a folder to let ChatGPT list, search and read files inside that boundary."
        />
      ) : (
        <div className="flex flex-col gap-3">
          {workspaces.map((ws) => (
            <WorkspaceCard
              key={ws.id}
              ws={ws}
              busy={busyId === ws.id}
              registerRef={
                lastAddedId.current === ws.id ? (node) => (lastAddedRef.current = node) : undefined
              }
              onToggleCapability={toggleCapability}
              onToggleExecute={toggleExecute}
              onChangeMode={changeMode}
              onRemove={(target) => setPendingConfirm({ kind: "remove", workspace: target })}
            />
          ))}
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
