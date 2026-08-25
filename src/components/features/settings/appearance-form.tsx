import { Monitor, Moon, Sun } from "lucide-react";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { useTheme } from "@/hooks/use-theme";
import type { ThemePreference } from "@/lib/types";
import type { LucideIcon } from "lucide-react";

const OPTIONS: Array<{ value: ThemePreference; label: string; icon: LucideIcon }> = [
  { value: "system", label: "System", icon: Monitor },
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
];

/** Pure control surface: persistence flows through ThemeProvider → App →
 * updateAppSettings. */
export function AppearanceForm() {
  const { theme, resolvedTheme, setTheme } = useTheme();
  return (
    <div className="flex flex-col gap-2">
      <RadioGroup value={theme} onValueChange={(v) => setTheme(v as ThemePreference)} className="flex gap-3">
        {OPTIONS.map(({ value, label, icon: Icon }) => (
          <Label
            key={value}
            className="flex flex-1 cursor-pointer items-center gap-3 rounded-md border p-3 font-normal has-[[data-state=checked]]:border-primary"
          >
            <RadioGroupItem value={value} />
            <Icon aria-hidden="true" className="size-4" />
            {label}
          </Label>
        ))}
      </RadioGroup>
      <p className="text-[13px] text-muted-foreground">Currently rendering in {resolvedTheme} mode.</p>
    </div>
  );
}
