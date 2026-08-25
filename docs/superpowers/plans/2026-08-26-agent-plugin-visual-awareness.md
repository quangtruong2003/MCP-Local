# Local Workspaces Agent Plugin + Visual Awareness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Ship a first-class `local-workspace-agent` Agent Skill alongside the Local Workspaces MCP app, add live agent policy and permission-gated Windows screenshot tools that return real MCP image content, and package/validate the Skill for ChatGPT upload.

**Architecture:** The Tauri app remains the security authority. Versioned settings feed a live `AgentPolicyState`; MCP exposes the policy and delegates screen enumeration/capture to an injectable `ScreenCaptureProvider`. The Agent Skill teaches autonomous engineering workflow and visual verification, while MCP instructions/prompts remain a fallback. Screenshot bytes are returned in-memory as PNG image content and are never persisted by the capture path.

**Tech Stack:** Rust/Tauri 2, rmcp 3.1, xcap, image/PNG encoding, React/TypeScript, Agent Skills (`SKILL.md`), PowerShell packaging script.

**Spec:** `docs/superpowers/specs/2026-08-26-agent-plugin-visual-awareness-design.md`

## Global Constraints

- `screen_capture_enabled` defaults to `false`.
- Unrestricted filesystem/execution access never implicitly enables screen capture.
- Pause always wins over screen capture.
- Screen capture is single-shot only; no recording, camera, microphone, keylogging, or GUI automation.
- Screenshot bytes are memory-only by default; audit stores metadata/hash, never pixels.
- The Skill is guidance only; backend policy is deterministic authority.
- Do not invent an undocumented OpenAI plugin manifest format.
- Preserve unrelated/concurrent user changes, including the existing `src-tauri/tests/fixtures/fake_sidecar.rs` diff.

---

### Task 1: Versioned agent settings and live policy state

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src/lib/types.ts`

**Interfaces:**
- Produces Rust enums `AgentMode::{Autonomous, Conservative}` and `CaptureScope::{WindowsAndDisplays, WindowsOnly}` with snake_case serde values.
- Produces `AgentPolicyState` with atomic/RwLock-backed live fields and `snapshot() -> AgentPolicySnapshot`.
- `AppSettings` v4 fields: `agent_mode`, `screen_capture_enabled`, `capture_scope`, `visual_verification`.

- [x] **Step 1: Write failing settings tests**

Extend `settings.rs` tests to assert v3 files acquire secure agent defaults and v4 settings round-trip all new fields:

```rust
assert_eq!(loaded.agent_mode, AgentMode::Autonomous);
assert!(!loaded.screen_capture_enabled);
assert_eq!(loaded.capture_scope, CaptureScope::WindowsAndDisplays);
assert!(loaded.visual_verification);
assert_eq!(loaded.version, 4);
```

- [x] **Step 2: Run the focused settings tests and confirm RED**

Run: `cargo test -p local-workspace-mcp-manager settings::tests:: --lib`
Expected: compile/test failure because the fields/enums do not exist.

- [x] **Step 3: Implement v4 settings + live state**

Add serde enums and defaults, bump `AppSettings::default().version` to 4, normalize enum values through serde rather than free-form strings, and add:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct AgentPolicySnapshot {
    pub mode: AgentMode,
    pub screen_capture_enabled: bool,
    pub capture_scope: CaptureScope,
    pub visual_verification: bool,
}
```

`AgentPolicyState::apply(&AppSettings)` updates the live values. Initialize it in `lib.rs` after settings load and update it in `commands::update_app_settings` after persistence succeeds.

- [x] **Step 4: Re-run focused tests and confirm GREEN**

Run: `cargo test -p local-workspace-mcp-manager settings::tests:: --lib`
Expected: PASS.

---

### Task 2: Agent Skill source and validator/package contract

**Files:**
- Create: `plugin/skills/local-workspace-agent/SKILL.md`
- Create: `plugin/skills/local-workspace-agent/references/workspace-workflow.md`
- Create: `plugin/skills/local-workspace-agent/references/security-contract.md`
- Create: `scripts/validate-agent-skill.py`
- Create: `scripts/package-agent-skill.ps1`
- Modify: `package.json`

**Interfaces:**
- Skill root name and frontmatter name are exactly `local-workspace-agent`.
- Package output: `dist/local-workspace-agent.zip` containing a top-level `local-workspace-agent/` directory.

- [x] **Step 1: Write validator first and run it against the missing skill**

Validator checks: required files, YAML delimiters, exact lowercase name, discovery-oriented description, required workflow tokens, visual-awareness/privacy clauses, no bypass language that grants authority, and referenced files exist.

Run: `python scripts/validate-agent-skill.py`
Expected: FAIL because `plugin/skills/local-workspace-agent/SKILL.md` is absent.

