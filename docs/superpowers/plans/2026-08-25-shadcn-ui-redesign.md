# shadcn/ui Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the vanilla-CSS UI layer with shadcn/ui + Tailwind v4 across all 5 views, adding a persisted Light/Dark/System theme setting, per spec `docs/superpowers/specs/2026-08-25-shadcn-ui-redesign-design.md`.

**Architecture:** Big-bang rewrite sequenced into ordered commits on branch `feat/tailscale-funnel`: foundation (Tailwind v4 CSS-first tokens + vendored shadcn primitives + custom ThemeProvider backed by `AppSettings.theme`), then per-view waves rebuilding pages thin over feature components, ending with deletion of all legacy CSS. Backend touch is limited to one additive serde-defaulted settings field.

**Tech Stack:** Tauri 2 · React 18 · TypeScript strict · Vite 5 · Tailwind CSS v4 (`@tailwindcss/vite`) · shadcn/ui (new-york/zinc) · Radix primitives · lucide-react · sonner · @fontsource-variable Inter/JetBrains Mono

**Spec:** `docs/superpowers/specs/2026-08-25-shadcn-ui-redesign-design.md`

## Global Constraints

- Radius: `--radius: 0.125rem` (2px); derived scale pinned `--radius-sm:1px --radius-md:2px --radius-lg:2px --radius-xl:3px`. Switch/radio-dot keep pill geometry (vendor default).
- Colors: primary light `#4f46e5` (fg white) / dark `#818cf8` (fg near-black); success `#15803d`/`#4ade80`; warning `#b45309`/`#fbbf24`; base = stock shadcn zinc. No hard-coded colors outside `src/styles/globals.css`.
- Fonts: `@fontsource-variable/inter`, `@fontsource-variable/jetbrains-mono` — bundled only, never CDN. Type floor 13px table/meta, 14px body.
- Dark mode: `.dark` class on `<html>`, `@custom-variant dark (&:is(.dark *))`; System mode tracks `matchMedia("(prefers-color-scheme)")`.
- Navigation stays state-based `ViewId`; no router/state-library/new UI kit.
- IPC (`invoke`) only inside `src/lib/api.ts`; components never import it.
- Security toggles stay pessimistic: await backend result before reflecting UI state (preserve `busyId` pattern).
- `components/ui/*` is vendored shadcn output — do not modify except where a task explicitly says so.
- New frontend files use kebab-case. Match existing repo commit style (`feat:`/`refactor:`/`chore:` prefixes).
- Verification commands: frontend gate `npx tsc --noEmit` then `npm run build`; Rust gate `export PATH="$HOME/.cargo/bin:$PATH" && cargo check` (repo-root workspace includes `crates/core` + `src-tauri`). No frontend test framework exists or is added; manual smoke lists below are the acceptance bar.

---

### Task 1: P0 — Delete dead components and deprecated Cloudflare types

**Files:**
- Delete: `src/components/ActivityPage.tsx`, `src/components/SecurityPage.tsx`, `src/components/SettingsDialog.tsx`, `src/components/ConnectionHero.tsx`, `src/components/StatusBar.tsx`, `src/components/PauseButton.tsx`, `src/components/AuditTable.tsx`, `src/components/WorkspaceList.tsx`
- Modify: `src/api.ts` (remove deprecated type block, lines ~15–49)

**Interfaces:**
- Produces: cleaner baseline; nothing may reference the deleted symbols afterwards.

- [ ] **Step 1: Prove the dead code is unreferenced**

```bash
grep -rn "SettingsDialog\|ConnectionHero\|StatusBar\|PauseButton\|AuditTable\|WorkspaceList\|ConnectionSnapshot\|ConnectionSettingsSummary\|SaveConnectionSettingsInput" src/
grep -rn "from \"../components/ActivityPage\"\|from \"../components/SecurityPage\"" src/
```
Expected: hits only inside the files being deleted and `src/api.ts` itself. If any other hit appears, STOP and reconcile before deleting.

- [ ] **Step 2: Delete the 8 component files**

```bash
git rm src/components/ActivityPage.tsx src/components/SecurityPage.tsx src/components/SettingsDialog.tsx src/components/ConnectionHero.tsx src/components/StatusBar.tsx src/components/PauseButton.tsx src/components/AuditTable.tsx src/components/WorkspaceList.tsx
```

- [ ] **Step 3: Remove deprecated block from `src/api.ts`**

Delete the comment banner and these three interfaces entirely: `ConnectionSnapshot`, `ConnectionSettingsSummary`, `SaveConnectionSettingsInput` (everything between the `// --- Deprecated compat types …` comment and the `AuditEvent` interface).

- [ ] **Step 4: Verify**

Run: `npx tsc --noEmit && npm run build`
Expected: both succeed with zero errors.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "refactor: remove dead Cloudflare-era components and deprecated api types"
```

---

### Task 2: P1 — Tailwind v4 install, path aliases, globals.css scaffold

**Files:**
- Modify: `vite.config.ts`, `tsconfig.json`, `src/main.tsx`
- Create: `src/styles/globals.css`

**Interfaces:**
- Produces: `"@/*"` import alias usable everywhere; `globals.css` containing Tailwind entry + dark variant declaration (token values land in Task 4).

- [ ] **Step 1: Install deps**

```bash
npm i -D tailwindcss @tailwindcss/vite @types/node
```

- [ ] **Step 2: Configure Vite (full replacement of `vite.config.ts`)**

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
});
```

- [ ] **Step 3: Add TS paths (`tsconfig.json`)**

Add inside `compilerOptions`: `"baseUrl": "."` and `"paths": { "@/*": ["./src/*"] }`. Keep everything else untouched.

- [ ] **Step 4: Create `src/styles/globals.css`**

```css
@import "tailwindcss";

@custom-variant dark (&:is(.dark *));
```

- [ ] **Step 5: Order CSS imports in `src/main.tsx`**

