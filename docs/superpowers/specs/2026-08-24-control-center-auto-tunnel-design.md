# Local Workspace MCP Manager — Control Center + Auto Tunnel Design

Date: 2026-08-24
Status: Approved for implementation

## 1. Goal

Upgrade the Windows-first Local Workspace MCP Manager from an MVP single-screen utility into a complete desktop control center with a clear Start/Stop lifecycle, automatic Secure MCP Tunnel provisioning on first use, persistent tunnel reuse, trustworthy readiness reporting, improved workspace/security/activity UX, and safer credential handling.

## 2. Product principles

- One obvious primary action: **Start** when offline, **Stop** when online.
- Reuse the same remote tunnel whenever possible. A normal Start never creates a new tunnel if a saved tunnel_id exists and remains valid.
- Never label the tunnel Ready merely because the sidecar process spawned. Readiness is based on tunnel-client health/readiness endpoints.
- Keep the existing filesystem security invariants: approved-workspace confinement, read/write capability gates, sensitive-file deny list, optimistic concurrency, bounded results, append-only audit.
- Keep MCP server loopback-only.
- Do not expose a Cloudflare Quick Tunnel URL or public localhost URL as the primary connection model. The stable connector identity is the OpenAI tunnel ID.
- Separate admin credentials used for tunnel CRUD from runtime credentials used by the long-lived sidecar.

## 3. Official tunnel behavior implemented

The app integrates with the bundled `tunnel-client` CLI.

### First start when no tunnel exists

1. Validate required setup values:
   - runtime API key
   - admin API key
   - at least one scope: organization_id or workspace_id
2. Run `tunnel-client admin --json tunnels create` with:
   - a stable app-generated name
   - a descriptive label
   - the configured organization/workspace scope
   - `OPENAI_ADMIN_KEY` passed through the environment, never argv
3. Parse the JSON response and persist `tunnel_id`.
4. Start the runtime with the saved tunnel ID and runtime key.
5. Configure tunnel-client to bind its operator/health listener to `127.0.0.1:0` and write the resolved base URL to an app-data health URL file.
6. Poll `/healthz` and `/readyz` with a bounded startup timeout. Newly created tunnels may take roughly 25–30 seconds before becoming active; the UI communicates this as provisioning rather than presenting an error prematurely.
7. Transition to Ready only after `/readyz` returns HTTP 200.

### Subsequent starts

1. Load the saved tunnel ID.
2. Validate it with `tunnel-client admin --json tunnels get <tunnel_id>` using the runtime key.
3. If it is valid, start the runtime directly and reuse it.
4. If the saved tunnel no longer exists, transition to `needs_setup` with a recoverable message; do not silently create a replacement unless the user explicitly uses Regenerate or normal Start is configured to recover a missing remote tunnel using the existing saved admin setup.

### Regenerate

Settings exposes an explicit **Regenerate tunnel** action. It is distinct from normal Start and clearly warns that ChatGPT connector configuration may need to be updated to the new tunnel ID.

## 4. Connection state machine

Backend state is serialized as a structured snapshot instead of a lowercase enum string.

States:

- `stopped`
- `starting_server`
- `validating_tunnel`
- `provisioning_tunnel`
- `connecting`
- `checking_health`
- `ready`
- `stopping`
- `needs_setup`
- `error`

Connection snapshot fields:

- state
- tunnel_id? (safe to expose)
- local_mcp_url
- health_url?
- started_at?
- ready_at?
- message?
- error_code?
- error_detail?

The frontend receives current state through a Tauri command and live updates through a `connection-status` event.

## 5. Settings and secrets

Non-secret settings persisted in app data:

- tunnel_id
- organization_id?
- workspace_id?
- tunnel_name
- auto_start (default false)

Secret fields:

- runtime_api_key
- admin_api_key

For this implementation phase, introduce a `SecretStore` abstraction. On Windows the production implementation uses DPAPI through the `windows` crate to encrypt secret payloads before writing them to app data. No key may be returned by a Tauri read command. Tests use an in-memory/test implementation where appropriate.

Legacy `settings.json` containing `tunnel_api_key` is migrated on load: the value is imported as the runtime key into the protected secret store, removed from settings JSON, and the existing tunnel_id is preserved.

## 6. Backend modules

### `settings.rs`

Owns non-secret settings, migration, scope validation, atomic persistence.

### `secrets.rs`

Owns protected runtime/admin API key storage. Public operations are save/update/has/delete; there is no UI read-back command.

### `tunnel.rs`

Owns sidecar path resolution and the low-level process handle only. It can:

