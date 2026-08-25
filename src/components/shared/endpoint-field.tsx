import { AlertTriangle, ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/shared/copy-button";

interface Props {
  url: string;
  label?: string;
  /** Amber caution line rendered under the field. */
  hint?: string;
  /** When provided, an external-link button opens the URL in the browser. */
  onOpen?: () => void;
}

/**
 * The secret-capability-URL field: label + actions on one row, the full URL
 * wrapped (never truncated — it is the value the user must copy verbatim),
 * and an optional caution line. Shared by Dashboard, Onboarding and Security.
 */
export function EndpointField({ url, label = "Public MCP endpoint", hint, onOpen }: Props) {
  return (
    <div className="flex flex-col gap-2 rounded-md border bg-muted/40 p-3">
      <div className="flex items-center justify-between gap-2">
        <span className="text-[13px] font-medium">{label}</span>
        <div className="flex items-center gap-0.5">
          <CopyButton value={url} label="Copy endpoint URL" />
          {onOpen ? (
            <Button variant="ghost" size="icon" aria-label="Open endpoint in browser" onClick={onOpen}>
              <ExternalLink aria-hidden="true" />
            </Button>
          ) : null}
        </div>
      </div>
      <code className="block break-all rounded-sm border bg-background px-2 py-1.5 font-mono text-xs select-all">
        {url}
      </code>
      {hint ? (
        <p className="flex items-start gap-1.5 text-[13px] text-warning">
          <AlertTriangle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
          {hint}
        </p>
      ) : null}
    </div>
  );
}
