import { describe, expect, it, vi } from "vitest";
import {
  MAX_EVENTS,
  createAuditRefresh,
  mergeAuditEvents,
  subscribeToAuditEvents,
} from "./use-audit";
import type { AuditEvent } from "../api";

function ev(id: number, overrides: Partial<AuditEvent> = {}): AuditEvent {
  return {
    id,
    ts: "2026-08-26T00:00:00Z",
    source: "mcp",
    op: `op${id}`,
    decision: "allow",
    ...overrides,
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("mergeAuditEvents", () => {
  it("dedupes rows with the same persisted id", () => {
    const merged = mergeAuditEvents([ev(1), ev(2)], [ev(2), ev(3)]);
    expect(merged.map((e) => e.id)).toEqual([3, 2, 1]);
  });

  it("sorts out-of-order ids newest first", () => {
    const merged = mergeAuditEvents([], [ev(2), ev(9), ev(1), ev(5)]);
    expect(merged.map((e) => e.id)).toEqual([9, 5, 2, 1]);
  });

  it("keeps only the newest 100 rows", () => {
    const incoming = Array.from({ length: 150 }, (_, i) => ev(i + 1));
    const merged = mergeAuditEvents([], incoming);
    expect(merged).toHaveLength(MAX_EVENTS);
    expect(merged[0].id).toBe(150);
    expect(merged[MAX_EVENTS - 1].id).toBe(51);
  });

  it("stale snapshot cannot drop a newer live event", () => {
    const state = mergeAuditEvents([], [ev(105)]);
    const afterSnapshot = mergeAuditEvents(state, [ev(101), ev(102), ev(103), ev(104)]);
    expect(afterSnapshot.map((e) => e.id)).toEqual([105, 104, 103, 102, 101]);
  });

  it("stale snapshot cannot push newer live events past the cap", () => {
    const live = Array.from({ length: 100 }, (_, i) => ev(101 + i)); // 101..200
    const stale = Array.from({ length: 100 }, (_, i) => ev(50 + i)); // 50..149
    const merged = mergeAuditEvents(live, stale);
    expect(merged).toHaveLength(MAX_EVENTS);
    expect(merged[0].id).toBe(200);
    expect(merged[MAX_EVENTS - 1].id).toBe(101);
  });
});

describe("subscribeToAuditEvents", () => {
  it("registers the listener before the snapshot and keeps events emitted during it", async () => {
    let listener: ((e: AuditEvent) => void) | undefined;
    let resolveSnapshot!: (rows: AuditEvent[]) => void;
    const snapshot = new Promise<AuditEvent[]>((r) => (resolveSnapshot = r));
    const calls: string[] = [];
    let state: AuditEvent[] = [];

    subscribeToAuditEvents({
      listenAudit: async (onEvent) => {
        calls.push("listen");
        listener = onEvent;
        return () => {
          listener = undefined;
        };
      },
      fetchSnapshot: () => {
        calls.push("snapshot");
        return snapshot;
      },
      apply: (rows) => {
        state = mergeAuditEvents(state, rows);
      },
      reportError: () => {},
    });

    await vi.waitFor(() => expect(calls).toContain("snapshot"));
    expect(calls.indexOf("listen")).toBeLessThan(calls.indexOf("snapshot"));

    // Live event arrives while the snapshot fetch is still in flight.
    await vi.waitFor(() => expect(listener).toBeDefined());
    listener?.(ev(5));
    expect(state.map((e) => e.id)).toEqual([5]);

    // Snapshot lands later and overlaps the live event.
    resolveSnapshot([ev(1), ev(2), ev(5)]);
    await vi.waitFor(() => expect(state).toHaveLength(3));
    expect(state.map((e) => e.id)).toEqual([5, 2, 1]);
  });

  it("cleanup unsubscribes exactly once and stops further delivery", async () => {
    let listener: ((e: AuditEvent) => void) | undefined;
    const unsubscribe = vi.fn();
    let resolveSnapshot!: (rows: AuditEvent[]) => void;
    const snapshot = new Promise<AuditEvent[]>((r) => (resolveSnapshot = r));
    let state: AuditEvent[] = [];

    const dispose = subscribeToAuditEvents({
      listenAudit: async (onEvent) => {
        listener = onEvent;
        return unsubscribe;
      },
      fetchSnapshot: () => snapshot,
      apply: (rows) => {
        state = mergeAuditEvents(state, rows);
      },
      reportError: () => {},
    });

    await vi.waitFor(() => expect(listener).toBeDefined());
    dispose();
    dispose();
    expect(unsubscribe).toHaveBeenCalledTimes(1);

    listener?.(ev(9));
    resolveSnapshot([ev(1)]);
    await flush();
    expect(state).toEqual([]);
  });

  it("cleanup while listen is pending still releases the late listener", async () => {
    const unsubscribe = vi.fn();
    let resolveListen!: () => void;
    const listenPromise = new Promise<() => void>(
      (r) => (resolveListen = () => r(unsubscribe)),
    );
    const apply = vi.fn();

    const dispose = subscribeToAuditEvents({
      listenAudit: () => listenPromise,
      fetchSnapshot: async () => [ev(1)],
      apply,
      reportError: () => {},
    });

    dispose();
    expect(unsubscribe).not.toHaveBeenCalled();
    resolveListen();
    await flush();
    expect(unsubscribe).toHaveBeenCalledTimes(1);
    expect(apply).not.toHaveBeenCalled();
  });

  it("reports an error when the listener cannot be registered but still loads the snapshot", async () => {
    const reportError = vi.fn();
    const fetchSnapshot = vi.fn(async () => [ev(1), ev(2)]);
    let state: AuditEvent[] = [];

    subscribeToAuditEvents({
      listenAudit: async () => {
        throw new Error("listener unavailable");
      },
      fetchSnapshot,
      apply: (rows) => {
        state = mergeAuditEvents(state, rows);
      },
      reportError,
    });

    await vi.waitFor(() => expect(reportError).toHaveBeenCalledTimes(1));
    expect(fetchSnapshot).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(state.map((e) => e.id)).toEqual([2, 1]));
  });

  it("reports an error when the snapshot fetch fails", async () => {
    const reportError = vi.fn();
    const apply = vi.fn();

    subscribeToAuditEvents({
      listenAudit: async () => () => {},
      fetchSnapshot: async () => {
        throw new Error("snapshot unavailable");
      },
      apply,
      reportError,
    });

    await vi.waitFor(() => expect(reportError).toHaveBeenCalledTimes(1));
    expect(apply).not.toHaveBeenCalled();
  });
});

describe("createAuditRefresh (manual refresh path)", () => {
  it("merges fetched rows into the current state instead of replacing it", async () => {
    // A newer live event is already in state; the refresh returns stale rows.
    let state: AuditEvent[] = [ev(105)];
    const apply = (update: (events: AuditEvent[]) => AuditEvent[]) => {
      state = update(state);
    };

    const refresh = createAuditRefresh(
      apply,
      async () => [ev(101), ev(102), ev(103)],
      () => {},
    );
    await refresh();

    expect(state.map((e) => e.id)).toEqual([105, 103, 102, 101]);
  });

  it("dedupes rows already present in state", async () => {
    let state: AuditEvent[] = [ev(2), ev(1)];
    const apply = (update: (events: AuditEvent[]) => AuditEvent[]) => {
      state = update(state);
    };

    const refresh = createAuditRefresh(apply, async () => [ev(2), ev(3)], () => {});
    await refresh();

    expect(state.map((e) => e.id)).toEqual([3, 2, 1]);
  });

  it("reports an error and leaves state untouched when the fetch fails", async () => {
    let state: AuditEvent[] = [ev(1)];
    const apply = vi.fn((update: (events: AuditEvent[]) => AuditEvent[]) => {
      state = update(state);
    });
    const reportError = vi.fn();

    const refresh = createAuditRefresh(
      apply,
      async () => {
        throw new Error("boom");
      },
      reportError,
    );
    await refresh();

    expect(reportError).toHaveBeenCalledTimes(1);
    expect(reportError.mock.calls[0][0]).toContain("boom");
    expect(apply).not.toHaveBeenCalled();
    expect(state.map((e) => e.id)).toEqual([1]);
  });
});
