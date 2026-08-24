interface Props {
  paused: boolean;
  busy: boolean;
  onTogglePause: () => void;
}

const protections = [
  ["Workspace confinement", "Every file operation is restricted to folders you explicitly approve."],
  ["Symlink & junction protection", "Canonical path checks prevent links from escaping an approved workspace."],
  ["Sensitive-file deny list", ".env, private keys, credentials and other protected paths are hidden and blocked."],
  ["Optimistic concurrency", "Edits require the exact SHA-256 revision returned by the last read."],
  ["Patch-first editing", "Existing files are changed through anchored patches instead of blind whole-file overwrites."],
  ["Append-only audit", "MCP decisions are recorded in an audit log that cannot be updated or deleted."],
] as const;

export default function SecurityPage({ paused, busy, onTogglePause }: Props) {
  return (
    <div className="stack-lg">
      <section className={`panel security-control ${paused ? "paused" : ""}`}>
        <div>
          <span className="eyebrow">Global access</span>
          <h2>{paused ? "MCP access is paused" : "MCP access is enabled"}</h2>
          <p>
            {paused
              ? "All MCP filesystem tool calls are denied before workspace lookup. Local manager controls remain available."
              : "ChatGPT can use only the workspaces and capabilities currently enabled below."}
          </p>
        </div>
        <button
          className={`btn ${paused ? "btn-primary" : "btn-danger"}`}
          onClick={onTogglePause}
          disabled={busy}
        >
          {paused ? "Resume access" : "Pause all access"}
        </button>
      </section>

      <section className="panel">
        <div className="section-heading">
          <div>
            <span className="eyebrow">Protection layer</span>
            <h2>Security controls</h2>
          </div>
          <span className="badge badge-success">Enforced</span>
        </div>
        <div className="protection-list">
          {protections.map(([title, description]) => (
            <div className="protection-item" key={title}>
              <div className="protection-check">✓</div>
              <div>
                <strong>{title}</strong>
                <p>{description}</p>
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="section-heading compact">
          <div>
            <span className="eyebrow">Always blocked</span>
            <h2>Sensitive paths</h2>
          </div>
        </div>
        <div className="token-list" aria-label="Blocked file patterns">
          {[".env*", "*.pem", "*.key", "id_rsa*", ".git/config", "credentials*", "*.pfx", "*.p12", "secrets.*", "node_modules/**"].map((item) => (
            <code className="token" key={item}>{item}</code>
          ))}
        </div>
      </section>
    </div>
  );
}
