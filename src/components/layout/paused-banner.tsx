import { PauseCircle } from "lucide-react";

export function PausedBanner({ paused }: { paused: boolean }) {
  if (!paused) return null;
  return (
    <div role="status" className="flex items-center gap-2 border-b bg-warning/15 px-6 py-2 text-[13px] text-warning">
      <PauseCircle aria-hidden="true" className="size-4" />
      <span>MCP access is paused. Connection may be online, but file tools are blocked.</span>
    </div>
  );
}
