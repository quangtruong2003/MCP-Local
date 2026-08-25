import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { ConnectionState } from "@/lib/types";

const LABELS: Record<ConnectionState["state"], string> = {
  stopped: "Stopped",
  starting_server: "Starting…",
  starting_tunnel: "Starting tunnel…",
  needs_login: "Login required",
  needs_funnel_permission: "Funnel permission needed",
  needs_https_setup: "HTTPS setup needed",
  verifying: "Verifying…",
  online: "Online",
  degraded: "Degraded",
  stopping: "Stopping…",
  error: "Error",
};

type Tone = "success" | "warning" | "destructive" | "secondary";
const TONE: Record<ConnectionState["state"], Tone> = {
  online: "success",
  verifying: "warning",
  starting_server: "secondary",
  starting_tunnel: "secondary",
  stopping: "secondary",
  needs_login: "warning",
  needs_funnel_permission: "warning",
  needs_https_setup: "warning",
  degraded: "warning",
  stopped: "secondary",
  error: "destructive",
};

/** Color meaning comes from the dot; the chip itself stays outline — matches
 * Quiet Console restraint. */
export function StatusBadge({ state, paused }: { state: ConnectionState | null; paused?: boolean }) {
  if (!state) return <Badge variant="secondary">Unknown</Badge>;
  const tone = TONE[state.state];
  return (
    <Badge variant="outline" className="gap-1.5">
      <span
        aria-hidden="true"
        className={cn(
          "size-2 rounded-full",
          tone === "success" && !paused ? "bg-success"
          : tone === "destructive" ? "bg-destructive"
          : tone === "warning" ? "bg-warning"
          : "bg-muted-foreground",
        )}
      />
      {paused ? "Paused" : LABELS[state.state]}
    </Badge>
  );
}
