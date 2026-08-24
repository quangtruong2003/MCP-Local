import { useEffect, useRef } from "react";
import type { AuditEvent } from "../api";

/**
 * Detail view for one audit event (plan Task 19 Step 4): renders the SAFE
 * structured metadata the backend stored (counts, flags, hashes) as pretty
 * JSON. The audit contract never stores file contents, script bodies, or
 * command output, so there is nothing unsafe to render here.
 */

const EXEC_FIELDS: ReadonlyArray<{ key: string; label: string }> = [
  { key: "program", label: "Program" },
  { key: "script_bytes", label: "Script size (bytes)" },
  { key: "script_sha256", label: "Script SHA-256" },
  { key: "cwd", label: "Working directory" },
  { key: "exit_code", label: "Exit code" },
  { key: "duration_ms", label: "Duration (ms)" },
  { key: "timed_out", label: "Timed out" },
  { key: "stdout_truncated", label: "Stdout truncated" },
  { key: "stderr_truncated", label: "Stderr truncated" },
];

interface Props {
  event: AuditEvent | null;
  onClose: () => void;
}

export default function ActivityDetail({ event, onClose }: Props) {
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!event) return;
    closeRef.current?.focus();
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [event, onClose]);

  if (!event) return null;

  const detail = event.detail;
  const execRows =
    detail !== null && typeof detail === "object" && !Array.isArray(detail)
      ? EXEC_FIELDS.filter((field) => field.key in (detail as Record<string, unknown>))
      : [];

  return (
    <div
      className="dialog-backdrop"
      role="presentation"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="dialog-card activity-detail-card" role="dialog" aria-modal="true" aria-labelledby="activity-detail-title">
        <div className="activity-detail-head">
          <h2 id="activity-detail-title">{event.op.replaceAll("_", " ")}</h2>
          <button ref={closeRef} className="icon-button" aria-label="Close details" onClick={onClose}>
            ×
          </button>
        </div>

        <p className="activity-detail-meta">
          <span>Source: {event.source}</span>
          {event.path ? <span>Path: <code>{event.path}</code></span> : null}
          {event.error_code ? <span>Error: {event.error_code}</span> : null}
        </p>

        {execRows.length > 0 ? (
          <dl className="activity-exec-fields">
            {execRows.map(({ key, label }) => (
              <div key={key} className="activity-exec-field">
                <dt>{label}</dt>
                <dd>
                  <code>{String((detail as Record<string, unknown>)[key])}</code>
                </dd>
              </div>
            ))}
          </dl>
        ) : null}

        {detail !== null && detail !== undefined ? (
          <>
            <h3 className="activity-detail-subhead">Recorded metadata</h3>
            <pre className="activity-json" data-testid="activity-json">
              {JSON.stringify(detail, null, 2)}
            </pre>
          </>
        ) : (
          <p className="activity-no-detail">No extra metadata was recorded for this event.</p>
        )}
      </div>
    </div>
  );
}
