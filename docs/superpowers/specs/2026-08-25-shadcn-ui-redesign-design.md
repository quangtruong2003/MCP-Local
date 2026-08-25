# shadcn/ui Redesign — Design System & Frontend Architecture

Date: 2026-08-25
Status: Approved by user (big-bang on current branch, Light/Dark/System toggle, full visual refresh, Tailwind v4, radius 1–2 px)
Scope: frontend only, plus one additive backend touch (theme field in `AppSettings`)

## 1. Goal

Replace the hand-rolled vanilla-CSS UI layer (~1,423-line `components.css`, ad-hoc primitives) with **shadcn/ui on Tailwind CSS v4**, so the app has a consistent, accessible, scalable design system. The rewrite is a **big-bang performed on the current branch (`feat/tailscale-funnel`) as a sequence of ordered commits**, each leaving `tsc && vite build` green and the app runnable.

Non-goals:

- No new product features beyond the theme setting required by dark mode.
- No router, no state-management library, no other UI framework.
- No backend behavior changes except persisting the new `theme` preference.
- No mobile/Tauri-2 phone targets in this effort (responsive rules exist only to survive desktop window resizing).

## 2. Decisions log

| Decision | Choice | Rationale |
|---|---|---|
| Migration strategy | Big-bang on current branch | User choice; sequenced commits keep every commit buildable |
| Dark mode | Light / Dark / System toggle persisted in settings | User choice; needs one additive `AppSettings` field |
| Visual identity | Full refresh ("Quiet Console": zinc neutrals, indigo primary, borders over shadows) | User choice ("refresh hoàn toàn") |
| Corner radius | **1–2 px** (`--radius: 2px`) | User requirement; sharp technical console look |
| Tailwind version | v4 (CSS-first `@theme`, official Vite plugin) | User choice; standard for new setups |
| Navigation | Keep state-based `ViewId` switching; no react-router | Flat 5-view desktop app; URLs add nothing inside a Tauri window |

## 3. Current-state audit (facts this design builds on)

- Stack: Tauri 2 (Windows-first) · React 18 · Vite 5 · TS strict · lucide-react. No Tailwind/shadcn/Radix yet. No frontend test framework.
- Styling today: `src/styles/tokens.css` (semantic vars, light/dark via `prefers-color-scheme`), `global.css`, `components.css` (single 1,423-line sheet).
- Views: Dashboard, Workspaces, Activity, Security, Settings (+ `OnboardingFlow` wizard). Shell = fixed 232 px sidebar + page header + scrollable content. Dense utility app (13 px type floor).
- Dead code verified by grep (8 files): `components/ActivityPage.tsx`, `components/SecurityPage.tsx` (stale duplicates of `pages/`), `SettingsDialog`, `ConnectionHero`, `StatusBar`, `PauseButton`, `AuditTable`, `WorkspaceList`. Plus Cloudflare-era `@deprecated` types in `api.ts`.
- Existing hand-rolled primitives map cleanly onto shadcn components (see §7 table).
- Backend `AppSettings` (`src-tauri/src/settings.rs`) already uses `#[serde(default)]` fields with a `version` counter — adding a defaulted field is backward-compatible.

## 4. Design principles

1. **Tokens only** — no hard-coded colors/sizes in components; everything resolves through semantic tokens.
2. **Composition over configuration** — Radix primitives + CVA variants; no hand-written overlay/focus logic.
3. **Pages thin, features fat, `ui/` dumb** — pages compose; IPC lives in hooks; `ui/` is vendored shadcn code.
4. **Accessibility by default** — Radix semantics, visible focus rings, AA contrast in both themes, `prefers-reduced-motion` respected.
5. **Backend authoritative, UI pessimistic** — security toggles await the backend result before reflecting state (preserve current `busyId` pattern).
6. **Progressive disclosure** — danger/advanced content collapsed by default (AlertDialog confirmations stay).
7. **YAGNI** — nothing beyond §1 scope.

## 5. Visual direction — "Quiet Console"

- Base: zinc neutral scale both themes; primary indigo — light `#4f46e5` (indigo-600) with white foreground, dark `#818cf8` (indigo-400) with near-black foreground; status colors carry the only strong hues (success/warning/destructive).
- Surfaces separated by 1 px borders first, shadows second (`shadow-xs` cards, popover shadow only for floating layers).
- Radius: `--radius: 0.125rem` (2 px). Because stock shadcn derives smaller radii by subtraction from `--radius` and would go sub-pixel/negative, the derived scale is pinned explicitly: `--radius-sm: 1px`, `--radius-md: 2px`, `--radius-lg: 2px`, `--radius-xl: 3px`. Deliberate exceptions keeping pill geometry (physical metaphors / vendor defaults): Switch track+thumb, radio dot, avatar circle if ever used.
- Typography: self-hosted `Inter Variable` (UI) + `JetBrains Mono` (paths, FQDNs, commands). No CDN font loading — the app runs offline. Type scale keeps the dense floor: 12/13/14/16/18/22 px (13 px minimum for table/meta text, 14 px body).

