# Local Workspaces Agent Plugin + Visual Awareness Design

**Date:** 2026-08-26
**Status:** Approved in chat

## Goal

Turn Local Workspaces from a custom MCP app into a plugin-ready workflow made of a first-class Agent Skill plus the existing MCP app, while keeping the Tauri desktop application as the security/control plane. Add explicit visual awareness so the agent can capture a selected Windows display or window and return the image through MCP for model vision analysis.

## Product shape

```text
Local Workspaces Plugin
├─ Skill: local-workspace-agent
│  ├─ discovers when Local Workspaces is relevant
│  ├─ follows an agentic engineering loop
│  └─ uses the Local Workspaces MCP app as its tool surface
│
└─ App: Local Workspaces MCP
   ├─ workspace/file tools
   ├─ execution tools
   ├─ runtime agent policy
   └─ visual-awareness tools
        ├─ list_capture_targets
        └─ capture_screen

Tauri Desktop App = authority + settings
├─ Pause
├─ workspace Read / Write / Execute
├─ execution mode / unrestricted access
├─ Agent mode
└─ Screen capture permission + scope
```

The Skill tells the model how to work. It never grants authority. The Rust backend remains the deterministic enforcement boundary.

## Agent Skill

Create an Agent Skills-compatible directory at `plugin/skills/local-workspace-agent/`.

The frontmatter name is `local-workspace-agent`. The description is discovery-oriented: it describes the situations in which the skill should load, not a compressed copy of the workflow. The main `SKILL.md` stays concise and progressively loads two focused references:

- `references/workspace-workflow.md` — discovery, planning, revision-safe editing, verification and recovery.
- `references/security-contract.md` — user authority, screen-capture privacy, Pause, workspace boundaries and non-bypass rules.

The operating loop is:

`DISCOVER → PLAN → INSPECT → ACT → VERIFY → RECOVER → REPORT`

Behavioral requirements:

1. Continue from clear user intent to a verified result instead of stopping at suggestions.
2. Discover the approved workspace before operating when an id is unknown.
3. Search/list/find before broad reads.
4. Read an existing file before patching and retain the latest revision.
5. Preserve unrelated user/concurrent changes.
6. Treat `FILE_CHANGED_SINCE_READ` as evidence: re-read, recompute, retry only when still valid.
7. Use execution tools for tests/build/lint when permission allows.
8. Diagnose failures before changing code again.
9. Use visual inspection only when visual evidence helps the task.
10. Never claim success without current verification evidence.
11. Never bypass disabled capabilities by switching tools, shelling out, or writing helper scripts.

The existing MCP `instructions` and five MCP prompts remain as compatibility/fallback guidance for hosts that do not load first-class Skills.

## Runtime agent policy

Add an MCP read-only tool named `get_agent_policy`.

It returns live settings rather than relying on connector metadata that a host may cache. The response contains:

- `mode`: `autonomous` or `conservative`
- `paused`
- `unrestricted_access`
- `screen_capture_enabled`
- `capture_scope`: `windows_and_displays` or `windows_only`
- `visual_verification`

Workspace-specific Read/Write/Execute capability remains discoverable via `list_workspaces`; `get_agent_policy` must not duplicate absolute filesystem paths or secrets.

`Autonomous` is the default mode. `Conservative` is guidance only: it tells the Skill to ask before materially consequential choices when user intent does not resolve them. Security remains backend-enforced in both modes.

## Visual awareness

### Tool surface

Add two MCP tools:

### `list_capture_targets`

Read-only. Returns sanitized capture targets available under current settings:

- display targets: stable session-local id, name, geometry, scale factor, primary flag
- window targets: stable session-local id, title, application name when available, geometry

When `screen_capture_enabled` is false, return `SCREEN_CAPTURE_DISABLED`. When Pause is active, return `ACCESS_PAUSED`.

When capture scope is `windows_only`, omit displays entirely.

### `capture_screen`

Read-only from the filesystem perspective but privacy-sensitive. Accepts one target id from `list_capture_targets` and optionally a maximum edge size. It captures a single frame, downsizes if necessary, encodes PNG, and returns:

1. MCP image content with `image/png`, so a vision-capable client can reason over actual pixels.
2. Structured/text metadata: target kind/title, original and returned dimensions, encoded byte count and SHA-256 hash.

The screenshot is not persisted to disk by default. Audit records contain metadata/hash, never the image bytes.

### Implementation dependency

Use `xcap` on Windows for monitor/window enumeration and capture. Keep capture behind a small `screen_capture` module so platform-specific details and image encoding do not leak into the MCP handler.

