import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, HardDrive, Loader2, ShieldAlert } from "lucide-react";
import { WorkspaceCard } from "@/components/features/workspaces/workspace-card";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ConfirmDialog } from "@/components/shared/confirm-dialog";
import { EmptyState } from "@/components/shared/empty-state";
import { api, parseErrorMessage } from "../api";
import type { AppSettings, ExecutionMode, Workspace } from "@/lib/types";

/**
 * Workspaces permission management:
 * one card per workspace exposing exactly three top-level capabilities
 * (Read / Write / Execute), with the execution mode nested under Execute.
 * Backend remains authoritative; the UI merely keeps its own toggles coherent.
 * UI-added folders and drives arrive with full access (backend `add_workspace`).
 */

const FULL_POWERSHELL_TITLE = "Enable Full PowerShell?";
const FULL_POWERSHELL_BODY =
  "PowerShell will start in this workspace, but the working directory is not a security sandbox. Scripts may access files, processes, the network, and other resources allowed to your Windows account. Enable this only for workspaces you trust.";
const FULL_COMPUTER_TITLE = "Give ChatGPT full access to this computer?";
const FULL_COMPUTER_BODY =
  "Every detected drive is added as a workspace with Read, Write and Full PowerShell. Unrestricted mode disables sensitive-file filtering and approved-command checks, and raises execution limits across workspaces. Pause and audit stay active. File tools honor workspace boundaries, but PowerShell scripts can access anything allowed by your Windows account.";
const REMOVE_TITLE = "Remove workspace?";
const REMOVE_BODY = "ChatGPT will lose access to this workspace. Files on disk will not be deleted.";

type PendingConfirm =
  | { kind: "full-powershell"; workspace: Workspace }
  | { kind: "full-computer" }
  | { kind: "remove"; workspace: Workspace }
  | null;

interface Props {
  workspaces: Workspace[];
  settings: AppSettings | null;
  /** Refreshes the workspace list from the backend after any mutation. */
  onWorkspacesChanged: () => Promise<void>;
  /** Persists settings; used when granting full computer access. */
  onSaveSettings: (next: AppSettings) => Promise<void>;
  reportError: (message: string) => void;
}

/** Case-insensitive root comparison so `C:\` matches the stored workspace root. */
function normalizeRoot(path: string) {
  return path.replace(/[\\/]+$/, "").toLowerCase();
}

export default function WorkspacesPage({
  workspaces,
  settings,
  onWorkspacesChanged,
  onSaveSettings,
  reportError,
}: Props) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [fullBusy, setFullBusy] = useState(false);
  const [drives, setDrives] = useState<string[] | null>(null);
  const [pendingConfirm, setPendingConfirm] = useState<PendingConfirm>(null);
  const lastAddedRef = useRef<HTMLElement | null>(null);
  const lastAddedId = useRef<string | null>(null);

  const existingRoots = new Set(workspaces.map((ws) => normalizeRoot(ws.root_path)));

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

  async function addDrive(drive: string) {
    if (adding) return;
    setAdding(true);
    try {
      const created = await api.addWorkspace(drive);
      lastAddedId.current = created.id;
      await onWorkspacesChanged();
    } catch (err) {
      reportError(parseErrorMessage(err));
    } finally {
      setAdding(false);
    }
  }

  async function loadDrives() {
    try {
      setDrives(await api.listDrives());
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }

  async function grantFullComputerAccess() {
    if (fullBusy || !settings) return;
    setFullBusy(true);
    try {
      const allDrives = await api.listDrives();
      const known = new Set(workspaces.map((ws) => normalizeRoot(ws.root_path)));
      for (const drive of allDrives) {
        if (!known.has(normalizeRoot(drive))) await api.addWorkspace(drive);
      }
      if (!settings.unrestricted_access) {
        await onSaveSettings({ ...settings, unrestricted_access: true });
      }
      await onWorkspacesChanged();
    } catch (err) {
      reportError(parseErrorMessage(err));
      await onWorkspacesChanged();
    } finally {
      setFullBusy(false);
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
    const pending = pendingConfirm;
    setPendingConfirm(null);
    if (pending.kind === "full-powershell") {
      void applyExecutionAccess(pending.workspace, true, "full_powershell");
    } else if (pending.kind === "full-computer") {
      void grantFullComputerAccess();
    } else {
      setBusyId(pending.workspace.id);
      void (async () => {
        try {
          await api.removeWorkspace(pending.workspace.id);
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
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <h2 className="text-sm font-semibold">Approved folders</h2>
          <p className="text-[13px] text-muted-foreground">
            Folders and drives get Read, Write and Full PowerShell immediately. Use the switches to
            revoke anything.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            onClick={() => setPendingConfirm({ kind: "full-computer" })}
            disabled={fullBusy || !settings}
          >
            {fullBusy ? <Loader2 aria-hidden="true" className="animate-spin" /> : <ShieldAlert aria-hidden="true" />}
            Full computer access
          </Button>
          <DropdownMenu
            onOpenChange={(isOpen) => {
              if (isOpen) void loadDrives();
            }}
          >
            <DropdownMenuTrigger asChild>
              <Button variant="outline" disabled={adding}>
                <HardDrive aria-hidden="true" />
                Add drive…
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {drives === null ? (
                <DropdownMenuItem disabled>Loading drives…</DropdownMenuItem>
              ) : drives.length === 0 ? (
                <DropdownMenuItem disabled>No drives found</DropdownMenuItem>
              ) : (
                drives.map((drive) => {
                  const added = existingRoots.has(normalizeRoot(drive));
                  return (
                    <DropdownMenuItem
                      key={drive}
                      disabled={added || adding}
                      onSelect={() => void addDrive(drive)}
                    >
                      <HardDrive aria-hidden="true" />
                      {drive}
                      {added ? <span className="text-muted-foreground">· added</span> : null}
                    </DropdownMenuItem>
                  );
                })
              )}
            </DropdownMenuContent>
          </DropdownMenu>
          <Button onClick={() => void addWorkspace()} disabled={adding}>
            {adding ? <Loader2 aria-hidden="true" className="animate-spin" /> : <FolderPlus aria-hidden="true" />}
            Add folder…
          </Button>
        </div>
      </div>

      {workspaces.length === 0 ? (
        <EmptyState
          icon={FolderPlus}
          title="No workspaces yet"
          message="Add a folder or drive to let ChatGPT work with files inside that boundary."
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
        open={pendingConfirm?.kind === "full-computer"}
        title={FULL_COMPUTER_TITLE}
        message={FULL_COMPUTER_BODY}
        tone="danger"
        confirmLabel="Give full access"
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