- [x] **Step 2: Author the minimal Skill and references**

The Skill body instructs the model to call `get_agent_policy` early, use `DISCOVER → PLAN → INSPECT → ACT → VERIFY → RECOVER → REPORT`, use visual tools only when relevant, and read the two references when doing edits/security-sensitive work. The description only identifies triggers such as working on a user-approved local codebase, debugging/building/reviewing it, or visually checking its UI.

- [x] **Step 3: Run validator and confirm GREEN**

Run: `python scripts/validate-agent-skill.py`
Expected: PASS.

- [x] **Step 4: Add deterministic packaging**

PowerShell removes any old archive, stages only the skill directory, and uses `Compress-Archive` to create `dist/local-workspace-agent.zip`. Add npm scripts:

```json
"validate:skill": "python scripts/validate-agent-skill.py",
"package:skill": "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-agent-skill.ps1"
```

- [x] **Step 5: Package and inspect archive entries**

Run: `npm run package:skill`
Expected: archive exists and contains `local-workspace-agent/SKILL.md` plus both references, with no source/build files.

---

### Task 3: MCP live `get_agent_policy`

**Files:**
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/mcp_integration.rs`

**Interfaces:**
- `WorkspaceServer::new(core, agent_policy, capture_provider)` becomes the explicit dependency constructor; production `serve` supplies live state/provider and tests supply fakes.
- Tool output includes `mode`, `paused`, `unrestricted_access`, `screen_capture_enabled`, `capture_scope`, `visual_verification`.

- [x] **Step 1: Add failing MCP discovery/policy tests**

Assert tool discovery includes `get_agent_policy` and a call returns safe defaults with `paused=false`, `unrestricted_access=false`, `mode="autonomous"`, `screen_capture_enabled=false`.

- [x] **Step 2: Run focused MCP test and confirm RED**

Run: `cargo test -p local-workspace-mcp-manager --test mcp_integration get_agent_policy`
Expected: FAIL because tool/constructor dependency does not exist.

- [x] **Step 3: Implement read-only audited policy tool**

The handler reads live atomics/state, includes Pause and unrestricted state, writes a fail-closed audit row with no secret/path content, then returns structured content.

- [x] **Step 4: Re-run focused tests and confirm GREEN**

Run the same focused test and confirm PASS.

---

### Task 4: Screen-capture abstraction and stable errors

**Files:**
- Create: `src-tauri/src/screen_capture.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `crates/core/src/error.rs`

**Interfaces:**

```rust
pub trait ScreenCaptureProvider: Send + Sync {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>, CoreError>;
    fn capture(&self, target_id: &str, max_edge: Option<u32>) -> Result<CapturedImage, CoreError>;
}
```

`CaptureTarget` carries a session-local id, kind, display label, geometry and optional application label. `CapturedImage` carries PNG bytes, original/returned dimensions and target metadata.

- [x] **Step 1: Add failing unit tests for provider-independent helpers and new error strings**

Test target filtering for `WindowsOnly`, max-edge resize math, target-not-found mapping, and exact stable strings:

```text
SCREEN_CAPTURE_DISABLED
SCREEN_CAPTURE_UNSUPPORTED
CAPTURE_TARGET_NOT_FOUND
SCREEN_CAPTURE_FAILED
```

- [x] **Step 2: Run focused tests and confirm RED**

Run: `cargo test -p core error::tests:: --lib && cargo test -p local-workspace-mcp-manager screen_capture::tests:: --lib`
Expected: FAIL before implementation.

- [x] **Step 3: Implement abstraction and Windows xcap provider**

Add `xcap` and image/PNG support. Enumerate monitors/windows, generate deterministic session-local ids from target kind + native id/geometry, capture one frame, resize only when requested, PNG-encode in memory. On non-Windows builds return `SCREEN_CAPTURE_UNSUPPORTED`.

- [x] **Step 4: Re-run focused tests and confirm GREEN**

Run focused tests; no test requires an interactive desktop.

---

### Task 5: MCP visual tools with image content and fail-closed audit

