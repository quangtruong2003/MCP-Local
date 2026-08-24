import type { ReactNode } from "react";
import Sidebar from "./Sidebar";
import type { ViewId } from "../types";

interface Props {
  page: ViewId;
  onNavigate: (page: ViewId) => void;
  ready: boolean;
  title: string;
  subtitle: string;
  children: ReactNode;
}

export default function AppShell({ page, onNavigate, ready, title, subtitle, children }: Props) {
  return (
    <div className="app-shell">
      <Sidebar page={page} onNavigate={onNavigate} ready={ready} />
      <main className="main-area">
        <header className="page-header">
          <div>
            <h1>{title}</h1>
            <p>{subtitle}</p>
          </div>
          <div className="connection-chip">
            {ready ? "Connected" : "Offline"}
          </div>
        </header>
        <div className="page-content">{children}</div>
      </main>
    </div>
  );
}