Replace lines 4–6 so Tailwind loads FIRST and legacy sheets AFTER (legacy element rules keep winning over preflight during the transition):

```ts
import "./styles/globals.css";
import "./styles/tokens.css";
import "./styles/global.css";
import "./styles/components.css";
```

- [ ] **Step 6: Verify**

Run: `npx tsc --noEmit && npm run build` — expect success. Launch `npm run dev`, open app: visuals must be unchanged (Tailwind emits only preflight at this point; any visible regression means an import-order mistake).

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: add tailwind v4 pipeline and @ path alias"
```

---

### Task 3: P1 — shadcn init + vendored primitives

**Files:**
- Modify: `src/styles/globals.css` (init writes zinc tokens), `package.json`, `package-lock.json`
- Create: `src/lib/utils.ts` (+ `src/components/ui/*.tsx`)

**Interfaces:**
- Produces: `cn()` from `"@/lib/utils"`; ui primitives at `@/components/ui/<name>` used by every later task.

- [ ] **Step 1: Init shadcn (answer prompts exactly)**

```bash
npx shadcn@latest init
```
Answers: TypeScript ✓ · default style (new-york) · base color **zinc** · CSS variables **yes** · CSS file `src/styles/globals.css`. If prompted about React 19 peer deps / `--force` semantics with React 18, choose the conservative option (no force).

- [ ] **Step 2: Add the primitive set**

```bash
npx shadcn@latest add button badge card dialog alert-dialog switch radio-group input select label separator table sheet tooltip dropdown-menu
```
(sonner intentionally excluded — Task 8 mounts the `sonner` package directly so theming comes from our own provider instead of next-themes.)

- [ ] **Step 3: Verify vendored output**

Check `src/components/ui/` contains exactly the added primitives and `src/lib/utils.ts` exports `cn`. Run: `npx tsc --noEmit && npm run build` — expect success.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: vendor shadcn/ui primitives on zinc base"
```

---

### Task 4: P1 — Pin design tokens (radius 2px, indigo primary, success/warning, fonts)

**Files:**
- Modify: `src/styles/globals.css` (edit the init-generated `:root`, `.dark`, `@theme inline`)
- Modify: `src/main.tsx` (font imports), `package.json`

**Interfaces:**
- Produces: final token contract every component relies on — utilities `bg-background text-foreground border-border ring-ring bg-primary bg-success bg-warning font-mono rounded-sm/md/lg/xl`.

- [ ] **Step 1: Install fonts**

```bash
npm i @fontsource-variable/inter @fontsource-variable/jetbrains-mono
```
In `src/main.tsx`, directly under the CSS imports:
```ts
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
```

- [ ] **Step 2: Edit tokens in `src/styles/globals.css`**

Inside `:root` (light) set/override:
```css
  --radius: 0.125rem;
  --radius-sm: calc(var(--radius) - 1px);
  --radius-md: var(--radius);
  --radius-lg: var(--radius);
  --radius-xl: calc(var(--radius) + 1px);
  --primary: #4f46e5;
  --primary-foreground: #ffffff;
  --success: #15803d;
  --success-foreground: #ffffff;
  --warning: #b45309;
  --warning-foreground: #ffffff;
```
Inside `.dark` set/override:
```css
  --primary: #818cf8;
  --primary-foreground: #09090b;
  --success: #4ade80;
  --success-foreground: #09090b;
  --warning: #fbbf24;
  --warning-foreground: #09090b;
```
(Leave all other zinc values exactly as init generated them.)

In `@theme inline` append:
```css
  --color-success: var(--success);
  --color-success-foreground: var(--success-foreground);
  --color-warning: var(--warning);
  --color-warning-foreground: var(--warning-foreground);
  --font-sans: "Inter Variable", ui-sans-serif, system-ui, sans-serif;
  --font-mono: "JetBrains Mono Variable", ui-monospace, monospace;
```

- [ ] **Step 3: Smoke-check tokens render**

Temporarily add `<div className="bg-primary text-primary-foreground p-2">T</div>` to `App.tsx` root, run `npm run dev`: button-blue tile in light mode, lighter indigo in OS-dark mode. Remove the probe div.

- [ ] **Step 4: Verify + commit**

Run: `npx tsc --noEmit && npm run build` — success.
```bash
git add -A && git commit -m "feat: pin quiet-console tokens (2px radius, indigo primary, success/warning)"
```

---

### Task 5: P1 — Persisted `theme` setting (Rust + TS contract)

**Files:**
- Modify: `src-tauri/src/settings.rs` (struct ~line 140, `Default` impl, load fn)
- Modify: `src/types.ts` (`AppSettings` interface)

**Interfaces:**
- Produces: `AppSettings.theme: "system" | "light" | "dark"` end-to-end (serde-defaulted, version 3). Later tasks consume via `settings.theme` and `updateAppSettings`.

- [ ] **Step 1: Extend the Rust struct**

In `pub struct AppSettings` add:
```rust
    #[serde(default = "default_theme")]
    pub theme: String,
```
Next to `fn default_tailscale_hostname()` add:
```rust
fn default_theme() -> String {
    "system".into()
}
```
In `impl Default for AppSettings`: change `version: 2` → `version: 3` and add `theme: default_theme(),`.

- [ ] **Step 2: Normalize on load**

Locate the function that deserializes stored settings (grep `serde_json::from` / `get_app_settings` command path in `settings.rs`/`commands.rs`). Immediately after successful deserialization insert:
```rust
    if !matches!(settings.theme.as_str(), "light" | "dark" | "system") {
        settings.theme = "system".into();
    }
```

- [ ] **Step 3: Mirror in TS (`src/types.ts`)**

```ts
export type ThemePreference = "system" | "light" | "dark";

export interface AppSettings {
  version: number;
  start_on_launch: boolean;
  tailscale_hostname: string;
  onboarding_complete: boolean;
  theme: ThemePreference;
}
```

- [ ] **Step 4: Verify**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo check` then `npx tsc --noEmit`.
Expected: both pass. (`cargo check` will flag every construction site of `AppSettings` missing `theme` — fix each with `default_theme()`.)

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(settings): add persisted theme preference (v3)"
```

---

### Task 6: P1 — ThemeProvider + use-theme hook

**Files:**
- Create: `src/hooks/use-theme.tsx`
- Modify: `src/App.tsx` (wrap tree)

**Interfaces:**
- Consumes: `ThemePreference`, `AppSettings` from `@/lib/types` (re-exported through `api.ts` chain — import from `"./types"` while that file still exists; Task 10 moves it to `lib/`).
- Produces:
```ts
function ThemeProvider(props: {
  initialTheme: ThemePreference;
  onThemeChange: (t: ThemePreference) => void;
  children: React.ReactNode;
}): JSX.Element
function useTheme(): { theme: ThemePreference; resolvedTheme: "light" | "dark"; setTheme: (t: ThemePreference) => void }
```

- [ ] **Step 1: Implement `src/hooks/use-theme.tsx`**

```tsx
import { createContext, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { ThemePreference } from "../types";

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
  }), [theme, onThemeChange]);

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used within ThemeProvider");
  return ctx;
}
```

- [ ] **Step 2: Mount in `App.tsx`**

While `settings` can still be null during load, remember the last known theme so late mount keeps it:

```tsx
const savedTheme: ThemePreference = settings?.theme ?? "system";
const handleThemeChange = useCallback(async (t: ThemePreference) => {
  if (!settings) return;
  try {
    await saveSettings({ ...settings, theme: t });
  } catch (err) {
    reportError(parseErrorMessage(err));
  }
}, [settings, saveSettings, reportError]);
```
Wrap the returned tree: `<ThemeProvider initialTheme={savedTheme} onThemeChange={(t) => void handleThemeChange(t)}><AppShell …>…</AppShell></ThemeProvider>`.

- [ ] **Step 3: Verify**

`tsc` + build green. Dev-run: toggle is not reachable yet — just confirm app boots identically and `<html>` has no stray `dark` class in light OS mode, has one in dark OS mode.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(ui): theme provider with system tracking over .dark class"
```

---

### Task 7: P1 — Rebuild App Shell, Sidebar, PageHeader, PausedBanner

**Files:**
- Create: `src/components/layout/app-shell.tsx`, `src/components/layout/sidebar.tsx`, `src/components/layout/page-header.tsx`, `src/components/layout/paused-banner.tsx`
- Delete: `src/components/AppShell.tsx`, `src/components/Sidebar.tsx`, `src/components/StatusBadge.tsx` (only consumer was the old Sidebar)
- Modify: `src/App.tsx` (swap imports/props)

**Interfaces:**
- Consumes: ui `button sheet tooltip separator badge`; `ViewId`, `VIEW_META` from `"../types"`.
- Produces:
```tsx
function AppShell(props: { page: ViewId; onNavigate: (p: ViewId) => void; ready: boolean; children: ReactNode }): JSX.Element
// PageHeader is rendered INSIDE AppShell using VIEW_META[page]; title/subtitle props removed vs old API.
function PausedBanner(): JSX.Element | null   // reads paused state passed via prop `paused: boolean`
```
(App passes `paused` to AppShell which forwards to banner; keep prop drilling ≤1 level.)

- [ ] **Step 1: `sidebar.tsx` (nav items shared with mobile sheet)**

```tsx
import { FolderOpen, LayoutDashboard, ScrollText, Settings as SettingsIcon, ShieldCheck } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { ViewId } from "../../types";

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
```

- [ ] **Step 2: `app-shell.tsx` (rail ≥md, hidden <md with hamburger Sheet)**

```tsx
import { Menu } from "lucide-react";
import { useState } from "react";
import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Sheet, SheetContent, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { TooltipProvider } from "@/components/ui/tooltip";
import { VIEW_META, type ViewId } from "../../types";
import { Brand, NavItems, TunnelFooter } from "./sidebar";
import { PageHeader } from "./page-header";
import { PausedBanner } from "./paused-banner";

interface Props {
  page: ViewId;
  onNavigate: (p: ViewId) => void;
  ready: boolean;
  paused: boolean;
  children: ReactNode;
}

export function AppShell({ page, onNavigate, ready, paused, children }: Props) {
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const meta = VIEW_META[page];
  const navigate = (p: ViewId) => { setMobileNavOpen(false); onNavigate(p); };

  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex h-screen overflow-hidden">
        {/* Desktop rail: full ≥lg, icons-only md–lg, hidden <md */}
        <aside className="hidden w-60 flex-col border-r bg-card p-3 md:flex max-lg:w-14 max-lg:items-center">
          <Brand />
          <NavItems page={page} onNavigate={navigate} />
          <TunnelFooter ready={ready} />
        </aside>

        <div className="flex min-w-0 flex-1 flex-col">
          {/* Mobile top bar */}
          <div className="flex items-center gap-2 border-b px-3 py-2 md:hidden">
            <Sheet open={mobileNavOpen} onOpenChange={setMobileNavOpen}>
              <SheetTrigger asChild>
                <Button variant="ghost" size="icon" aria-label="Open navigation"><Menu aria-hidden="true" /></Button>
              </SheetTrigger>
              <SheetContent side="left" className="w-64 p-3">
                <SheetTitle className="sr-only">Navigation</SheetTitle>
                <Brand />
                <Separator className="my-2" />
                <NavItems page={page} onNavigate={navigate} />
                <TunnelFooter ready={ready} />
              </SheetContent>
            </Sheet>
            <span className="text-sm font-semibold">{meta.title}</span>
          </div>

          <PageHeader title={meta.title} subtitle={meta.subtitle} />
          <PausedBanner paused={paused} />
          <main className="flex-1 overflow-y-auto">
            <div className="mx-auto w-full max-w-5xl px-6 py-6 lg:px-8">{children}</div>
          </main>
        </div>
      </div>
    </TooltipProvider>
  );
}
```

Note for the executor: the old shell received `title/subtitle` and rendered its own header; the new one derives them from `VIEW_META` internally. Update `App.tsx` call-site accordingly (drop `title/subtitle` props, add `paused={connection?.paused === true}`).

- [ ] **Step 3: `page-header.tsx` and `paused-banner.tsx`**

```tsx
// page-header.tsx
interface Props { title: string; subtitle: string; actions?: React.ReactNode }
export function PageHeader({ title, subtitle, actions }: Props) {
  return (
    <header className="sticky top-0 z-10 flex items-start justify-between gap-4 border-b bg-background/80 px-6 py-4 backdrop-blur lg:px-8">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
        <p className="mt-0.5 text-[13px] text-muted-foreground">{subtitle}</p>
      </div>
      {actions ? <div className="flex items-center gap-2">{actions}</div> : null}
    </header>
  );
}
```
```tsx
// paused-banner.tsx
import { PauseCircle } from "lucide-react";
export function PausedBanner({ paused }: { paused: boolean }) {
  if (!paused) return null;
  return (
    <div role="status" className="flex items-center gap-2 border-b bg-warning/15 px-6 py-2 text-[13px] text-warning-foreground">
      <PauseCircle aria-hidden="true" className="size-4" />
      <span>MCP access is paused. Connection may be online, but file tools are blocked.</span>
    </div>
  );
}
```
(`bg-warning/15` over `--background` yields AA-legible text because `--warning-foreground` is near-black; verify visually in both themes.)

- [ ] **Step 4: Rewire `App.tsx`, delete old shell files**

Swap `import AppShell from "./components/AppShell"` → `{ AppShell } from "@/components/layout/app-shell"`. Update props per Step 2 note. Then:
```bash
git rm src/components/AppShell.tsx src/components/Sidebar.tsx src/components/StatusBadge.tsx
```

- [ ] **Step 5: Verify smoke**

Build green. Dev-run manual: nav switches all 5 views; active item highlighted; resize window — rail collapses to icons ≥768px, hamburger + sheet <768px; tunnel badge reflects connection; Esc closes sheet; keyboard Tab reaches every nav item with visible ring.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(layout): rebuild shell/sidebar/header on shadcn primitives"
```

