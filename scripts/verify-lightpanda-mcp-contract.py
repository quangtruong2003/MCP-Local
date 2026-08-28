#!/usr/bin/env python3
"""Static verifier for the frozen Lightpanda 0.3.7 native MCP contract.

Checks the bundled tools/list snapshot and the Rust mapping constant against
the invariants required by the native parity migration:

- snapshot metadata matches the pinned runtime (0.3.7, 31 tools);
- tool names are exactly the expected 31 and unique;
- no upstream input schema owns the reserved ``session_id`` property;
- ``NATIVE_TOOL_MAPPINGS`` in ``src/browser/contract.rs`` is one-to-one,
  uses only the ``browser_lightpanda_`` prefix, and follows the
  camelCase-to-snake_case naming rule.

Exits non-zero on any violation.
"""

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CONTRACT_JSON = REPO_ROOT / "src-tauri" / "resources" / "lightpanda-mcp-contract-0.3.7.json"
CONTRACT_RS = REPO_ROOT / "src-tauri" / "src" / "browser" / "contract.rs"

EXPECTED_VERSION = "0.3.7"
EXPECTED_TOOL_COUNT = 31
EXPECTED_NAMES = {
    "goto", "search", "markdown", "html", "links", "evaluate", "extract",
    "tree", "nodeDetails", "interactiveElements", "structuredData", "detectForms",
    "click", "fill", "scroll", "waitForSelector", "waitForScript", "waitForState",
    "hover", "press", "selectOption", "setChecked", "findElement", "consoleLogs",
    "getUrl", "getCookies", "getEnv", "save", "session_new", "session_list",
    "session_close",
}
PUBLIC_PREFIX = "browser_lightpanda_"


def fail(message: str) -> None:
    print(f"FAIL: {message}")
    sys.exit(1)


def camel_to_snake(name: str) -> str:
    return re.sub(r"(?<=[a-z0-9])([A-Z])", r"_\1", name).lower()


def iter_schema_properties(schema):
    """Yield (tool_name, property_name) for every property in a schema,
    recursing into nested object schemas."""
    if not isinstance(schema, dict):
        return
    properties = schema.get("properties")
    if isinstance(properties, dict):
        for prop_name, prop_schema in properties.items():
            yield prop_name
            if isinstance(prop_schema, dict):
                yield from iter_schema_properties(prop_schema)


def verify_snapshot() -> None:
    if not CONTRACT_JSON.is_file():
        fail(f"contract snapshot missing: {CONTRACT_JSON}")
    contract = json.loads(CONTRACT_JSON.read_text(encoding="utf-8"))

    version = contract.get("lightpanda_version")
    if version != EXPECTED_VERSION:
        fail(f"lightpanda_version must be {EXPECTED_VERSION}, found {version!r}")

    tools = contract.get("tools")
    if not isinstance(tools, list):
        fail("contract snapshot must contain a tools array")

    if contract.get("tool_count") != EXPECTED_TOOL_COUNT:
        fail(f"tool_count must be {EXPECTED_TOOL_COUNT}, found {contract.get('tool_count')!r}")
    if len(tools) != EXPECTED_TOOL_COUNT:
        fail(f"tools array must hold {EXPECTED_TOOL_COUNT} entries, found {len(tools)}")

    names = [tool.get("name") for tool in tools]
    if len(set(names)) != len(names):
        duplicates = sorted({n for n in names if names.count(n) > 1})
        fail(f"duplicate tool names in snapshot: {duplicates}")

    if set(names) != EXPECTED_NAMES:
        missing = sorted(EXPECTED_NAMES - set(names))
        unexpected = sorted(set(names) - EXPECTED_NAMES)
        fail(f"tool name set drift: missing={missing} unexpected={unexpected}")

    for tool in tools:
        if "session_id" in iter_schema_properties(tool.get("inputSchema", {})):
            fail(f"upstream tool {tool['name']} owns reserved session_id property")
        if "outputSchema" in tool:
            fail(f"pinned 0.3.7 must not advertise outputSchema (found on {tool['name']})")


def verify_mappings() -> None:
    if not CONTRACT_RS.is_file():
        fail(f"contract module missing: {CONTRACT_RS}")
    source = CONTRACT_RS.read_text(encoding="utf-8")

    match = re.search(
        r"pub const NATIVE_TOOL_MAPPINGS: &\[\(&str, &str\)\] = &\[(.*?)\];",
        source,
        re.DOTALL,
    )
    if not match:
        fail("NATIVE_TOOL_MAPPINGS constant not found in contract.rs")

    pairs = re.findall(r'\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)', match.group(1))
    if len(pairs) != EXPECTED_TOOL_COUNT:
        fail(f"NATIVE_TOOL_MAPPINGS must hold {EXPECTED_TOOL_COUNT} pairs, found {len(pairs)}")

    upstream_names = [upstream for upstream, _ in pairs]
    public_names = [public for _, public in pairs]
    if len(set(upstream_names)) != len(upstream_names):
        fail("duplicate upstream names in NATIVE_TOOL_MAPPINGS")
    if len(set(public_names)) != len(public_names):
        fail("duplicate public names in NATIVE_TOOL_MAPPINGS")
    if set(upstream_names) != EXPECTED_NAMES:
        fail("NATIVE_TOOL_MAPPINGS upstream names do not match the frozen snapshot")

    for upstream, public in pairs:
        if not public.startswith(PUBLIC_PREFIX):
            fail(f"public name {public} lacks the {PUBLIC_PREFIX} prefix")
        expected_public = PUBLIC_PREFIX + camel_to_snake(upstream)
        if public != expected_public:
            fail(f"mapping {upstream} -> {public} violates naming rule (expected {expected_public})")


def main() -> None:
    verify_snapshot()
    verify_mappings()
    print(
        f"Lightpanda {EXPECTED_VERSION} MCP contract OK: "
        f"{EXPECTED_TOOL_COUNT} expected tools, unique names, "
        f"no session_id collision, mappings one-to-one."
    )


if __name__ == "__main__":
    main()
