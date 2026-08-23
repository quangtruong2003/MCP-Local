import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, parseErrorMessage, Workspace } from "../api";

interface Props {
  workspaces: Workspace[];
  onChanged: () => void;
  onError: (message: string | null) => void;
}

export default function WorkspaceList({ workspaces, onChanged, onError }: Props) {
  const [busyId, setBusyId] = useState<string | null>(null);

  const add = async () => {
    onError(null);
    try {
      const dir = await open({ directory: true, multiple: false });
      if (typeof dir === "string") {
        await api.addWorkspace(dir);
        onChanged();
      }
    } catch (err) {
      onError(parseErrorMessage(err));
    }
  };

  const toggle = async (ws: Workspace, cap: "read" | "write") => {
    onError(null);
    setBusyId(ws.id);
    try {
      if (cap === "read") {
        await api.setCapabilities(ws.id, !ws.can_read, ws.can_write);
      } else {
        await api.setCapabilities(ws.id, ws.can_read, !ws.can_write);
      }
      onChanged();
    } catch (err) {
      onError(parseErrorMessage(err));
    } finally {
      setBusyId(null);
    }
  };

  const remove = async (ws: Workspace) => {
    if (!confirm(`Remove workspace "${ws.name}"? Files on disk are NOT deleted.`)) return;
    onError(null);
    setBusyId(ws.id);
    try {
      await api.removeWorkspace(ws.id);
      onChanged();
    } catch (err) {
      onError(parseErrorMessage(err));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <section>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
        }}
      >
        <h2 style={{ margin: 0, fontSize: 16 }}>Workspaces</h2>
        <button onClick={add}>+ Add folder</button>
      </div>
      <p style={{ margin: "4px 0 8px", color: "var(--text-muted)", fontSize: 12 }}>
        Folders ChatGPT may access. New folders start with read on and write off.
      </p>
      {workspaces.map((ws) => (
        <div
          key={ws.id}
          style={{
            display: "flex",
            gap: 12,
            alignItems: "center",
            padding: "10px 12px",
            marginBottom: 8,
            background: "var(--surface)",
            border: "1px solid var(--border)",
            borderRadius: 8,
            opacity: busyId === ws.id ? 0.6 : 1,
          }}
        >
          <div style={{ flex: 1, minWidth: 0 }}>
            <strong>{ws.name}</strong>
            <div
              style={{
                fontSize: 12,
                color: "var(--text-muted)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {ws.root_path}
            </div>
          </div>
          <label className="check">
            <input
              type="checkbox"
              checked={ws.can_read}
              disabled={busyId === ws.id}
              onChange={() => toggle(ws, "read")}
            />
            Read
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={ws.can_write}
              disabled={busyId === ws.id}
              onChange={() => toggle(ws, "write")}
            />
            Write
          </label>
          <button
            onClick={() => remove(ws)}
            disabled={busyId === ws.id}
            style={{ color: "var(--danger)" }}
          >
            Remove
          </button>
        </div>
      ))}
      {workspaces.length === 0 && (
        <p style={{ color: "var(--text-muted)", fontStyle: "italic" }}>
          No workspaces yet — add a folder to let ChatGPT access it.
        </p>
      )}
    </section>
  );
}
