import type { AuditEvent, Workspace } from "../api";

function fmtTime(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  return d.toLocaleString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function workspaceName(event: AuditEvent, workspaces: Workspace[]): string {
  if (!event.workspace_id) return "Global";
  return workspaces.find((w) => w.id === event.workspace_id)?.name ?? "Removed workspace";
}

export default function ActivityPage({ events, workspaces }: { events: AuditEvent[]; workspaces: Workspace[] }) {
  return (
    <section className="panel activity-panel">
      <div className="section-heading">
        <div>
          <span className="eyebrow">Latest 100 events</span>
          <h2>Recent activity</h2>
        </div>
        <span className="muted">Newest first</span>
      </div>

      {events.length === 0 ? (
        <div className="empty-state compact-empty">
          <div className="empty-icon">≋</div>
          <strong>No activity yet</strong>
          <p>MCP requests and manager changes will appear here.</p>
        </div>
      ) : (
        <div className="activity-list">
          {events.map((event) => (
            <article className="activity-item" key={event.id}>
              <div className={`activity-marker ${event.decision}`} />
              <div className="activity-body">
                <div className="activity-title-row">
                  <div>
                    <strong>{event.tool ?? event.op}</strong>
                    <span className={`badge badge-${event.decision}`}>{event.decision}</span>
                  </div>
                  <time>{fmtTime(event.ts)}</time>
                </div>
                <div className="activity-meta">
                  <span>{workspaceName(event, workspaces)}</span>
                  <span>·</span>
                  <span>{event.source.toUpperCase()}</span>
                  {event.error_code ? <><span>·</span><code>{event.error_code}</code></> : null}
                </div>
                {event.path ? <code className="activity-path">{event.path}</code> : null}
              </div>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}