---

### Task 8: P1 — Sonner replaces hand-rolled toast stack

**Files:**
- Create: `src/hooks/use-toast.ts`
- Modify: `src/App.tsx`, `src/main.tsx` (Toaster mount), `package.json`
- Delete: toast state/markup inside `App.tsx` (`toasts`, `pushToast`, `dismissToast`, `TOAST_ICONS`, `.toast-stack` block, `TOAST_TTL_MS`)

**Interfaces:**
- Consumes: `useTheme().resolvedTheme` for Toaster theme.
- Produces: `useToast(): { info(m: string): void; success(m: string): void; error(m: string): void }`. `reportError(message)` in App becomes `error(message)` — signature seen by child pages does NOT change.

- [ ] **Step 1: Install + create wrapper**

```bash
npm i sonner
```
```ts
// src/hooks/use-toast.ts
import { toast } from "sonner";

export function useToast() {
  return {
    info: (message: string) => toast.info(message),
    success: (message: string) => toast.success(message),
    error: (message: string) => toast.error(message),
  };
}
```

- [ ] **Step 2: Mount themed Toaster in `main.tsx`**

Mounting lives in App (needs theme): in `App.tsx` add
```tsx
import { Toaster } from "sonner";
// inside ThemeProvider, after AppShell:
<Toaster position="bottom-right" theme={resolvedFromUseTheme} richColors closeButton />
```
Obtain `resolvedTheme` via `useTheme()` in the inner subtree — if App renders the Provider itself, extract an `AppInner` component so `useTheme()` is legal inside.

