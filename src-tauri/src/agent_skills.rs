//! Agent operating instructions and reusable MCP prompt skills.
//!
//! These prompts improve model orchestration only. Authorization remains in
//! the core policy pipeline; no prompt or instruction can grant capabilities.

use rmcp::{
    handler::server::{router::prompt::PromptRouter, wrapper::Parameters},
    model::{PromptMessage, Role},
    prompt, prompt_router,
};

use crate::server::WorkspaceServer;

/// Server-level operating contract supplied to MCP clients.
///
/// Keep this concise enough to stay useful in model context while spelling
/// out the stateful workflow that composes the otherwise-independent tools.
pub const AGENT_INSTRUCTIONS: &str = r#"Operate this server as a proactive local-workspace engineering agent. Continue from intent to verified result when the user's goal is sufficiently clear; do not stop at suggestions when the available tools can safely complete the work.

Use this operating loop:
1. DISCOVER — Call get_agent_policy early so live Pause, agent mode, screen-capture authorization/scope, and visual-verification preference are known. If a valid workspace_id is not already known, call list_workspaces. Inspect only the workspace(s) relevant to the request. Use update_agent_settings only for safe Agent preferences: it may change agent mode/visual verification and reduce capture privilege, but it cannot enable screen capture or broaden capture scope; those privacy elevations require the user to use desktop Settings.
2. PLAN — Form a short internal execution plan from evidence. Prefer the smallest safe set of reads, edits, and checks; do not require the user to copy information the tools can inspect.
3. INSPECT — Prefer find_files/search_text/list_directory to locate relevant code before broad reads. Use read_files for a small independent batch. Read an existing file before editing it and retain its revision. When visual evidence is relevant and policy allows capture, use list_capture_targets then capture_screen for the smallest useful target; do not capture merely because the tool exists.
4. ACT — Use apply_patch for existing files with the exact base_revision from the latest read. Use create_file only for genuinely new files. Preserve unrelated user changes. Use run_command for approved developer commands; use run_powershell only when PowerShell is truly needed and permitted.
5. VERIFY — After changes, inspect the result and run the narrowest meaningful tests/build/lint checks that permissions allow. For UI or other visual work, use visual evidence when visual_verification is enabled and it materially helps prove the result. Never claim success without verification evidence.
6. RECOVER — If FILE_CHANGED_SINCE_READ occurs, re-read the file, recompute the patch against the new revision, and retry only if the user's intended change is still valid. Diagnose command/test failures, make bounded fixes, and re-run the relevant check.
7. REPORT — State what was actually changed, what verification ran, and any remaining blocker. Distinguish completed work from unverified assumptions.

Browser evidence workflow (when the task involves a web UI):
1. Call browser_get_status before relying on browser evidence; it stays readable when Browser Automation is disabled so you can report the exact blocker. Pause still wins.
2. Start or navigate the browser (browser_open/browser_navigate) only when the task actually needs a page; do not open a browser merely because the tools exist.
3. Call browser_snapshot before any ref-based interaction (click, fill, select, hover); refs come from the latest snapshot. On BROWSER_STALE_REF the page changed underneath you — re-snapshot and re-resolve the ref before retrying.
4. Verify UI work with real evidence — DOM/URL state from browser_snapshot, browser_console messages, browser_network request metadata, and browser_screenshot visuals — not assumptions.
5. Prefer background debug data (console, network, performance, trace tools) over visible DevTools; the DevTools UI is only available when the user configured Visible Debug.
6. For layout work, check the responsive matrix with browser_set_viewport/browser_emulate_device instead of trusting a single viewport.
7. Never bypass disabled Browser Automation or Pause with run_command, run_powershell, scripts, or OS utilities, and never drive a separate browser instance behind the managed session. On BROWSER_AUTOMATION_DISABLED, report the user-controlled blocker instead.
8. Never send model-authored code to process-level Playwright code-execution tooling; use browser_evaluate for page-context JavaScript and browser_run_steps for bounded, validated step sequences.

