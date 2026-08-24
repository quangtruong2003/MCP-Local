export type AppPage = "dashboard" | "workspaces" | "activity" | "security" | "settings";

interface Props {
  page: AppPage;
  onNavigate: (page: AppPage) => void;
  ready: boolean;
}

const items: Array<{ id: AppPage; label: string; glyph: string }> = [
  { id: "dashboard", label: "Dashboard", glyph: "⌂" },
  { id: "workspaces", label: "Workspaces", glyph: "▱" },
  { id: "activity", label: "Activity", glyph: "≋" },
  { id: "security", label: "Security", glyph: "◇" },
  { id: "settings", label: "Settings", glyph: "⚙" },
];

export default function Sidebar({ page, onNavigate, ready }: Props) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark">M</div>
        <div>
          <div className="brand-title">MCP Workspace</div>
          <div className="brand-subtitle">Local control center</div>
        </div>
      </div>

      <nav className="nav-list" aria-label="Primary navigation">
        {items.map((item) => (
          <button
            key={item.id}
            className={`nav-item ${page === item.id ? "active" : ""}`}
            onClick={() => onNavigate(item.id)}
          >
            <span className="nav-glyph" aria-hidden="true">{item.glyph}</span>
            <span>{item.label}</span>
          </button>
        ))}
      </nav>

      <div className="sidebar-footer">
        <span className={`status-dot ${ready ? "ready" : "offline"}`} />
        <span>{ready ? "Tunnel ready" : "Tunnel offline"}</span>
      </div>
    </aside>
  );
}
