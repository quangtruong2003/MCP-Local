import { useEffect, useState } from "react";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import type { AppSettings } from "@/lib/types";

interface Props {
  settings: AppSettings | null;
  /** Persists via api.updateAppSettings; throws propagate for rollback. */
  onSaveSettings: (next: AppSettings) => Promise<void>;
  reportError: (message: string) => void;
}

interface Flags {
  start_on_launch: boolean;
  launch_on_startup: boolean;
  close_to_tray: boolean;
}

const ROWS: ReadonlyArray<{
  key: keyof Flags;
  id: string;
  label: string;
  description: string;
}> = [
  {
    key: "start_on_launch",
    id: "setting-start-tunnel",
    label: "Auto-start tunnel on app launch",
    description: "Starts the local MCP server and Tailscale Funnel connection as soon as the app opens.",
  },
  {
    key: "launch_on_startup",
    id: "setting-launch-startup",
    label: "Start with Windows",
    description: "Registers the app to launch automatically when you sign in to Windows.",
  },
  {
    key: "close_to_tray",
    id: "setting-close-tray",
    label: "Keep running in the system tray when closed",
    description:
      "Closing the window hides it to the tray and keeps the MCP server reachable. Turn off to quit the app instead.",
  },
];

export function GeneralForm({ settings, onSaveSettings, reportError }: Props) {
  const [flags, setFlags] = useState<Flags>({
    start_on_launch: false,
    launch_on_startup: false,
    close_to_tray: true,
  });

  // Re-seed local toggles whenever App loads/replaces settings.
  useEffect(() => {
    if (!settings) return;
    setFlags({
      start_on_launch: settings.start_on_launch,
      launch_on_startup: settings.launch_on_startup,
      close_to_tray: settings.close_to_tray,
    });
  }, [settings]);

  async function toggle(key: keyof Flags) {
    if (!settings) return;
    const previous = flags[key];
    const next = !previous;
    setFlags((current) => ({ ...current, [key]: next }));
    try {
      await onSaveSettings({ ...settings, [key]: next });
    } catch (err) {
      reportError(
        typeof err === "object" && err !== null && "message" in err
          ? String((err as { message: unknown }).message)
          : String(err),
      );
      setFlags((current) => ({ ...current, [key]: previous })); // roll back
    }
  }

  return (
    <div role="group" aria-label="General behavior" className="flex flex-col gap-4">
      {ROWS.map((row) => (
        <div key={row.key} className="flex items-center justify-between gap-4">
          <Label htmlFor={row.id} className="flex-col items-start gap-0.5 font-normal">
            <span className="text-sm font-medium">{row.label}</span>
            <span className="text-[13px] text-muted-foreground">{row.description}</span>
          </Label>
          <Switch
            id={row.id}
            checked={flags[row.key]}
            disabled={!settings}
            onCheckedChange={() => void toggle(row.key)}
          />
        </div>
      ))}
    </div>
  );
}
