import { TerminalSquare } from "lucide-react";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { cn } from "@/lib/utils";
import type { ExecutionMode } from "@/lib/types";

interface Props {
  value: ExecutionMode;
  disabled: boolean;
  onChange: (mode: ExecutionMode) => void;
}

/** Two adjacent segmented options; selection semantics come from RadioGroup. */
export function ExecutionModePicker({ value, disabled, onChange }: Props) {
  return (
    <RadioGroup
      value={value}
      onValueChange={(v) => onChange(v as ExecutionMode)}
      disabled={disabled}
      className="grid grid-cols-1 gap-2 sm:grid-cols-2"
    >
      <Label
        className={cn(
          "flex cursor-pointer items-center gap-3 rounded-md border p-3 font-normal",
          "has-[[data-state=checked]]:border-primary",
        )}
      >
        <RadioGroupItem value="approved_commands" />
        <span className="text-sm">Approved commands</span>
      </Label>
      <Label
        className={cn(
          "flex cursor-pointer items-center gap-3 rounded-md border p-3 font-normal",
          "has-[[data-state=checked]]:border-primary",
        )}
      >
        <RadioGroupItem value="full_powershell" />
        <TerminalSquare aria-hidden="true" className="size-4" />
        <span className="text-sm">Full PowerShell</span>
      </Label>
    </RadioGroup>
  );
}
