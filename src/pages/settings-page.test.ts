import { describe, expect, it } from "vitest";
import {
  BROWSER_NATIVE_CAPABILITY_COPY,
  REMOVE_RUNTIME_BODY,
  RESET_DATA_BODY,
} from "./settings-page";

describe("native Lightpanda settings copy", () => {
  it("describes exact native parity and session support", () => {
    expect(BROWSER_NATIVE_CAPABILITY_COPY).toEqual([
      "Lightpanda native MCP over a private local HTTP channel",
      "31/31 native MCP tools available when the pinned runtime contract is verified",
      "Native isolated browser sessions are supported",
      "Lightpanda has no graphical rendering or browser screenshots",
    ]);
  });

  it("distinguishes reset from runtime removal for PandaScripts", () => {
    expect(RESET_DATA_BODY).toMatch(/preserv.*PandaScript/i);
    expect(REMOVE_RUNTIME_BODY).toMatch(/remov.*PandaScript/i);
  });
});
