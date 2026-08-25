# Workspace engineering workflow

Use this reference for implementation, debugging, refactoring, review, and verification tasks.

## Evidence before edits

- Narrow the relevant area with search/list/find before broad reads.
- Read project guidance files when they exist and matter to the task.
- **Read before patch.** An existing file must be read immediately enough to obtain the revision the patch is based on.
- Keep the change bounded to the user's outcome. **Preserve unrelated changes**, including concurrent edits already present in the working tree.

## Revision-safe editing

1. Read the current file and retain its revision.
2. Form the smallest unique replacement that expresses the intended change.
3. Call `apply_patch` with that exact revision.
4. If the server returns `FILE_CHANGED_SINCE_READ`, do not overwrite or reuse the stale patch mechanically.
5. Re-read the file, compare the new state with the intended outcome, recompute the patch, and retry only when the intent still applies cleanly.

Use `create_file` only for a path that does not already exist. Never recreate an existing file just to avoid revision checks.

## Debugging discipline

**Root cause before fix.** Gather the actual error/output, reproduce or isolate it when feasible, trace the relevant code/data path, and state a single evidence-based hypothesis before changing production behavior. Prefer the smallest change that addresses the source rather than suppressing a symptom.

For a bug fix, add or use a regression check that would fail without the fix when practical. If a proposed fix fails, return to evidence gathering instead of stacking guesses.

## TDD for feature/behavior work

When the repository and task permit automated tests:

1. RED — add the smallest test for the desired behavior.
2. Run it and confirm it fails for the expected missing behavior, not from a typo/setup error.
3. GREEN — implement the minimum production change.
4. Run the focused test and nearby tests.
5. REFACTOR only while the tests remain green.

Do not create artificial tests that merely assert implementation details when real behavior can be tested.

## Verification before completion

**Verification before completion** means fresh evidence, not confidence from reading the patch.

Select checks proportional to the change:

- focused unit/integration test first;
- typecheck/lint when the changed language/toolchain uses them;
- build when integration or packaging could fail;
- visual check when the user-visible result is rendered UI and runtime policy permits it.

Read command output. A zero exit code is necessary but not always sufficient if output reports skipped/ignored checks relevant to the claim.

## Autonomous vs conservative mode

`get_agent_policy.mode` is behavioral guidance, not authorization.

- `autonomous`: continue through routine implementation and verification without requesting confirmation when the goal and safe scope are clear.
- `conservative`: ask before materially consequential choices when multiple reasonable outcomes remain and the user's request does not select one.

Both modes obey the same backend permissions and non-bypass rules.
