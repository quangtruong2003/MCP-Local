import {
  LayoutDashboard,
  FolderOpen,
  ScrollText,
  ShieldCheck,
  Settings as SettingsIcon,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import StatusBadge from "./StatusBadge";
import type { ViewId } from "../types";

interface Props {
  page: ViewId;
  onNavigate: (page: ViewId) => void;
  ready: boolean;
}

const ITEMS: Array<{ id: ViewId; label: string; icon: LucideIcon }> = [
  { id: "dashboard", label: "Dashboard", icon: LayoutDashboard },
  { id: "workspaces", label: "Workspaces", icon: FolderOpen },
  { id: "activity", label: "Activity", icon: ScrollText },
  { id: "security", label: "Security", icon: ShieldCheck },
  { id: "settings", label: "Settings", icon: SettingsIcon },
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
        {ITEMS.map((item) => (
          <button
            key={item.id}
            className={`nav-item ${page === item.id ? "active" : ""}`}
            onClick={() => onNavigate(item.id)}
            aria-current={page === item.id ? "page" : undefined}
          >
            <item.icon aria-hidden="true" />
            <span>{item.label}</span>
          </button>
        ))}
      </nav>

      <div className="sidebar-footer">
        <StatusBadge
          tone={ready ? "ready" : "offline"}
          text={ready ? "Tunnel ready" : "Tunnel offline"}
        />
      </div>
    </aside>
  );
}
