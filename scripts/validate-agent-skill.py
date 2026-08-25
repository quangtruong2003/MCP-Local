from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKILL_ROOT = ROOT / "plugin" / "skills" / "local-workspace-agent"
SKILL = SKILL_ROOT / "SKILL.md"
REQUIRED_REFERENCES = [
    SKILL_ROOT / "references" / "workspace-workflow.md",
    SKILL_ROOT / "references" / "security-contract.md",
]


def fail(message: str) -> None:
    print(f"skill validation failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def parse_frontmatter(text: str) -> tuple[dict[str, str], str]:
    if not text.startswith("---\n"):
        fail("SKILL.md must start with YAML frontmatter")
    end = text.find("\n---\n", 4)
    if end < 0:
        fail("SKILL.md frontmatter must end with ---")
    raw = text[4:end]
    body = text[end + 5 :]
    values: dict[str, str] = {}
    for line in raw.splitlines():
        if not line.strip():
            continue
        if ":" not in line:
            fail(f"unsupported frontmatter line: {line}")
        key, value = line.split(":", 1)
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values, body


def main() -> None:
    if not SKILL.is_file():
        fail(f"missing {SKILL.relative_to(ROOT)}")
    for path in REQUIRED_REFERENCES:
        if not path.is_file():
            fail(f"missing {path.relative_to(ROOT)}")

    text = SKILL.read_text(encoding="utf-8")
    frontmatter, body = parse_frontmatter(text)
    name = frontmatter.get("name", "")
    description = frontmatter.get("description", "")

    if name != SKILL_ROOT.name:
        fail("frontmatter name must exactly match the skill directory")
    if not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", name):
        fail("name must contain lowercase letters/numbers and single hyphens only")
    if len(name) > 64:
        fail("name exceeds Agent Skills 64-character limit")
    if not description.startswith("Use when "):
        fail("description must be discovery-oriented and start with 'Use when '")
    if len(description) > 500:
        fail("description is too long for efficient skill discovery")
    if any(token in description.upper() for token in ("DISCOVER", "PLAN", "INSPECT", "ACT", "VERIFY", "RECOVER", "REPORT")):
        fail("description must not summarize the workflow; keep process in the skill body")

    required_body_terms = [
        "DISCOVER",
        "PLAN",
        "INSPECT",
        "ACT",
        "VERIFY",
        "RECOVER",
        "REPORT",
        "get_agent_policy",
        "list_workspaces",
        "FILE_CHANGED_SINCE_READ",
        "capture_screen",
        "screen capture",
        "Never bypass",
        "references/workspace-workflow.md",
        "references/security-contract.md",
    ]
    for term in required_body_terms:
        if term not in body:
            fail(f"SKILL.md missing required contract term: {term}")

    security = REQUIRED_REFERENCES[1].read_text(encoding="utf-8")
    for phrase in (
        "Pause always wins",
        "unrestricted access does not enable screen capture",
        "Do not use run_command or run_powershell",
        "screenshots are not persisted",
    ):
        if phrase not in security:
            fail(f"security-contract.md missing: {phrase}")

    workflow = REQUIRED_REFERENCES[0].read_text(encoding="utf-8")
    for phrase in (
        "Read before patch",
        "Preserve unrelated changes",
        "Root cause before fix",
        "Verification before completion",
    ):
        if phrase not in workflow:
            fail(f"workspace-workflow.md missing: {phrase}")

    print("local-workspace-agent skill validation: OK")


if __name__ == "__main__":
    main()
