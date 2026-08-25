import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

interface Props {
  icon?: LucideIcon;
  title: string;
  message: string;
  action?: ReactNode;
}

export function EmptyState({ icon: Icon, title, message, action }: Props) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 rounded-md border border-dashed px-6 py-12 text-center">
      {Icon ? (
        <div className="grid size-12 place-items-center rounded-md bg-muted">
          <Icon aria-hidden="true" className="size-6 text-muted-foreground" />
        </div>
      ) : null}
      <div className="font-medium">{title}</div>
      <p className="max-w-sm text-sm text-muted-foreground">{message}</p>
      {action}
    </div>
  );
}