**Files:**
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/tests/mcp_integration.rs`

**Interfaces:**
- `list_capture_targets({})`
- `capture_screen({ target_id, max_edge? })`
- Both are denied by Pause; both are denied when capture is disabled; display targets are filtered when scope is `windows_only`.

- [x] **Step 1: Add fake provider and failing MCP tests**

The fake returns one display + one window and a tiny deterministic PNG. Tests assert:

1. both tools are discovered;
2. disabled capture returns `SCREEN_CAPTURE_DISABLED`;
3. Pause returns `ACCESS_PAUSED` even if capture is enabled;
4. windows-only filters displays;
5. successful capture includes an MCP image block whose MIME type is `image/png` and metadata text/structured content with SHA-256/dimensions;
6. audit row contains hash/byte count but not base64/image bytes.

- [x] **Step 2: Run visual MCP tests and confirm RED**

Run: `cargo test -p local-workspace-mcp-manager --test mcp_integration capture_`
Expected: FAIL because handlers are absent.

- [x] **Step 3: Implement visual handlers**

Enforce in this order: Pause → screen-capture enabled → scope/target validation → provider operation → audit write → return response. If audit write fails, return the audit error and discard the captured bytes.

Construct `CallToolResult` with an image `ContentBlock` plus human/structured metadata. Never log/store the image data.

- [x] **Step 4: Re-run visual MCP tests and confirm GREEN**

Run focused test set; all PASS.

---

### Task 6: Teach fallback MCP instructions about first-class Skill and visual evidence

**Files:**
- Modify: `src-tauri/src/agent_skills.rs`
- Modify: `src-tauri/tests/mcp_integration.rs`

**Interfaces:**
- Existing five MCP prompts stay stable.
- Server instructions add `get_agent_policy` and visual verification behavior without claiming the prompt layer controls security.

- [x] **Step 1: Strengthen existing instruction contract test and confirm RED**

Require strings/meaning for `get_agent_policy`, `capture_screen`, visual evidence relevance, and the rule forbidding shell-based screenshot bypass.

- [x] **Step 2: Update server instructions minimally**

Add policy discovery near DISCOVER and visual inspection under INSPECT/VERIFY. Explicitly state that disabled screen capture may not be bypassed using execution tools.

- [x] **Step 3: Re-run instruction tests and confirm GREEN**

Run the relevant MCP integration tests.

---

### Task 7: Desktop Agent settings UI

**Files:**
- Modify: `src/lib/types.ts`
- Modify: `src/pages/settings-page.tsx`
- Modify if necessary: `src/api.ts`

**Interfaces:**

```ts
export type AgentMode = "autonomous" | "conservative";
export type CaptureScope = "windows_and_displays" | "windows_only";
```

- [x] **Step 1: Extend TypeScript settings contract first**

Add v4 fields so existing JSX assignments fail until Settings UI handles the complete object consistently.

- [x] **Step 2: Add Agent card**

Use existing shadcn controls/styles. Include mode selector, screen capture switch, scope selector disabled when capture is off, visual verification switch, and explicit privacy warning that screen capture can expose content outside workspace folders.

- [x] **Step 3: Run frontend build and fix only task-related type/UI issues**

Run: `npm run build`
Expected: PASS with no TypeScript errors.

---

### Task 8: Documentation and plugin publication handoff

**Files:**
- Modify: `README.md`
- Create: `docs/plugin/CHATGPT_INSTALL.md`

**Interfaces:**
- Explain two testable artifacts: custom MCP app endpoint + `dist/local-workspace-agent.zip`.
- Explain ChatGPT Skills upload path and that real workspace/Directory plugin composition/publishing is an external ChatGPT operation.

- [x] **Step 1: Document install/update flow**

Cover desktop app install, enable workspace permissions, opt into screen capture, publish/refresh MCP app, upload Skill zip, install Skill, and low-risk E2E prompts.

- [x] **Step 2: Document screenshot privacy and capability behavior**

State that screen capture defaults off, unrestricted does not imply capture, Pause wins, screenshots are memory-only, and audit stores metadata/hash only.

- [x] **Step 3: Validate no docs claim an undocumented local plugin manifest exists**

Search docs for `plugin.json` / claims that local build alone publishes to the Plugin Directory; retain only the explicit non-invention warning.

---

### Task 9: Full verification and release artifact

**Files:**
- No new production files unless verification exposes a task-scoped defect.

- [x] **Step 1: Validate/package Skill**

Run:

```text
npm run validate:skill
npm run package:skill
```

Expected: PASS and `dist/local-workspace-agent.zip` exists.

- [x] **Step 2: Rust quality gates**

Run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all PASS.

- [x] **Step 3: Frontend + Go sidecar gates**

Run:

```text
npm run build
npm run build:sidecar
go test ./...
go vet ./...
```

Run the Go commands from `sidecars/tailscale-funnel`. Expected: all PASS.

- [x] **Step 4: Release packaging**

Run: `npx tauri build`
Expected: release app and NSIS installer generated successfully.

- [x] **Step 5: Git hygiene**

Run `git diff --check` and `git status --short`. Confirm `src-tauri/tests/fixtures/fake_sidecar.rs` remains preserved as an unrelated change and was not modified as part of this work.

- [x] **Step 6: External E2E remains explicit**

Record that local verification proves protocol/build behavior, not ChatGPT workspace publication. Manual acceptance is: upload/install Skill, refresh/publish Local Workspaces app, verify the Skill appears under Skills, run a local coding task, enable screen capture and verify a visual task receives image evidence, then disable capture and verify the stable denial.
