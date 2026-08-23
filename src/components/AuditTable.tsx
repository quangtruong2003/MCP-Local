import { AuditEvent } from "../api";

const DECISION_COLOR: Record<string, string> = {
  allow: "var(--ok)",
  deny: "var(--danger)",
  error: "var(--warn)",
};

function fmtTime(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  return d.toLocaleTimeString();
}

export default function AuditTable({ events }: { events: AuditEvent[] }) {
  return (
    <section>
      <h2 style={{ margin: "0 0 8px", fontSize: 16 }}>Audit log (latest 100, newest first)</h2>
      <div
        style={{
          maxHeight: 260,
          overflowY: "auto",
          background: "var(--surface)",
          border: "1px solid var(--border)",
          borderRadius: 8,
        }}
      >
        <table
          style={{
            width: "100%",
            fontSize: 12,
            borderCollapse: "collapse",
          }}
        >
          <thead>
            <tr
              style={{
                position: "sticky",
                top: 0,
                background: "var(--surface)",
                textAlign: "left",
                borderBottom: "1px solid var(--border)",
              }}
            >
              <th style={th}>Time</th>
              <th style={th}>Source</th>
              <th style={th}>Op / tool</th>
              <th style={th}>Decision</th>
              <th style={th}>Path</th>
            </tr>
          </thead>
          <tbody>
            {events.map((e) => (
              <tr key={e.id} style={{ borderBottom: "1px solid var(--bg)" }}>
                <td style={{ ...td, whiteSpace: "nowrap" }}>{fmtTime(e.ts)}</td>
                <td style={td}>{e.source}</td>
                <td style={td}>{e.tool ?? e.op}</td>
                <td style={{ ...td, color: DECISION_COLOR[e.decision] ?? "var(--text)" }}>
                  {e.decision}
                  {e.error_code ? ` (${e.error_code})` : ""}
                </td>
                <td style={{ ...td, maxWidth: 320, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {e.path ?? ""}
                </td>
              </tr>
            ))}
            {events.length === 0 && (
              <tr>
                <td colSpan={5} style={{ ...td, color: "var(--text-muted)", fontStyle: "italic" }}>
                  No audit events yet.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}

const th: React.CSSProperties = {
  padding: "8px 10px",
  fontWeight: 600,
  position: "sticky",
  top: 0,
  background: "var(--surface)",
};

const td: React.CSSProperties = {
  padding: "5px 10px",
};
