import { useState } from "react";
import { api, parseErrorMessage } from "../api";

interface Props {
  onClose: () => void;
  onError: (message: string | null) => void;
}

/**
 * Note: the backend never reads the stored API key back to the UI (by
 * design), so fields always start empty. Saving requires both fields and
 * replaces the stored credentials entirely.
 */
export default function SettingsDialog({ onClose, onError }: Props) {
  const [tunnelId, setTunnelId] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [saving, setSaving] = useState(false);

  const canSave = tunnelId.trim().length > 0 && apiKey.trim().length > 0;

  const save = async () => {
    if (!canSave) return;
    onError(null);
    setSaving(true);
    try {
      await api.configureTunnel(tunnelId.trim(), apiKey.trim());
      onClose();
    } catch (err) {
      onError(parseErrorMessage(err));
      setSaving(false);
    }
  };

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(15,23,42,0.45)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-label="Tunnel settings"
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "var(--surface)",
          padding: 24,
          borderRadius: 10,
          minWidth: 400,
          maxWidth: "90vw",
          display: "flex",
          flexDirection: "column",
          gap: 12,
          boxShadow: "0 8px 30px rgba(0,0,0,0.18)",
        }}
      >
        <h3 style={{ margin: 0 }}>Tunnel settings</h3>
        <label style={field}>
          Tunnel ID
          <input value={tunnelId} onChange={(e) => setTunnelId(e.target.value)} style={input} autoFocus />
        </label>
        <label style={field}>
          API key
          <input type="password" value={apiKey} onChange={(e) => setApiKey(e.target.value)} style={input} />
        </label>
        <p style={{ margin: 0, fontSize: 12, color: "var(--text-muted)" }}>
          Both fields are required — saving replaces the stored credentials.
        </p>
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <button onClick={onClose}>Cancel</button>
          <button
            onClick={save}
            disabled={!canSave || saving}
            style={{ background: "var(--accent)", color: "#fff", borderColor: "var(--accent)" }}
          >
            {saving ? "Connecting…" : "Save & connect"}
          </button>
        </div>
      </div>
    </div>
  );
}

const field: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 4,
  fontWeight: 600,
};

const input: React.CSSProperties = {
  width: "100%",
  padding: "7px 9px",
  border: "1px solid var(--border)",
  borderRadius: 6,
  font: "inherit",
};
