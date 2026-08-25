import { ShieldAlert } from "lucide-react";

/** Global danger strip shown while unrestricted MCP access is on. */
export function UnrestrictedBanner({ active }: { active: boolean }) {
  if (!active) return null;
  return (
    <div
      role="alert"
      className="flex items-center gap-2 border-b bg-destructive/15 px-6 py-2 text-[13px] font-medium text-destructive"
    >
      <ShieldAlert aria-hidden="true" className="size-4 shrink-0" />
      <span>
        Unrestricted access is ON — capability gates, the sensitive-file deny list, the
        approved-command list, the Full-PowerShell gate and execution bounds are bypassed. Pause
        remains the kill switch.
      </span>
    </div>
  );
}
