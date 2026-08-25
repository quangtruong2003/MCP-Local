# ChatGPT install and publication handoff

**Last verified:** 2026-08-26

Local Workspaces ships two independently testable artifacts:

1. **Local Workspaces MCP app** — the Tailscale Funnel secret capability endpoint backed by the desktop app.
2. **Local Workspace Agent Skill** — `dist/local-workspace-agent.zip`, produced from `plugin/skills/local-workspace-agent/`.

The local repository can build and validate both artifacts. Combining, sharing, or publishing them as a ChatGPT workspace plugin is a separate ChatGPT operation and is not proven by local tests.

## 1. Build and start Local Workspaces

Install the desktop app or run a release/development build. Complete onboarding, add only the folders you want to expose as workspaces, and enable Write/Execute only where needed.

Start the connection from the Dashboard. When it is Online, use the secret Tailscale Funnel capability endpoint as the MCP URL for the Local Workspaces custom app in ChatGPT. Treat this URL like a password and rotate it if it is exposed.

The desktop app remains the security authority. ChatGPT Skill instructions do not grant filesystem, execution, or screen-capture permissions.

## 2. Validate and package the Agent Skill

From the repository root:

```powershell
npm run validate:skill
npm run package:skill
```

The expected artifact is:

```text
dist/local-workspace-agent.zip
└─ local-workspace-agent/
   ├─ SKILL.md
   └─ references/
      ├─ workspace-workflow.md
      └─ security-contract.md
```

The package intentionally contains the Skill only. It does not contain the desktop application, source tree, build output, secrets, or an invented plugin manifest.

## 3. Upload/install the Skill in ChatGPT

Current ChatGPT navigation as of 2026-08-26 is **Plugins → Skills**. To upload the packaged Skill, choose **Create → Upload from your computer** and select `dist/local-workspace-agent.zip`.

Workspace administrators may need to enable Skills and skill uploading for the relevant role. Availability can also depend on plan, workspace configuration, surface, and region.

After upload, install/enable the Skill for the account or role that will use Local Workspaces.

## 4. Configure the Local Workspaces app

Create or refresh the custom MCP app/connection using the Online secret capability endpoint from Local Workspaces. Workspace admins manage app availability and permissions independently from Skill availability.

A published custom MCP app does not by itself prove that a separate plugin containing that app and this Skill has been published. The app and Skill remain independently manageable capabilities.

## 5. Share or publish as a workspace plugin

ChatGPT's Plugin Directory can package workflow capabilities that include Skills and Apps. Workspace sharing/publication is performed in ChatGPT and depends on the account's workspace permissions.

For a plugin owned in the workspace, the current workspace flow is under **Plugins**. Use the plugin's sharing controls to choose invited users, workspace members with the link, or (when permitted) visibility in the workspace directory. Admin permissions such as **Use plugins**, **Share plugins**, and **Publish plugins to workspace** govern what members can do.

Do not treat a local zip, a repository folder, or a made-up `plugin.json` as proof that a workspace/public Plugin Directory listing exists. This repository deliberately does not define an undocumented plugin manifest format.

## Screen-capture privacy contract

Screen capture is a separate privacy capability because a screenshot may reveal content outside approved workspace folders.

- It defaults to **off**.
- **Unrestricted access does not enable screen capture**.
- **Pause always blocks capture**, even when capture is enabled.
- Capture is single-shot only; there is no recording, camera, microphone, keyboard logging, or GUI automation in this feature.
- The capture path returns PNG bytes in memory and does not persist screenshots to disk by default.
- Audit records store safe metadata such as target id/kind, dimensions, encoded byte count, and SHA-256. They do not store PNG/base64 pixels or window-title/application text.
- `windows_only` is the narrower scope and hides full-display targets.
- The Agent Skill must not use shell commands, PowerShell, helper scripts, or OS utilities to bypass disabled screen capture.

## Low-risk acceptance checks

After installing the Skill and configuring the app, verify these manually in ChatGPT:

1. Ask for a read-only project overview. Confirm the agent discovers Local Workspaces and reads only the approved workspace.
2. Ask for a small code change with a targeted test. Confirm revision-safe edits and verification are used.
3. With screen capture disabled, request visual inspection. Confirm the stable `SCREEN_CAPTURE_DISABLED` denial is respected rather than bypassed.
4. Enable screen capture and choose the desired scope. Ask for a visual UI check and confirm an image is received through `capture_screen`.
5. Turn on Pause and repeat a capture request. Confirm `ACCESS_PAUSED` wins.
6. Disable screen capture again when visual access is no longer needed.

## What local verification proves

The release gates validate the Skill structure/package, Rust protocol behavior, permission ordering, image-content response, audit privacy, frontend settings build, sidecar tests, and desktop packaging.

They do **not** prove:

- that a particular ChatGPT workspace has enabled Skills, custom apps, or plugin publication;
- that the Skill has been installed for a specific user/role;
- that the Local Workspaces app has been shared/published in that workspace;
- that a plugin is visible in a workspace or universal Plugin Directory.

Those are external acceptance steps performed in ChatGPT after the local build is complete.
