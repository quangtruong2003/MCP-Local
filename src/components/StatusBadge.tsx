import { CheckCircle2, CircleSlash, AlertTriangle, XOctagon } from "lucide-react";
import type { LucideIcon } from "lucide-react";

export type StatusTone = "ready" | "offline" | "warning" | "error";

const TONE_META: Record<StatusTone, { icon: LucideIcon; label: string }> = {
  ready: { icon: CheckCircle2, label: "Ready" },
  offline: { icon: CircleSlash, label: "Offline" },
  warning: { icon: AlertTriangle, label: "Warning" },
  error: { icon: XOctagon, label: "Error" },
};

interface Props {
  tone: StatusTone;
  /** Visible text — status is never conveyed by color alone. */
  text?: string;
}

export default function StatusBadge({ tone, text }: Props) {
  const meta = TONE_META[tone];
  const Icon = meta.icon;
  return (
    <span className={`status-badge status-${tone}`}>
      <Icon aria-hidden="true" />
      <span>{text ?? meta.label}</span>
    </span>
  );
}