## 6. Design tokens strategy (Tailwind v4)

`src/styles/globals.css` becomes the single stylesheet:

```css
@import "tailwindcss";
@custom-variant dark (&:is(.dark *));

:root { --background: …; --foreground: …; --card: …; --popover: …;
        --primary: …; --secondary: …; --muted: …; --accent: …;
        --destructive: …; --border: …; --input: …; --ring: …;
        --success: …; --success-foreground: …;
        --warning: …; --warning-foreground: …;
        --radius: 0.125rem; /* + pinned derived radii per §5 */ }
.dark { … }

@theme inline {
  --color-background: var(--background); /* exposes bg-background, text-*,
                                            border-*, ring-* utilities */
  …
  --color-success: var(--success);
  --color-warning: var(--warning);
  --font-sans: "Inter Variable", ui-sans-serif, system-ui, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, monospace;
}
```

- Mapping from legacy tokens: `--bg-app→background`, `--bg-surface→card`, `--accent→primary`, `--border-default→border`, `--danger→destructive`. shadcn has no success/warning pair, so they are registered extensions — mandatory because the app's core meaning is connection state. Pinned values: success light `#15803d` / dark `#4ade80`; warning light `#b45309` / dark `#fbbf24`; foregrounds white (light) / near-black (dark). Destructive uses stock shadcn red.
- Theme application: `ThemeProvider` sets/removes `.dark` on `<html>`, follows `matchMedia("(prefers-color-scheme)")` in System mode, and persists the choice through `AppSettings.theme`.
- Standardized interaction states across the app: hover via accent/muted tokens; focus-visible = 2 px `--ring` offset 2; disabled = opacity ~55% + not-allowed cursor; loading = disabled button containing `<Loader2 className="animate-spin" />`; empty = shared `EmptyState`; transient feedback = sonner toasts (aria-live handled by sonner).
- Motion: color transitions ~140 ms + Radix enter animations; honor `prefers-reduced-motion`.

### Backend touch (only one)

`src-tauri/src/settings.rs`: add `#[serde(default = "default_theme")] pub theme: String` (`"system" | "light" | "dark"`), bump `version` to 3, validate on load (unknown values → `"system"`). Mirrored in `src/lib/types.ts` (`AppSettings.theme`). Settings UI gains an Appearance section with a three-option control.

## 7. Component architecture

Mapping (current → target):

| Current | Target |
|---|---|
| `.btn*` CSS classes | `Button` variants (default/destructive/outline/secondary/ghost/link) |
| Hand-rolled `ConfirmDialog` | `confirm-dialog.tsx` wrapper over `AlertDialog` (keeps existing prop API: open/title/message/tone/onConfirm/onCancel) |
| Toast stack in `App.tsx` | `sonner` `<Toaster />`; `use-toast` keeps the `reportError(message)` call-shape |
| `StatusBadge` | `Badge` + tone→variant mapping driven by `ConnectionState` |
| Native checkbox permission toggles | `Switch` + `Label` + helper text |
| Execution-mode radiogroup | `RadioGroup` (Radix arrow-key semantics) styled as segmented options |
| Inline audit markup | `Table` (+ detail in a `Sheet`) |
| `StatCard`, `EmptyState` | Rebuilt once under `components/shared/` on top of `Card`/tokens |
| Onboarding wizard | Dialog-based stepper under `components/features/onboarding/` using ui primitives |

State restructure (removes the god-component):

```
App.tsx = <ThemeProvider><AppShell>{view switch}</AppShell></ThemeProvider>
hooks/: use-theme, use-connection (status + event subscription),
        use-workspaces (list + mutations + busyId), use-audit, use-toast
```

Rules preventing duplication: generic-and-app-blind → `components/ui/`; cross-app composites → `components/shared/`; domain-aware pieces → `components/features/<domain>/`; pages only wire hooks into features. IPC (`invoke`) is callable only inside `lib/api.ts` consumed by hooks — never directly in components.

## 8. Folder structure

```
src/
├── main.tsx
├── App.tsx                      # providers + view switch only
├── styles/globals.css           # sole stylesheet (replaces 3 legacy CSS files)
├── lib/
│   ├── utils.ts                 # cn()
│   ├── api.ts                   # moved from src/api.ts, deprecated types removed
│   ├── types.ts                 # moved from src/types.ts
│   └── format.ts                # fmtTime & friends extracted from components
├── hooks/
│   ├── use-theme.ts  use-connection.ts  use-workspaces.ts
│   ├── use-audit.ts  use-toast.ts
├── components/
│   ├── ui/                      # vendored shadcn primitives — do not edit casually
│   ├── layout/                  # app-shell, sidebar, page-header, paused-banner
│   ├── shared/                  # empty-state, status-badge, stat-card, copy-button,
│   │                            # confirm-dialog (AlertDialog wrapper)
│   └── features/
│       ├── dashboard/  workspaces/  activity/  security/
│       ├── settings/   onboarding/
└── pages/                       # kebab-case, thin composition only
    ├── dashboard-page.tsx  workspaces-page.tsx  activity-page.tsx
    └── security-page.tsx   settings-page.tsx
```

