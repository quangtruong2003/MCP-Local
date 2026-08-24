import { useEffect, useState } from "react";
import type { ConnectionSettingsSummary, SaveConnectionSettingsInput } from "../api";

interface Props {
  settings: ConnectionSettingsSummary | null;
  busy: boolean;
  onSave: (input: SaveConnectionSettingsInput) => Promise<boolean>;
  onRegenerate: () => Promise<boolean>;
}

export default function SettingsPage({ settings, busy, onSave, onRegenerate }: Props) {
  const [organizationId, setOrganizationId] = useState("");
  const [workspaceId, setWorkspaceId] = useState("");
  const [tunnelName, setTunnelName] = useState("Local Workspace MCP Manager");
  const [runtimeKey, setRuntimeKey] = useState("");
  const [adminKey, setAdminKey] = useState("");
  const [autoStart, setAutoStart] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (!settings) return;
    setOrganizationId(settings.organization_id ?? "");
    setWorkspaceId(settings.workspace_id ?? "");
    setTunnelName(settings.tunnel_name);
    setAutoStart(settings.auto_start);
  }, [settings]);

  const save = async () => {
    setSaved(false);
    const success = await onSave({
      organizationId: organizationId.trim() || null,
      workspaceId: workspaceId.trim() || null,
      tunnelName: tunnelName.trim() || "Local Workspace MCP Manager",
      autoStart,
      runtimeApiKey: runtimeKey.trim() || null,
      adminApiKey: adminKey.trim() || null,
    });
    if (!success) return;
    setRuntimeKey("");
    setAdminKey("");
    setSaved(true);
  };

  const regenerate = async () => {
    if (!confirm("Create a new tunnel? ChatGPT may need to be updated to use the new Tunnel ID.")) return;
    await onRegenerate();
  };

  return (
    <div className="settings-grid">
      <section className="panel settings-panel">
        <div className="section-heading">
          <div>
            <span className="eyebrow">OpenAI connection</span>
            <h2>Secure MCP Tunnel</h2>
            <p>Create a tunnel once, then reuse the same Tunnel ID on every Start.</p>
          </div>
        </div>

        <div className="form-grid two-col">
          <label className="field">
            <span>Organization ID</span>
            <input value={organizationId} onChange={(e) => setOrganizationId(e.target.value)} placeholder="org_…" />
            <small>Optional when a Workspace ID is provided.</small>
          </label>
          <label className="field">
            <span>Workspace ID</span>
            <input value={workspaceId} onChange={(e) => setWorkspaceId(e.target.value)} placeholder="ws_…" />
            <small>Use the ChatGPT workspace that should see this tunnel.</small>
          </label>
        </div>

        <label className="field">
          <span>Tunnel name</span>
          <input value={tunnelName} onChange={(e) => setTunnelName(e.target.value)} />
        </label>

        <div className="credential-block">
          <div className="credential-heading">
            <div>
              <strong>Runtime API key</strong>
              <p>Used by the long-running tunnel. Requires Tunnels Read + Use.</p>
            </div>
            <span className={`badge ${settings?.has_runtime_key ? "badge-success" : "badge-neutral"}`}>
              {settings?.has_runtime_key ? "Configured" : "Not configured"}
            </span>
          </div>
          <input
            type="password"
            value={runtimeKey}
            onChange={(e) => setRuntimeKey(e.target.value)}
            placeholder={settings?.has_runtime_key ? "Enter only to replace existing key" : "Paste runtime API key"}
            autoComplete="off"
          />
        </div>

        <div className="credential-block">
          <div className="credential-heading">
            <div>
              <strong>Admin API key</strong>
              <p>Used only to create or regenerate tunnel records.</p>
            </div>
            <span className={`badge ${settings?.has_admin_key ? "badge-success" : "badge-neutral"}`}>
              {settings?.has_admin_key ? "Configured" : "Not configured"}
            </span>
          </div>
          <input
            type="password"
            value={adminKey}
            onChange={(e) => setAdminKey(e.target.value)}
            placeholder={settings?.has_admin_key ? "Enter only to replace existing key" : "Paste admin API key"}
            autoComplete="off"
          />
        </div>

        <label className="toggle-row">
          <div>
            <strong>Start automatically</strong>
            <span>Reconnect using the saved tunnel when the desktop app opens.</span>
          </div>
          <input type="checkbox" checked={autoStart} onChange={(e) => setAutoStart(e.target.checked)} />
        </label>

        <div className="form-actions">
          {saved ? <span className="saved-note">Saved securely</span> : <span />}
          <button className="btn btn-primary" onClick={() => void save()} disabled={busy}>
            {busy ? "Saving…" : "Save settings"}
          </button>
        </div>
      </section>

      <section className="panel tunnel-identity-panel">
        <span className="eyebrow">Persistent identity</span>
        <h2>Current tunnel</h2>
        {settings?.tunnel_id ? (
          <>
            <code className="large-code">{settings.tunnel_id}</code>
            <p>Normal Start reuses this Tunnel ID. Regeneration is intentionally separate.</p>
            <button className="btn btn-danger-ghost" onClick={() => void regenerate()} disabled={busy}>
              Regenerate tunnel
            </button>
          </>
        ) : (
          <div className="empty-inline">
            <strong>No tunnel created yet</strong>
            <p>Save your keys and scope, then press Start on Dashboard. The app will create and remember the tunnel automatically.</p>
          </div>
        )}
      </section>
    </div>
  );
}
