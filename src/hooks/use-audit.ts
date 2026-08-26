import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, parseErrorMessage } from "../api";
import type { AuditEvent } from "../api";

export const MAX_EVENTS = 100;

/** Merges incoming audit rows into the current state by persisted id:
 * dedupes duplicate ids, sorts newest (highest id) first, caps at
 * MAX_EVENTS. The merge is a union, so a stale snapshot can never drop a
 * newer live event already in state. */
export function mergeAuditEvents(
  events: AuditEvent[],
  incoming: AuditEvent[],
): AuditEvent[] {
  const byId = new Map(events.map((event) => [event.id, event]));
  for (const event of incoming) byId.set(event.id, event);
  return [...byId.values()].sort((a, b) => b.id - a.id).slice(0, MAX_EVENTS);
}

export interface AuditSubscription {
  /** Registers the live-event listener; resolves with its unsubscribe. */
  listenAudit: (onEvent: (event: AuditEvent) => void) => Promise<() => void>;
  /** Reads the persisted snapshot (newest first). */
  fetchSnapshot: () => Promise<AuditEvent[]>;
  /** Merges rows into the visible state. */
  apply: (incoming: AuditEvent[]) => void;
  reportError: (message: string) => void;
}

/** Builds the manual-refresh path. Fetched rows are merged into the current
 * state by persisted id (never a replace), so a stale refresh cannot drop a
 * newer live event already in state. Errors are reported, not thrown. */
export function createAuditRefresh(
  apply: (update: (events: AuditEvent[]) => AuditEvent[]) => void,
  fetchRecent: () => Promise<AuditEvent[]>,
  reportError: (message: string) => void,
): () => Promise<void> {
  return async () => {
    try {
      const recent = await fetchRecent();
      apply((events) => mergeAuditEvents(events, recent));
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  };
}

/** Subscribes to live audit events BEFORE fetching the initial snapshot, so
 * records persisted during the fetch survive (the merge dedupes overlap).
 * If the listener cannot be registered, the error is reported and the
 * snapshot is still fetched so history remains visible.
 * Returns a synchronous cleanup that unsubscribes exactly the registered
 * listener, even if cleanup runs while `listenAudit` is still pending. */
export function subscribeToAuditEvents(sub: AuditSubscription): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  void (async () => {
    try {
      const stop = await sub.listenAudit((event) => {
        if (!disposed) sub.apply([event]);
      });
      if (disposed) {
        stop();
        return;
      }
      unlisten = stop;
    } catch (err) {
      if (!disposed) sub.reportError(parseErrorMessage(err));
    }
    if (disposed) return;
    try {
      const snapshot = await sub.fetchSnapshot();
      if (!disposed) sub.apply(snapshot);
    } catch (err) {
      if (!disposed) sub.reportError(parseErrorMessage(err));
    }
  })();
  return () => {
    disposed = true;
    const stop = unlisten;
    unlisten = null;
    stop?.();
  };
}

export function useAudit(reportError: (m: string) => void) {
  const [auditEvents, setAuditEvents] = useState<AuditEvent[]>([]);

  // Stable error reporting so the subscription effect mounts exactly once.
  // Re-subscribing on callback identity changes would open gaps in which
  // recorded events could be missed.
  const reportErrorRef = useRef(reportError);
  useEffect(() => {
    reportErrorRef.current = reportError;
  }, [reportError]);

  const refreshAudit = useMemo(
    () =>
      createAuditRefresh(
        setAuditEvents,
        api.recentAudit,
        (message) => reportErrorRef.current(message),
      ),
    [],
  );

  useEffect(() => {
    return subscribeToAuditEvents({
      listenAudit: (onEvent) =>
        listen<AuditEvent>("audit-recorded", (event) => onEvent(event.payload)),
      fetchSnapshot: api.recentAudit,
      apply: (incoming) =>
        setAuditEvents((events) => mergeAuditEvents(events, incoming)),
      reportError: (message) => reportErrorRef.current(message),
    });
  }, []);

  return { auditEvents, refreshAudit };
}
