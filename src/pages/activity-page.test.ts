import { describe, expect, it } from "vitest";
import { matchesFilter } from "./activity-page";
import type { AuditEvent } from "../api";

function ev(overrides: Partial<AuditEvent>): AuditEvent {
  return {
    id: 1,
    ts: "2026-08-26T00:00:00Z",
    source: "mcp",
    op: "read_file",
    decision: "allow",
    ...overrides,
  };
}

describe("activity filter tabs", () => {
  it("blocked shows deny rows and errors does not claim them", () => {
    const denied = ev({ decision: "deny", error_code: "access_denied" });
    expect(matchesFilter(denied, "blocked")).toBe(true);
    expect(matchesFilter(denied, "errors")).toBe(false);
  });

  it("errors shows only rows whose decision is error", () => {
    const failed = ev({ decision: "error", error_code: "io_error" });
    expect(matchesFilter(failed, "errors")).toBe(true);
    expect(matchesFilter(failed, "blocked")).toBe(false);
    expect(matchesFilter(failed, "allowed")).toBe(false);
  });

  it("an error decision counts as an error even without an error_code", () => {
    const failed = ev({ decision: "error", error_code: null });
    expect(matchesFilter(failed, "errors")).toBe(true);
  });

  it("allowed shows allow rows", () => {
    expect(matchesFilter(ev({ decision: "allow" }), "allowed")).toBe(true);
    expect(matchesFilter(ev({ decision: "allow" }), "blocked")).toBe(false);
    expect(matchesFilter(ev({ decision: "allow" }), "errors")).toBe(false);
  });

  it("all shows every row", () => {
    for (const decision of ["allow", "deny", "error"]) {
      expect(matchesFilter(ev({ decision }), "all")).toBe(true);
    }
  });
});
