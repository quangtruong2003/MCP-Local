import { useState } from "react";
import { ChevronRight, FolderOpen, ShieldCheck } from "lucide-react";
import { ConnectionCard } from "@/components/features/dashboard/connection-card";
import { ServerInfoCard } from "@/components/features/dashboard/server-info-card";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/shared/empty-state";
import { api, parseErrorMessage } from "../api";
import type { AuditEvent, ServerStatus } from "../api";
import type { ConnectionStatus, ViewId, Workspace } from "@/lib/types";

interface Props {
  connection: ConnectionStatus | null;
  workspaces: Workspace[];
  auditEvents: AuditEvent[];
  server: ServerStatus | null;
  onNavigate: (page: ViewId) => void;
  reportError: (message: string) => void;
}

export default function DashboardPage({
  connection,
  workspaces,
  auditEvents,
  server,
  onNavigate,
  reportError,
}: Props) {
  // One in-flight lifecycle request at a time; buttons disable while true.
  const [pending, setPending] = useState(false);

  async function runLifecycle(action: () => Promise<unknown>) {
    if (pending) return;
    setPending(true);
    try {
      await action();
      // The live `connection-status` event drives the UI from here.
    } catch (err) {
      reportError(
        typeof err === "object" && err !== null && "message" in err
          ? String((err as { message: unknown }).message)
          : String(err),
      );
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      {!connection ? (
        <EmptyState title="No connection data" message="Waiting for the local backend…" />
      ) : (
        <ConnectionCard
          state={connection.state}
          paused={connection.paused}
          pending={pending}
          onStart={() => void runLifecycle(api.startConnection)}
          onStop={() => void runLifecycle(api.stopConnection)}
          onRetry={() => void runLifecycle(api.retryConnection)}
          onSignIn={() => {
            if (connection.state.state !== "needs_login") return;
            void api
              .openExternalUrl(connection.state.login_url)
              .catch((err) => reportError(parseErrorMessage(err)));
          }}
          onOpenAdmin={(url) => {
            void api.openExternalUrl(url).catch((err) => reportError(parseErrorMessage(err)));
          }}
        />
      )}

      <ServerInfoCard server={server} auditEvents={auditEvents} onNavigate={onNavigate} />

      <div className="grid gap-3 sm:grid-cols-2">
        <QuickLink
          icon={FolderOpen}
          title="Workspaces"
          description={
            workspaces.length === 0
              ? "Approve folders ChatGPT may reach."
              : `${workspaces.length} folder${workspaces.length === 1 ? "" : "s"} approved.`
          }
          onClick={() => onNavigate("workspaces")}
        />
        <QuickLink
          icon={ShieldCheck}
          title="Security"
          description="Pause access, rotate the URL or review safeguards."
          onClick={() => onNavigate("security")}
        />
      </div>
    </div>
  );
}

function QuickLink({
  icon: Icon,
  title,
  description,
  onClick,
}: {
  icon: typeof FolderOpen;
  title: string;
  description: string;
  onClick: () => void;
}) {
  return (
    <Card className="flex items-center justify-between gap-3 p-4">
      <div className="flex items-start gap-3">
        <div className="grid size-9 shrink-0 place-items-center rounded-md bg-muted">
          <Icon aria-hidden="true" className="size-4" />
        </div>
        <div>
          <div className="text-sm font-medium">{title}</div>
          <div className="text-[13px] text-muted-foreground">{description}</div>
        </div>
      </div>
      <Button variant="ghost" size="icon" aria-label={`Open ${title}`} onClick={onClick}>
        <ChevronRight aria-hidden="true" />
      </Button>
    </Card>
  );
}