- [ ] **Step 3: Port call-sites**

`pushToast(tone, message)` occurrences become `toast[tone](message)` via `const toast = useToast();` — `reportError` body: `(message: string) => toast.error(message)`. Delete all legacy toast code listed above.

- [ ] **Step 4: Verify**

Build green; dev-run: trigger an error toast (stop backend? simplest: temporarily call `reportError("test")` in a useEffect, see styled toast bottom-right in both themes, auto-dismiss, close button works, screen-reader announces via role=status/alert built into sonner). Remove probe.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(ui): adopt sonner toasts with theme-aware styling"
```

---

### Task 9: P1 — Extract data hooks from the god-component

**Files:**
- Create: `src/hooks/use-connection.ts`, `src/hooks/use-workspaces.ts`, `src/hooks/use-audit.ts`
- Modify: `src/App.tsx` (consume hooks; delete moved state/effects)

**Interfaces:**
- Consumes: `api` + `parseErrorMessage` from `"../lib/api"` (pre-move path `"../api"`).
- Produces:
```ts
function useConnection(reportError: (m: string) => void): {
  connection: ConnectionStatus | null;
  refreshConnection: () => Promise<void>;   // re-pull after mutations
}
function useWorkspaces(reportError: (m: string) => void): {
  workspaces: Workspace[];
  reloadWorkspaces: () => Promise<void>;    // list + audit refresh pairing stays in caller
}
function useAudit(reportError: (m: string) => void): {
  auditEvents: AuditEvent[];
  refreshAudit: () => Promise<void>;
}
```

- [ ] **Step 1: Implement the three hooks by MOVING existing logic verbatim**

`use-connection.ts` owns the `connection` state plus the `connection-status` event subscription currently in App's first `useEffect` (the `listen<ConnectionStatus>("connection-status", …)` block; on event it also calls `refreshAudit` — accept it as a parameter to avoid cross-hook coupling):

```ts
import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, parseErrorMessage } from "../lib/api";
import type { ConnectionStatus } from "../lib/types";

