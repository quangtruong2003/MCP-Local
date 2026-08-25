import { FolderOpen, LayoutDashboard, ScrollText, Settings as SettingsIcon, ShieldCheck } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { ViewId } from "@/lib/types";

const ITEMS: Array<{ id: ViewId; label: string; icon: LucideIcon }> = [
  { id: "dashboard", label: "Dashboard", icon: LayoutDashboard },
  { id: "workspaces", label: "Workspaces", icon: FolderOpen },
  { id: "activity", label: "Activity", icon: ScrollText },
  { id: "security", label: "Security", icon: ShieldCheck },
  { id: "settings", label: "Settings", icon: SettingsIcon },
];

function NavItems({ page, onNavigate }: { page: ViewId; onNavigate: (p: ViewId) => void }) {
  return (
    <nav aria-label="Primary navigation" className="flex flex-col gap-1">
      {ITEMS.map((item) => (
        <Button
          key={item.id}
          variant={page === item.id ? "secondary" : "ghost"}
          onClick={() => onNavigate(item.id)}
          aria-current={page === item.id ? "page" : undefined}
          className="w-full justify-start gap-2.5 px-2.5 font-medium max-lg:justify-center max-lg:px-0"
        >
          <item.icon aria-hidden="true" className="size-[17px] shrink-0" />
          <span className="max-lg:hidden">{item.label}</span>
        </Button>
      ))}
    </nav>
  );
}

function Brand() {
  return (
    <div className="flex items-center gap-2.5 px-2 pb-4">
      <div className="grid size-9 place-items-center rounded-md bg-primary font-semibold text-primary-foreground">M</div>
      <div className="max-lg:hidden">
        <div className="text-sm font-semibold leading-tight">MCP Workspace</div>
        <div className="text-xs text-muted-foreground">Local control center</div>
      </div>
    </div>
  );
}

function TunnelFooter({ ready }: { ready: boolean }) {
  return (
    <div className="mt-auto pt-2">
      <Badge variant={ready ? "outline" : "secondary"} className="gap-1.5">
        <span aria-hidden="true" className={cn("size-2 rounded-full", ready ? "bg-success" : "bg-destructive")} />
        <span className="max-lg:hidden">{ready ? "Tunnel ready" : "Tunnel offline"}</span>
      </Badge>
    </div>
  );
}

export { NavItems, Brand, TunnelFooter, ITEMS };
export default NavItems;
