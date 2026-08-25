import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

interface Props {
  value: string;
  label?: string;
}

export function CopyButton({ value, label = "Copy" }: Props) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard denial is surfaced by the caller when it matters; a copy
      // affordance failing silently on a read-only row is acceptable.
    }
  }

  return (
    <TooltipProvider>
      <Tooltip>
        {/* Native <button> + buttonVariants: Radix asChild needs a ref, which
            React 18 only delivers to host elements. */}
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label={label}
            onClick={() => void copy()}
            className={cn(buttonVariants({ variant: "ghost", size: "icon" }))}
          >
            {copied ? <Check aria-hidden="true" className="text-success" /> : <Copy aria-hidden="true" />}
          </button>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