export function useConnection(
  reportError: (m: string) => void,
  onConnectionEvent: () => void,
) {
  const [connection, setConnection] = useState<ConnectionStatus | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void (async () => {
      try {
        const status = await api.connectionStatus();
        if (!disposed) setConnection(status);
        unlisten = await listen<ConnectionStatus>("connection-status", (e) => {
          setConnection(e.payload);
          onConnectionEvent();
        });
      } catch (err) {
        reportError(parseErrorMessage(err));
      }
    })();
    return () => { disposed = true; unlisten?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const refreshConnection = useCallback(async () => {
    try {
      setConnection(await api.connectionStatus());
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }, [reportError]);

  return { connection, refreshConnection };
}
```

`use-workspaces.ts`: move `workspaces` state + expose `reloadWorkspaces` wrapping `api.listWorkspaces()` with the same try/catch/reportError shape as above. `use-audit.ts`: move `auditEvents` state + `refreshAudit` (`api.recentAudit()`) identically.

- [ ] **Step 2: Rewire `App.tsx`**

Replace the three moved state atoms/effects with hook calls; keep orchestration effects that COMBINE loaders exactly as today:

```tsx
const { connection, refreshConnection } = useConnection(reportError, () => void refreshAudit());
const { workspaces, reloadWorkspaces } = useWorkspaces(reportError);
const { auditEvents, refreshAudit } = useAudit(reportError);
```
The initial-load effect keeps its single `Promise.all([reloadWorkspaces(), refreshAudit(), api.serverStatus()])` shape and `initialLoadDone`; settings/onboarding effect stays in App (theme provider consumes it). No page props change.

- [ ] **Step 3: Verify + commit**

Run: `npx tsc --noEmit && npm run build`. Dev smoke: boot loads lists once, connection events still trigger audit refresh, no duplicate subscriptions under StrictMode double-mount.
```bash
git add -A && git commit -m "refactor(app): extract connection/workspaces/audit data hooks"
```

---

### Task 10: P2 — Shared primitives + format helpers

**Files:**
- Create: `src/components/shared/status-badge.tsx`, `src/components/shared/stat-card.tsx`, `src/components/shared/copy-button.tsx`, `src/components/shared/confirm-dialog.tsx`, `src/components/shared/empty-state.tsx`, `src/lib/format.ts`, `src/lib/types.ts` (move of `src/types.ts`)
- Modify: all files importing `"../types"` / `"./types"` (mechanical re-point to `"@/lib/types"`); `src/api.ts` type imports

**Interfaces:**
- Produces:
```tsx
function StatusBadge(props: { state: ConnectionState | null; paused?: boolean }): JSX.Element
function StatCard(props: { label: string; value: string; hint?: string; icon?: LucideIcon }): JSX.Element
function CopyButton(props: { value: string; label?: string }): JSX.Element
function ConfirmDialog(props: { open: boolean; title: string; message: string; confirmLabel?: string; cancelLabel?: string; tone?: "danger" | "warning"; onConfirm(): void; onCancel(): void }): JSX.Element
function EmptyState(props: { icon?: LucideIcon; title: string; message: string; action?: ReactNode }): JSX.Element
function formatTime(ts: string): string       // HH:MM:SS local
function formatRelative(ts: string): string   // "2m ago"
```

- [ ] **Step 1: Move `src/types.ts` → `src/lib/types.ts`** (content unchanged), then repoint every import (`grep -rln 'from "\.\./types"\|from "\./types"' src/`) to `@/lib/types`. `api.ts` keeps re-exporting domain types.

- [ ] **Step 2: `lib/format.ts`**

```ts
export function formatTime(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function formatRelative(ts: string): string {
  const d = new Date(ts).getTime();
  if (Number.isNaN(d)) return ts;
  const s = Math.max(0, Math.round((Date.now() - d) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}
```

- [ ] **Step 3: `status-badge.tsx`**

```tsx
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { ConnectionState } from "@/lib/types";

const LABELS: Record<ConnectionState["state"], string> = {
  stopped: "Stopped", starting_server: "Starting…", starting_tunnel: "Starting tunnel…",
  needs_login: "Login required", needs_funnel_permission: "Funnel permission needed",
  verifying: "Verifying…", online: "Online", degraded: "Degraded", stopping: "Stopping…", error: "Error",
};

type Tone = "success" | "warning" | "destructive" | "secondary";
const TONE: Record<ConnectionState["state"], Tone> = {
  online: "success", verifying: "warning", starting_server: "secondary", starting_tunnel: "secondary",
  stopping: "secondary", needs_login: "warning", needs_funnel_permission: "warning",
  degraded: "warning", stopped: "secondary", error: "destructive",
};

export function StatusBadge({ state, paused }: { state: ConnectionState | null; paused?: boolean }) {
  if (!state) return <Badge variant="secondary">Unknown</Badge>;
  return (
    <Badge variant="outline" className="gap-1.5">
      <span aria-hidden="true" className={cn(
        "size-2 rounded-full",
        TONE[state.state] === "success" && !paused ? "bg-success"
        : TONE[state.state] === "destructive" ? "bg-destructive"
        : TONE[state.state] === "warning" ? "bg-warning" : "bg-muted-foreground",
      )} />
      {paused ? "Paused" : LABELS[state.state]}
    </Badge>
  );
}
```
(Color meaning comes from the dot; the chip itself stays outline — matches Quiet Console restraint.)

- [ ] **Step 4: `copy-button.tsx`** — icon Button (variant ghost, size icon) with `Copy`/`Check` swap for 1.5s, `navigator.clipboard.writeText(value)`, `aria-label={label ?? "Copy"}`, wrapped in Tooltip showing `label`.

- [ ] **Step 5: `confirm-dialog.tsx`** over AlertDialog, preserving the legacy prop API exactly:

```tsx
import { AlertTriangle, XOctagon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog";
import { cn } from "@/lib/utils";

export function ConfirmDialog({ open, title, message, confirmLabel = "Confirm", cancelLabel = "Cancel", tone = "danger", onConfirm, onCancel }: {
  open: boolean; title: string; message: string; confirmLabel?: string; cancelLabel?: string;
  tone?: "danger" | "warning"; onConfirm: () => void; onCancel: () => void;
}) {
  const Icon = tone === "warning" ? AlertTriangle : XOctagon;
  return (
    <AlertDialog open={open} onOpenChange={(o) => !o && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle className="flex items-center gap-2">
            <Icon aria-hidden="true" className={cn("size-5", tone === "warning" ? "text-warning" : "text-destructive")} />
            {title}
          </AlertDialogTitle>
          <AlertDialogDescription>{message}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel onClick={onCancel}>{cancelLabel}</AlertDialogCancel>
          <AlertDialogAction
            onClick={onConfirm}
            className={cn(tone === "warning" ? "" : "bg-destructive text-white hover:bg-destructive/90")}
          >
            {confirmLabel}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
```
(Focus lands on Cancel by default via Radix ordering — the safe action, matching old behavior.)

- [ ] **Step 6: `stat-card.tsx` + `empty-state.tsx`** — Card with muted uppercase 12px label, `text-2xl font-semibold tabular-nums` value, optional hint line + icon top-right; EmptyState = centered flex column, muted icon in `rounded-md bg-muted size-12` square, title `font-medium`, message `text-sm text-muted-foreground`, optional action node.

- [ ] **Step 7: Verify** — `npx tsc --noEmit && npm run build` green (old pages untouched still compile against moved types).

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "feat(shared): status/stat/copy/confirm/empty primitives and format helpers"
```

---

### Task 11: P2 — Dashboard rebuild

**Files:**
- Create: `src/components/features/dashboard/connection-card.tsx`, `src/components/features/dashboard/server-info-card.tsx`
- Rewrite: `src/pages/DashboardPage.tsx` → rename to `src/pages/dashboard-page.tsx`
- Delete: `src/components/ConnectionCard.tsx` (263-line legacy), `src/components/StatCard.tsx` (superseded by shared)

**Interfaces:**
- Consumes: shared `StatusBadge/StatCard/CopyButton/EmptyState`, `formatTime/formatRelative`, hooks live in App and pass down (unchanged data flow).
- Produces: `DashboardPage` default export with SAME props as today: `{ connection, workspaces, auditEvents, server, onNavigate, reportError }`.

- [ ] **Step 1: `connection-card.tsx`** — Card containing: StatusBadge + FQDN row (`font-mono truncate` + CopyButton + external-link Button calling `api.openExternalUrl(fqdn)`), primary Start/Stop Button (`size-lg`, spinner while busy — reuse existing `startConnection/stopConnection/retryConnection/rotateConnectionUrl` handlers from the old `ConnectionCard.tsx` VERBATIM, including error paths and busy gating; only the JSX changes). Needs-login / needs-permission / degraded states render their guidance text + action buttons exactly as the old card did — port each conditional branch, do not redesign behavior.

- [ ] **Step 2: `server-info-card.tsx`** — grid of 3 `StatCard`s: Local port (mono), Server state (running/paused), Recent activity count; below, latest 3 audit rows (`formatRelative(ts)` with `title={new Date(ts).toLocaleString()}`), "View all" ghost Button → `onNavigate("activity")`.

- [ ] **Step 3: `dashboard-page.tsx`** — composition: `<ConnectionCard/>`, `<ServerInfoCard/>`, quick-links grid to Workspaces/Security using Cards with chevron Buttons. Keep prop plumbing to App identical; update `App.tsx` import path to the kebab-case file.

- [ ] **Step 4: Delete superseded files, verify, commit**

```bash
git rm src/components/ConnectionCard.tsx src/components/StatCard.tsx
npx tsc --noEmit && npm run build
git add -A && git commit -m "feat(dashboard): rebuild view on shadcn primitives"
```
Manual smoke (both themes): offline → Start flow reaches login/verify/online branches; copy button copies FQDN; stats match server panel; keyboard traversal clean.

---

### Task 12: P2 — Security rebuild

**Files:**
- Create: `src/components/features/security/pause-card.tsx`, `src/components/features/security/url-rotation-card.tsx`, `src/components/features/security/identity-reset-card.tsx`
- Rewrite: `src/pages/SecurityPage.tsx` → `src/pages/security-page.tsx`
- Delete: none (legacy SecurityPage duplicate died in Task 1)

**Interfaces:**
- Produces: `SecurityPage` default export, SAME props `{ connection, workspaces, onPauseChanged, onConnectionChanged, reportError }`.

- [ ] **Step 1: Port handlers verbatim from current `src/pages/SecurityPage.tsx`** (`setPaused`, rotate URL, reset identity incl. its ConfirmDialog flows and diagnostics toggle) — logic untouched; presentation swaps: native checkbox pause → `Switch` + Label "Pause MCP access"; destructive confirms → shared `ConfirmDialog` (same tone mapping: warning/danger); diagnostics `<details>` → `Collapsible`-style Card with chevron Button toggling `recent_sidecar_logs` list in `font-mono text-xs`.

- [ ] **Step 2: Compose three cards** in one column (`space-y-4`): Pause, URL rotation (fqdn mono + CopyButton + rotate Button with busy spinner), Identity reset (collapsed danger zone, red-outline Button).

- [ ] **Step 3: Verify + commit**

```bash
npx tsc --noEmit && npm run build
git add -A && git commit -m "feat(security): rebuild view on shadcn primitives"
```
Smoke: pause strip appears globally when toggled (Task 7 banner), rotation updates URL everywhere, reset confirm dialog traps focus and cancels safely.

---

### Task 13: P3 — Workspaces rebuild

**Files:**
- Create: `src/components/features/workspaces/workspace-card.tsx`, `src/components/features/workspaces/execution-mode-picker.tsx`
- Rewrite: `src/pages/WorkspacesPage.tsx` → `src/pages/workspaces-page.tsx`

**Interfaces:**
- Consumes: shared `ConfirmDialog`, `EmptyState`, ui `switch radio-group label button card badge`.
- Produces: `WorkspacesPage` default export, SAME props `{ workspaces, onWorkspacesChanged, reportError }`.

- [ ] **Step 1: Port ALL business rules verbatim** from current `WorkspacesPage.tsx` — Read↔Write coherence (`toggleCapability`), execute-off keeps mode, full-powershell re-enable requires confirm (`toggleExecute`/`changeMode`/`applyExecutionAccess`), `busyId` gating, focus-scroll-to-new-workspace effect. Zero logic edits allowed; this task is presentational only.

- [ ] **Step 2: Presentation swaps**
  - Permission toggles: three `Card`-row sections each with `Switch` checked={ws.can_*} disabled={busy}, strong label + 13px muted description (copy preserved: "List, search and inspect files" etc.), `role=group aria-label` kept.
  - `execution-mode-picker.tsx`: `RadioGroup` (value=`ws.execution_mode`, onValueChange→changeMode) two items styled as adjacent segmented Buttons — Approved commands / TerminalSquare Full PowerShell; `aria-checked` semantics come from RadioGroup.
  - Approved executables list: `Badge variant="secondary" font-mono text-xs` per cmd (data stays `APPROVED_COMMANDS`).
  - High-risk note: `text-[13px]` with `text-warning` icon + `AlertTriangle`.
  - Remove action: ghost-destructive Button (`variant="ghost" className="text-destructive hover:bg-destructive/10"`).
  - Toolbar: h2 + description left, primary "Add folder…" right (`disabled={adding}`, Loader2 spin) — matches PageHeader toolbar convention at content level.

- [ ] **Step 3: Verify + commit**

```bash
npx tsc --noEmit && npm run build
git add -A && git commit -m "feat(workspaces): rebuild view on shadcn primitives"
```
Smoke: toggle matrix behaves exactly as before (Read off ⇒ Write off; Write on ⇒ Read on; Execute off/on; mode change to Full PowerShell always confirms); busy spinner per card; narrow window stacks switches vertically.

---

### Task 14: P3 — Settings rebuild + Appearance (theme picker)

**Files:**
- Create: `src/components/features/settings/general-form.tsx`, `src/components/features/settings/appearance-form.tsx`
- Rewrite: `src/pages/SettingsPage.tsx` → `src/pages/settings-page.tsx`

**Interfaces:**
- Consumes: `useTheme()` (Task 6), `saveSettings` prop (existing), ui `switch input label radio-group card`.
- Produces: `SettingsPage` default export, SAME props `{ settings, connection, onSaveSettings, reportError }`.

- [ ] **Step 1: `general-form.tsx`** — port existing fields: `start_on_launch` → `Switch`; `tailscale_hostname` → `Input` with mono font + inline validation (non-empty, no spaces/`.`-leading — mirror any validation the old page had; if it had none, trim+require non-empty). Save semantics unchanged: optimistic-local disabled — keep the existing save/rollback pattern exactly (`onSaveSettings` throws propagate → reportError + rollback).

- [ ] **Step 2: `appearance-form.tsx`**

```tsx
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

export function AppearanceForm() {
  const { theme, setTheme } = useTheme();
  return (
    <RadioGroup value={theme} onValueChange={(v) => setTheme(v as ThemePreference)} className="flex gap-3">
      {OPTIONS.map(({ value, label, icon: Icon }) => (
        <Label key={value} className="flex flex-1 cursor-pointer items-center gap-3 rounded-md border p-3 has-[[data-state=checked]]:border-primary">
          <RadioGroupItem value={value} />
          <Icon aria-hidden="true" className="size-4" />
          {label}
        </Label>
      ))}
    </RadioGroup>
  );
}
```
Persistence happens inside ThemeProvider → App `handleThemeChange` → `updateAppSettings` (already built in Task 6); this form is purely the control surface.

- [ ] **Step 3: Compose page** — two labeled Card sections (General / Appearance) in one column. Rename file kebab-case, update App import.

- [ ] **Step 4: Verify + commit**

```bash
npx tsc --noEmit && npm run build
git add -A && git commit -m "feat(settings): rebuild view and add appearance theme picker"
```
Smoke: pick Dark → whole app flips instantly AND survives restart (persisted v3 setting); System follows OS toggle live; invalid hostname rolls back with error toast.

---

### Task 15: P4 — Activity rebuild (table + detail sheet)

**Files:**
- Create: `src/components/features/activity/audit-table.tsx`, `src/components/features/activity/audit-detail-sheet.tsx`
- Rewrite: `src/pages/ActivityPage.tsx` → `src/pages/activity-page.tsx`
- Delete: `src/components/ActivityDetail.tsx` (superseded)

**Interfaces:**
- Consumes: `formatTime/formatRelative`, ui `table sheet badge input tooltip`, shared `CopyButton/EmptyState`.
- Produces: `ActivityPage` default export, SAME props `{ auditEvents, workspaces, onRefresh, reportError }`.

- [ ] **Step 1: `audit-table.tsx`** — ui Table: columns Time (`formatRelative`, `title` absolute, `whitespace-nowrap`), Source, Op (`font-medium`), Decision (`Badge`: allow→outline w/ success dot, deny/error→destructive), Path (`font-mono text-xs truncate max-w-[16rem]` + Tooltip full + CopyButton). Row click → opens detail Sheet; Enter key opens focused row (make row a `button` semantically via `tabIndex=0 role=button onKeyDown`). Search `Input` filters client-side on op/path/source (port existing filter logic verbatim). Sticky `TableHeader` (`sticky top-0 bg-card`).

- [ ] **Step 2: `audit-detail-sheet.tsx`** — right-side Sheet listing all `AuditEvent` fields as definition rows; `detail` JSON pretty-printed in `pre font-mono text-xs bg-muted rounded-md p-3 overflow-x-auto`.

- [ ] **Step 3: Narrow-window degradation** — `< md`: hide table, render same events as stacked Cards (time+op+decision badge+path) tapping into the same Sheet. Implement as a `max-md:block hidden` sibling list driven by identical filtered array — no duplicated filtering logic.

- [ ] **Step 4: Swap page, delete old, verify + commit**

```bash
git rm src/components/ActivityDetail.tsx
npx tsc --noEmit && npm run build
git add -A && git commit -m "feat(activity): rebuild audit trail as table plus detail sheet"
```
Smoke: refresh button works; search filters; row→sheet shows full payload; keyboard: Tab to row, Enter opens, Esc closes.

---

### Task 16: P4 — Onboarding rebuild on Dialog

**Files:**
- Create: `src/components/features/onboarding/onboarding-dialog.tsx`
- Delete: `src/components/OnboardingFlow.tsx` (371 lines)

**Interfaces:**
- Consumes: ui `dialog button badge card`, shared `StatusBadge`, existing api methods.
- Produces: `OnboardingFlow` equivalent export with IDENTICAL props `{ connection, workspaces, onWorkspacesChanged, onClose, onFinish, reportError }`.

- [ ] **Step 1: Preserve the state machine verbatim.** Open old `OnboardingFlow.tsx`; copy its stage logic (stage derivation from `connection.state` variants incl. `needs_login` login_url open via `api.openExternalUrl`, workspace-add step calling folder picker + `api.addWorkspace`, finish gating ONLY at Online stage, skip/close never marks complete) into the new component without behavioral edits.

- [ ] **Step 2: Re-shell the JSX** — `Dialog open onOpenChange={(o)=>!o&&onClose()}` non-dismissible-by-overlay if the original blocked dismissal (match old behavior exactly; if old allowed overlay-close, keep allowing); header with step count Badge ("Step k of n"), body per-stage content ported with ui primitives (points list → muted list with CheckCircle2; permissions summary → Badge group; diagnostics → same collapsible pattern as security), footer: Skip (ghost, left) / Continue (right, primary; disabled until stage requirements met) / Finish (only Online).

- [ ] **Step 3: Verify + commit**

```bash
npx tsc --noEmit && npm run build
git add -A && git commit -m "feat(onboarding): rebuild wizard on dialog primitives"
```
Smoke (reset `onboarding_complete=false` in stored settings to trigger): full happy path login→online→Finish persists completion; Esc/skip leaves completion false; workspace added mid-flow refreshes list.

---

### Task 17: P4 — Legacy CSS purge (hard milestone)

**Files:**
- Delete: `src/styles/tokens.css`, `src/styles/global.css`, `src/styles/components.css`, `src/components/EmptyState.tsx` (old; pages now import shared), `src/components/ConfirmDialog.tsx` (old; security/workspaces use shared)
- Modify: `src/main.tsx` (remove 3 legacy imports — keep globals + fonts)

**Interfaces:**
- Produces: single stylesheet `globals.css`; zero legacy classnames in JSX.

- [ ] **Step 1: Repoint stragglers** — `grep -rn "components/EmptyState\|components/ConfirmDialog" src/` → switch those imports to `@/components/shared/*`.

- [ ] **Step 2: Delete files + imports**, then prove no legacy classes remain:

```bash
grep -rn "className=\"[^\"]*\b\(btn\b\|toast\|dialog-backdrop\|dialog-card\|workspace-card\|nav-item\|sidebar\|permission-toggle\|mode-option\|onboarding-\)" src/ || echo CLEAN
```
Expected: `CLEAN` (any hit = an unmigrated classname; fix before proceeding).

- [ ] **Step 3: Full visual sweep** — dev-run every view in light AND dark looking for unstyled remnants (the moment preflight fully owns base styles). Fix by ADDING Tailwind classes, never by resurrecting legacy CSS.

- [ ] **Step 4: Verify + commit**

```bash
npx tsc --noEmit && npm run build
git add -A && git commit -m "refactor!: remove legacy css layer — tailwind is sole styling system"
```

---

### Task 18: P5 — Polish, window min-size, docs

**Files:**
- Modify: `src-tauri/tauri.conf.json` (window min size), `README.md` (stack section)

- [ ] **Step 1: Window minimums** — read `tauri.conf.json` windows config; if `minWidth`/`minHeight` absent or smaller, set `"minWidth": 720, "minHeight": 480`.

- [ ] **Step 2: A11y & polish checklist (fix findings inline)** — run through EVERY view, both themes, keyboard-only, and reduced-motion emulation (DevTools): visible focus rings everywhere; contrast spot-check muted text + badges (AA); icon-only buttons have aria-label + tooltip; dialogs trap and restore focus; no horizontal scrollbar at 720px width; animations respect reduced motion.

- [ ] **Step 3: Docs** — README: update tech-stack bullets (Tailwind v4, shadcn/ui, sonner), note theme setting location, replace any screenshots mentioning old UI.

- [ ] **Step 4: Final gates + commit**

```bash
npx tsc --noEmit && npm run build
export PATH="$HOME/.cargo/bin:$PATH" && cargo check
git add -A && git commit -m "chore(polish): min window size, a11y pass, docs refresh"
```

---

## Completion definition

All tasks checked; every phase gate (tsc, vite build, cargo check) green; legacy CSS files absent from repo; theme persists across restart; manual smoke lists executed for all 5 views + onboarding in both themes at ≥720px and narrow widths.
