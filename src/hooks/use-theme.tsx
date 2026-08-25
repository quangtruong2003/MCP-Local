import { createContext, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { ThemePreference } from "@/lib/types";

interface ThemeContextValue {
  theme: ThemePreference;
  resolvedTheme: "light" | "dark";
  setTheme: (t: ThemePreference) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function systemPrefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export function ThemeProvider({ initialTheme, onThemeChange, children }: {
  initialTheme: ThemePreference;
  onThemeChange: (t: ThemePreference) => void;
  children: ReactNode;
}) {
  const [theme, setThemeState] = useState<ThemePreference>(initialTheme);
  // Settings load asynchronously: App mounts the provider with "system" and
  // later learns the persisted preference. Re-seed on that transition unless
  // the user already picked a theme in this session (their pick round-trips
  // through settings, so an equal value is a no-op).
  const seededInitial = useRef(initialTheme);
  useEffect(() => {
    if (initialTheme !== seededInitial.current) {
      seededInitial.current = initialTheme;
      setThemeState(initialTheme);
    }
  }, [initialTheme]);

  // Apply/remove the class and track OS changes while in System mode.
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = theme === "dark" || (theme === "system" && media.matches);
      document.documentElement.classList.toggle("dark", dark);
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  const value = useMemo<ThemeContextValue>(() => ({
    theme,
    resolvedTheme: theme === "dark" || (theme === "system" && systemPrefersDark()) ? "dark" : "light",
    setTheme: (next) => { setThemeState(next); onThemeChange(next); },
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }), [theme, onThemeChange]);

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used within ThemeProvider");
  return ctx;
}
