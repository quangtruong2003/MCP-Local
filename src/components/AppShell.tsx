import type { ReactNode } from "react";
import Sidebar, { AppPage } from "./Sidebar";

interface Props {
  page: AppPage;
  onNavigate: (page: AppPage) => void;
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
          <div className={`connection-chip ${ready ? "ready" : "offline"}`}>
            <span className={`status-dot ${ready ? "ready" : "offline"}`} />
            {ready ? "Ready" : "Offline"}
          </div>
        </header>
        <div className="page-content">{children}</div>
      </main>
    </div>
  );
}
