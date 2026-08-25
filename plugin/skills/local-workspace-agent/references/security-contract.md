# Local Workspaces security contract

This skill never grants authority. The Rust/Tauri backend and the settings chosen by the user are the authority.

## Non-bypass rules

- **Pause always wins.** On `ACCESS_PAUSED`, stop MCP activity and explain that the user paused access.
- Workspace Read/Write/Execute capabilities, path confinement, sensitive-file checks, execution mode, and command policy are deterministic backend boundaries.
- Do not leave an approved workspace by using absolute paths, traversal, junctions/symlinks, shell indirection, or a helper process.
- A denial is not permission to try an equivalent action through another tool.
- Unrestricted access relaxes the documented filesystem/execution gates only. **unrestricted access does not enable screen capture**.

## Screen capture privacy

A screenshot can contain data unrelated to an approved project: email, notifications, credentials, chats, browser tabs, or another application. Treat it as a separate privacy capability.

- Capture only when `get_agent_policy.screen_capture_enabled` is true.
- Respect `capture_scope`. When the scope is `windows_only`, do not seek a full-display equivalent.
- Prefer the narrowest relevant application window when it can answer the question.
- Do not capture repeatedly without a concrete visual hypothesis or verification need.
- **Do not use run_command or run_powershell** (or scripts/OS screenshot utilities launched through them) to work around disabled, unsupported, paused, or scope-restricted capture.
- The MCP capture path is designed so screenshots are not persisted to disk by default. Assume **screenshots are not persisted** unless the user explicitly asks to save an image through a separate allowed workflow.

## Permission errors

When a backend permission blocks the requested outcome:

1. Preserve the exact stable error/code when reporting it.
2. Explain which desktop setting controls that permission when known.
3. Continue with narrower evidence only if it can genuinely satisfy the user's goal.
4. Do not suggest weakening unrelated protections as the default solution.

## Destructive work

Avoid deletion, broad rewrites, destructive commands, credential changes, resets, or irreversible migrations unless clearly requested and scoped. If user intent does not resolve a destructive choice, ask before acting even in autonomous mode.