- create/get tunnel through child CLI commands
- spawn runtime with health URL file
- stop runtime
- expose captured process exit/status information

### `connection.rs`

Owns the state machine and orchestration:

- start
- stop
- regenerate
- status snapshot
- readiness polling
- event emission

No React component deals with process details.

## 7. Tauri commands

Replace the old tunnel-centric surface with:

- `connection_status`
- `start_connection`
- `stop_connection`
- `save_connection_settings`
- `connection_settings_summary`
- `regenerate_tunnel`

Keep temporary compatibility wrappers only where required during the migration; frontend should use the new API.

Settings summary returns booleans such as `has_runtime_key`, `has_admin_key` but never secret values.

## 8. Audit improvements

MCP operations should record the requested relative path where applicable and useful bounded detail:

- read/list/find/search/create/patch path or query scope
- bytes or matches where useful
- stable error code on failure

No sensitive file contents or credentials are logged.

The append-only database invariant remains unchanged.

## 9. UX architecture

Single window with a persistent left navigation:

- Dashboard
- Workspaces
- Activity
- Security
- Settings

### Dashboard

Offline hero:

- status icon + `MCP Offline`
- concise explanation
- primary `Start` button
- workspace/tool counts

Starting/provisioning:

- visible step/status text
- disabled destructive actions
- explain first-time tunnel provisioning may need several seconds

Ready:

- `Ready` status
- Tunnel ID + Copy button
- local MCP URL
- uptime
- Stop button
- summary cards: Workspaces, Tools, Recent Requests

Needs setup:

- concise reason
- primary action `Configure connection`

Error:

- human-readable summary
- `Retry` and `Open settings`
- optional technical details area

### Workspaces

Card/list view with:

- workspace name/path
- active read/write status
- Read and Write toggles
- remove action
- add-folder action
- helper copy explaining write risk

### Activity

Timeline/table hybrid showing latest audit events with:

- time
- operation
- workspace
- decision badge
- path/error code

### Security

Explain and expose current security controls:

- global Pause/Resume
- workspace confinement
- symlink/junction protection
- sensitive-file deny list
- optimistic concurrency
- atomic writes

This page is explanatory and operational, not decorative.

### Settings

Connection section contains:

- organization ID
- workspace ID
- runtime API key (replace-only password input)
- admin API key (replace-only password input)
- current saved Tunnel ID
- Regenerate tunnel

No credential field is prefilled with the stored secret.

## 10. Visual system

- Desktop-native, restrained control-center aesthetic.
- Clear page hierarchy instead of one long vertical document.
- Neutral surfaces, one accent, semantic success/warning/danger tokens.
- Consistent 8–12px corner radius; no excessive gradients/glass effects.
- Accessible focus states, button hierarchy, labels, disabled/busy states, keyboard navigation.
- CSS classes/design tokens replace the bulk of inline styles.
- Responsive enough for the existing 900×700 default window and larger sizes.

## 11. Reliability and process lifecycle

- Runtime stdout/stderr is captured to bounded app-data log files or drained by reader threads so the child never deadlocks on full pipes.
- Health URL file is unique to the app instance and removed/replaced safely.
- Normal app exit stops the runtime.
- A Windows Job Object with kill-on-close is preferred for sidecar orphan prevention; if this cannot be introduced without destabilizing the release, the process-handle cleanup remains and the limitation is documented.
- Start and Stop are idempotent.
- Concurrent Start requests collapse to one state-machine transition instead of spawning duplicate runtimes.

## 12. Tests and acceptance criteria

Backend tests cover:

- settings migration preserves old tunnel ID and removes plaintext key from JSON
- setup validation requires runtime key and scope; admin key only when provisioning/regenerating
- tunnel create JSON parsing
- saved tunnel reuse path does not invoke create
- missing saved tunnel recovery path
- state transitions for start/ready/stop/error
- readiness treats `/healthz` as liveness only and `/readyz` as Ready
- secrets are never returned by settings summary
- process args contain env references/IDs but not secret values

Frontend build/type checks must pass.

Manual acceptance:

1. Fresh app → configure credentials/scope → Start → tunnel created once → Ready → tunnel ID shown.
2. Stop → Start → same tunnel ID reused.
3. Restart app → Start → same tunnel ID reused.
4. Write off → ChatGPT write tool receives WRITE_DISABLED.
5. Pause → next MCP filesystem call receives ACCESS_PAUSED.
6. Ready label never appears before readiness probe succeeds.
7. No API key appears in settings JSON, command line arguments, UI readback, audit rows, or normal logs.