Files are renamed to kebab-case during migration (cheap under big-bang; matches shadcn ecosystem conventions).

## 9. Layout architecture & responsive strategy

Shell: flex row, `h-screen overflow-hidden`. Sidebar 240 px (brand, 5 nav items, tunnel status footer). Main column: sticky `PageHeader` (title/subtitle left, primary actions slot right, backdrop-blur) above scrollable content constrained to `max-w-5xl` with consistent padding. Global `PausedBanner` renders above content while paused. Sonner toasts fixed bottom-right. No breadcrumbs (flat single-level nav); no footer (legacy StatusBar was dead code).

Window-resize behavior (desktop-first; also the future path for narrow windows):

| Breakpoint | Sidebar | Content adaptations |
|---|---|---|
| ≥ lg (1024 px) | Full 240 px | Full layout, max-w-5xl content |
| md – lg | Icon rail 56 px + tooltips | Single-column grids; header actions retained |
| < md | Hidden; hamburger opens left `Sheet` | Permission grids stack vertically; audit table degrades to card list; long action rows collapse into DropdownMenu |

Set window `minWidth`/`minHeight` in `tauri.conf.json` (verify current values at implementation; propose ≈720×480). Density stays mouse-first (32 px controls); Sheet-triggered controls ≥36 px.

## 10. UX quality bar (applied per page during migration)

- Exactly one primary action per page header (Dashboard: Start/Stop; Workspaces: Add folder…; Security: Pause control; Settings: implicit save-on-change).
- Labels always visible; never placeholder-as-label. Helper text and inline errors associated via `aria-describedby`.
- Tables: sticky header, monospace truncated paths/FQDNs with Tooltip + CopyButton, relative timestamps with absolute value in `title`.
- Destructive flows keep the current high-quality warning copy behind AlertDialog (focus trap + aria-modal come free from Radix).
- Contrast checked AA in both themes (the old soft-on-tint combos get replaced by solid token pairs + muted-foreground).
- Keyboard: natural tab order, Esc closes overlays, arrow keys inside RadioGroup — all inherited from Radix; verify manually per page.

## 11. Migration roadmap (ordered commits on this branch)

Verification gate after every phase: clean `tsc`, green `vite build`, manual smoke of touched views; `cargo check` where Rust changed.

- **P0 Cleanup** — delete the 8 dead components and `@deprecated` Cloudflare types. Pure subtraction; shrinks the diff surface.
- **P1 Foundation** — install Tailwind v4 + Vite plugin; `shadcn init`; self-hosted fonts; tokens + pinned radii + success/warning extensions; `ThemeProvider` + Rust `theme` field (settings version 3); install ui primitives (button, badge, card, dialog, alert-dialog, switch, radio-group, input, select, label, separator, table, sheet, tooltip, dropdown-menu, sonner); rebuild AppShell/Sidebar/PageHeader/paused-banner; sonner replaces the toast stack. Legacy CSS still present but shell no longer uses it.
- **P2 Wave 1** — Dashboard + Security fully migrated.
- **P3 Wave 2** — Workspaces (Switch/RadioGroup/AlertDialog) + Settings incl. Appearance theme section.
- **P4 Final wave** — Activity (Table + detail Sheet) + Onboarding rebuild; **delete `tokens.css`, `global.css`, `components.css`** (hard milestone: vanilla CSS ceases to exist).
- **P5 Polish & audit** — contrast pass both themes, keyboard pass, reduced-motion check, window min-size, README/docs refresh.

No merge until P5 completes; branch remains `feat/tailscale-funnel`.

## 12. Anti-patterns explicitly banned

1. Casually editing `components/ui/*` — extend via variants or `shared/` wrappers.
2. Hard-coded hex/arbitrary sizes (`w-[247px]`) outside tokens.
3. Keeping dual styling systems past P4.
4. Calling `invoke` outside `lib/api.ts`/hooks.
5. Optimistic updates on security toggles.
6. Placeholder-as-label; icon-only buttons without `aria-label` (and tooltip).
7. Adding router/state-library/second UI kit (YAGNI).
8. Prop drilling deeper than two levels — use a hook/context instead.
9. CDN-loaded fonts.
10. Unrelated "drive-by" refactors inside migration commits.

## 13. Verification strategy

- Static gate per phase: `tsc --noEmit` + `vite build` (and `cargo check` for P1).
- Manual smoke checklist per migrated page: states (hover/focus/disabled/loading/empty/error/success), light+dark, keyboard-only traversal, narrow-window resize down to min size.
- No new test framework in this effort (pre-existing condition, out of scope per §1); the static gate plus manual checklist is the agreed bar.