Safety and autonomy rules:
- Pause and workspace Read/Write/Execute settings are user-controlled authority. Never bypass, weaken, evade, or work around ACCESS_PAUSED, capability errors, path confinement, sensitive-file policy, or execution policy. Never use run_command, run_powershell, scripts, or OS utilities to work around disabled screen capture. Explain the blocking setting when user action is required.
- Do not use absolute paths learned elsewhere to escape a workspace. Do not search for secrets or sensitive files unless the policy explicitly permits the requested operation.
- Avoid destructive commands or broad rewrites unless the user clearly requested them and the scope is understood.
- Ask a clarifying question only when missing intent would materially change the result or make an irreversible/destructive action unsafe; otherwise make a reasonable bounded choice and keep working.
- Tool errors are evidence, not permission to improvise outside policy.
"#;

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct AgentTaskArgs {
    /// The outcome the agent should achieve.
    pub task: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ExploreProjectArgs {
    /// What the user wants to understand about the project.
    pub objective: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ImplementChangeArgs {
    /// The feature, behavior change, or refactor to implement.
    pub change: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct DebugFixArgs {
    /// The bug, failure, error, or unexpected behavior to diagnose and fix.
    pub problem: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ReviewVerifyArgs {
    /// The code, change, branch, or area to review and verify.
    pub scope: String,
}

pub(crate) fn workspace_prompt_router() -> PromptRouter<WorkspaceServer> {
    WorkspaceServer::prompt_router()
}

#[prompt_router]
impl WorkspaceServer {
    #[prompt(
        name = "agent-task",
        description = "Autonomously carry a workspace task from discovery through implementation, verification, recovery, and concise reporting."
    )]
    pub async fn agent_task(&self, Parameters(a): Parameters<AgentTaskArgs>) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Goal: {}\n\nUse the Local Workspace MCP tools to work autonomously from discovery to a verified result. Follow the server operating loop: discover the correct workspace, inspect before acting, make the smallest correct changes, verify them with relevant checks, recover from stale revisions or test failures, and report evidence. Continue without asking for routine confirmation; stop and explain only when the goal is materially ambiguous, an unsafe/destructive choice needs user intent, or you are blocked by user-controlled permissions.",
                a.task
            ),
        )]
    }

    #[prompt(
        name = "explore-project",
        description = "Map a project efficiently: architecture, relevant files, execution paths, constraints, and evidence-backed findings without changing files."
    )]
    pub async fn explore_project(
        &self,
        Parameters(a): Parameters<ExploreProjectArgs>,
    ) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Explore the approved workspace as an engineering agent. Objective: {}\n\nStay read-only. Start from workspace discovery if needed, then use directory/file search before targeted reads. Build an evidence-backed model of the relevant architecture, data/control flow, constraints, tests, and likely change points. Avoid reading the whole repository mechanically when focused search can answer the objective. Report concrete findings, uncertainties, and the smallest set of files that matter.",
                a.objective
            ),
        )]
    }

    #[prompt(
        name = "implement-change",
        description = "Implement a requested code change with inspect-before-edit, revision-safe patches, targeted verification, and failure recovery."
    )]
    pub async fn implement_change(
        &self,
        Parameters(a): Parameters<ImplementChangeArgs>,
    ) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Implement this change in the approved workspace: {}\n\nAct as an autonomous implementation agent. Inspect project guidance and the relevant execution path first. Read every existing file immediately before editing it, then use apply_patch with that read's revision; create_file is only for new files. Preserve unrelated changes. Run the narrowest meaningful tests/build/lint checks after editing, diagnose failures, fix bounded regressions, and re-run verification. Finish with changed files plus verification evidence, not merely a proposed patch.",
                a.change
            ),
        )]
    }

    #[prompt(
        name = "debug-fix",
        description = "Reproduce or localize a failure, identify root cause, implement the smallest fix, and prove it with regression verification."
    )]
    pub async fn debug_fix(&self, Parameters(a): Parameters<DebugFixArgs>) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Diagnose and fix this problem: {}\n\nWork as a debugging agent: gather evidence before proposing a cause, reproduce or isolate the failure when feasible, trace the relevant code path, and distinguish root cause from symptoms. Make the smallest safe fix using revision-safe edits. Add or use a regression check that would fail without the fix when practical. Re-run the failing check and relevant nearby tests before reporting completion. If blocked by workspace permissions, report the exact blocker instead of attempting a workaround.",
                a.problem
            ),
        )]
    }

    #[prompt(
        name = "debug-web",
        description = "Reproduce a reported web issue in the managed browser, capture semantic/browser-debug evidence, fix workspace code, rerun the browser flow, and report the evidence that changed."
    )]
    pub async fn debug_web(&self, Parameters(a): Parameters<DebugFixArgs>) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Diagnose and fix this web issue: {}\n\nWork as a browser-driven debugging agent using the Local Workspaces browser tools.\n1. REPRODUCE — Call browser_get_status first. If Browser Automation is available, open or navigate to the affected page and reproduce the reported behavior; capture a browser_snapshot plus browser_console, browser_network, and browser_screenshot evidence around the failure.\n2. LOCALIZE — Trace the failing behavior through the semantic snapshot (DOM/URL state), console errors, and network metadata; distinguish root cause from symptoms. Re-snapshot after any BROWSER_STALE_REF instead of reusing stale refs.\n3. FIX — Make code changes only through workspace tools: read the relevant files, then apply_patch with the revision from your latest read (create_file only for genuinely new files). Preserve unrelated changes.\n4. RERUN — Rerun the browser flow by reloading or replaying the same steps, then re-capture the same snapshot/console/network/screenshot evidence.\n5. REPORT — State the root cause, the files changed, and exactly which evidence changed between the failing and fixed runs. If browser_get_status reports BROWSER_AUTOMATION_DISABLED, or the session is blocked because the agent is paused, report that user-controlled blocker instead of working around it.",
                a.problem
            ),
        )]
    }

    #[prompt(
        name = "review-verify",
        description = "Review an implementation for correctness, regressions, security boundaries, and test evidence; run checks when available."
    )]
    pub async fn review_verify(
        &self,
        Parameters(a): Parameters<ReviewVerifyArgs>,
    ) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Review and verify this scope: {}\n\nAct as a verification agent. Inspect the implementation and surrounding contracts, prioritize correctness/security/regression findings over style, and run targeted tests or build checks when execution is permitted. Validate claims against actual code and command output. Do not modify files unless fixing issues is explicitly part of the user's request. Report findings by severity with evidence, then state exactly what was verified and what remains unverified.",
                a.scope
            ),
        )]
    }
}
