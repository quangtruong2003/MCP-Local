import type { LucideIcon } from "lucide-react";
import { Card } from "@/components/ui/card";

interface Props {
  label: string;
  value: string;
  hint?: string;
  icon?: LucideIcon;
}

export function StatCard({ label, value, hint, icon: Icon }: Props) {
  return (
    <Card className="gap-1 p-4">
      <div className="flex items-start justify-between gap-2">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{label}</span>
        {Icon ? <Icon aria-hidden="true" className="size-4 text-muted-foreground" /> : null}
      </div>
      <div className="text-2xl font-semibold tabular-nums">{value}</div>
      {hint ? <div className="text-[13px] text-muted-foreground">{hint}</div> : null}
    </Card>
  );
}