The first release is Windows-only because the desktop app is currently Windows-oriented. On unsupported platforms the capture layer returns a stable unsupported error rather than silently changing behavior.

### Limits

- Single-shot screenshots only.
- No screen recording.
- No webcam/camera.
- No microphone.
- No keyboard logging.
- No mouse/keyboard automation.
- No OCR-only fallback as a substitute for returning image pixels.

## Screenshot security model

Screen capture is a separate capability from workspace Read/Write/Execute because a screenshot can reveal data outside approved filesystem roots.

Rules:

1. `screen_capture_enabled` defaults to `false`.
2. Enabling unrestricted filesystem/execution access does **not** enable screen capture.
3. Pause always denies screen capture.
4. The Skill must not use `run_command`, `run_powershell`, scripts, browser commands, or OS utilities to work around disabled screen capture.
5. When screen capture is enabled, the autonomous agent may capture an allowed target when relevant without asking on every frame. The user has already authorized this behavior by turning the setting on.
6. The capture scope controls whether full displays are exposed. `windows_only` is the narrower privacy option.
7. Audit failures are fail-closed: if the capture event cannot be recorded, the image is not returned.
8. Screenshots are memory-only by default and are not written into the workspace.

## Desktop settings

Persist a new settings version containing:

- `agent_mode`, default `autonomous`
- `screen_capture_enabled`, default `false`
- `capture_scope`, default `windows_and_displays`
- `visual_verification`, default `true`

Migrate older settings by supplying defaults; no existing preference is lost.

Add an **Agent** card in Settings with:

- Agent mode: Autonomous / Conservative
- Screen capture toggle
- Capture scope selector, enabled only while screen capture is on
- Visual verification toggle
- explanatory privacy copy that screen capture can expose content outside approved workspace folders

The UI does not become the enforcement boundary. Backend settings are loaded into live runtime policy used directly by MCP handlers.

## MCP error model and audit

Add stable errors for at least:

- `SCREEN_CAPTURE_DISABLED`
- `SCREEN_CAPTURE_UNSUPPORTED`
- `CAPTURE_TARGET_NOT_FOUND`
- `SCREEN_CAPTURE_FAILED`

`ACCESS_PAUSED` remains the Pause response.

Both visual tools write one audit row for requests that reach handlers. Audit detail may include target id/kind, dimensions, bytes and SHA-256 but not title text when it could unnecessarily expose private content; title can be returned to the caller but need not be retained in audit.

## Plugin packaging

OpenAI currently documents plugins as containers that may include Skills, Apps and App templates, but does not publish a general repository-level plugin manifest format that lets this desktop repository independently manufacture a Plugin Directory listing. Therefore the repository deliverable is deliberately split:

1. **Installable Agent Skill artifact** — a validated zip containing `local-workspace-agent/SKILL.md` and references, uploadable from ChatGPT Skills > Create > Upload from your computer.
2. **Local Workspaces MCP app** — the public Tailscale Funnel endpoint configured/published as the custom app.
3. **Plugin publication guide** — exact workspace steps for combining the Skill with the published Local Workspaces app into the workspace/plugin publication flow when that UI is available to the account.

Do not invent an undocumented `plugin.json` schema. A real Plugin Directory/workspace publication is an external ChatGPT operation and cannot be proven by local build tests.

## Validation strategy

Follow RED → GREEN → REFACTOR.

### Agent Skill validation

- Script/test validates directory/name/frontmatter constraints and required safety/workflow clauses.
- Build script produces deterministic skill zip contents.
- Manual ChatGPT E2E: upload skill, confirm it appears under Skills, invoke a workspace task and verify it chooses the Local Workspaces app.

### MCP integration tests

- `get_agent_policy` returns defaults/live values.
- capture tools are discoverable.
- capture disabled fails with the stable error.
- Pause wins over visual tools.
- scope filtering is correct.
- capture success returns image content and metadata without writing files.
- existing file/execution security regression tests still pass.

The capture success integration test uses an injectable fake capture provider; automated CI must not require an interactive Windows desktop.

### Settings/UI tests/build

- persisted settings migrate with secure defaults.
- toggles round-trip through Tauri settings APIs.
- frontend type/build checks succeed.

### Release verification

Run formatting, clippy with warnings denied, full Rust tests, frontend build, Go sidecar tests/vet, sidecar build, and Tauri release packaging. Keep the external ChatGPT Plugin/Skill installation E2E explicitly separate from local verification.

## Non-goals

- Persistent background autonomous daemon.
- GUI click/type automation.
- Capturing without explicit user enablement.
- Treating the Skill as a security mechanism.
- Replacing deterministic backend checks with model instructions.
- Inventing or claiming support for undocumented OpenAI plugin package formats.
