import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, parseErrorMessage, Workspace } from "../api";

interface Props {
  workspaces: Workspace[];
  onChanged: () => void | Promise<void>;
  onError: (message: string | null) => void;
}

export default function WorkspaceList({ workspaces, onChanged, onError }: Props) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  const add = async () => {
    onError(null);
    setAdding(true);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Choose a workspace folder" });
      if (typeof dir === "string") {
        await api.addWorkspace(dir);
        await onChanged();
      }
    } catch (err) {
      onError(parseErrorMessage(err));
    } finally {
      setAdding(false);
    }
  };

  const toggle = async (ws: Workspace, cap: "read" | "write") => {
    onError(null);
    setBusyId(ws.id);
    try {
      if (cap === "read") {
        const nextRead = !ws.can_read;
        await api.setCapabilities(ws.id, nextRead, nextRead ? ws.can_write : false);
      } else {
        const nextWrite = !ws.can_write;
        await api.setCapabilities(ws.id, nextWrite ? true : ws.can_read, nextWrite);
      }
      await onChanged();
    } catch (err) {
      onError(parseErrorMessage(err));
    } finally {
      setBusyId(null);
    }
  };

  const remove = async (ws: Workspace) => {
    if (!confirm(`Remove “${ws.name}” from MCP access? Files on disk will not be deleted.`)) return;
    onError(null);
    setBusyId(ws.id);
    try {
      await api.removeWorkspace(ws.id);
      await onChanged();
    } catch (err) {
      onError(parseErrorMessage(err));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <section className="panel workspace-panel">
      <div className="section-heading">
        <div>
          <span className="eyebrow">Approved folders</span>
          <h2>Workspaces</h2>
          <p>New folders start with Read enabled and Write disabled.</p>
        </div>
        <button className="btn btn-primary" onClick={() => void add()} disabled={adding}>
          {adding ? "Adding…" : "+ Add workspace"}
        </button>
      </div>

      {workspaces.length === 0 ? (
        <div className="empty-state">
          <div className="empty-icon">▱</div>
          <strong>No workspaces approved</strong>
          <p>Add a folder to let ChatGPT list, search and read files inside that boundary.</p>
          <button className="btn btn-primary" onClick={() => void add()} disabled={adding}>Add your first workspace</button>
        </div>
      ) : (
        <div className="workspace-list">
          {workspaces.map((ws) => {
            const busy = busyId === ws.id;
            return (
              <article className={`workspace-card ${busy ? "busy" : ""}`} key={ws.id}>
                <div className="workspace-main">
                  <div className="workspace-icon">▱</div>
                  <div className="workspace-copy">
                    <div className="workspace-title-row">
                      <strong>{ws.name}</strong>
                      <span className={`badge ${ws.can_read ? "badge-success" : "badge-neutral"}`}>
                        {ws.can_read ? "Active" : "Disabled"}
                      </span>
                    </div>
                    <code className="workspace-path">{ws.root_path}</code>
                  </div>
                </div>

                <div className="workspace-controls">
                  <label className="permission-toggle">
                    <div>
                      <strong>Read</strong>
                      <span>List, search and inspect files</span>
                    </div>
                    <input
                      type="checkbox"
                      checked={ws.can_read}
                      disabled={busy}
                      onChange={() => void toggle(ws, "read")}
                    />
                  </label>
                  <label className="permission-toggle">
                    <div>
                      <strong>Write</strong>
                      <span>Create files and apply anchored patches</span>
                    </div>
                    <input
                      type="checkbox"
                      checked={ws.can_write}
                      disabled={busy}
                      onChange={() => void toggle(ws, "write")}
                    />
                  </label>
                  <button className="btn btn-danger-ghost" onClick={() => void remove(ws)} disabled={busy}>
                    Remove
                  </button>
                </div>
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}
