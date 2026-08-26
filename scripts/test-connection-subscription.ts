import assert from "node:assert/strict";
import { subscribeToStatus } from "../src/hooks/subscribe-to-status.ts";

type ConnectionStatus =
  | { state: "stopped" }
  | { state: "online"; endpoint: string };

const stopped: ConnectionStatus = { state: "stopped" };
const online: ConnectionStatus = { state: "online", endpoint: "https://example/mcp" };

async function eventDuringInitialReadIsNotLost() {
  let listener: ((status: ConnectionStatus) => void) | undefined;
  const seen: ConnectionStatus[] = [];
  let reads = 0;

  const cleanup = await subscribeToStatus({
    readStatus: async () => {
      reads++;
      // Emit the newer state while the initial read is pending, then return
      // an older snapshot. The snapshot must not be applied at all once an
      // event has arrived, and no reconciliation re-read may happen.
      listener?.(online);
      return stopped;
    },
    listenStatus: async (onStatus) => {
      listener = onStatus;
      return () => {};
    },
    onStatus: (status) => seen.push(status),
  });

  assert.deepEqual(seen, [online]);
  assert.equal(reads, 1);
  cleanup();
}

async function listenerIsAttachedBeforeReadingStatus() {
  const order: string[] = [];
  const seen: ConnectionStatus[] = [];

  const cleanup = await subscribeToStatus({
    readStatus: async () => {
      order.push("read");
      return stopped;
    },
    listenStatus: async () => {
      order.push("listen");
      return () => {};
    },
    onStatus: (status) => seen.push(status),
  });

  assert.deepEqual(order, ["listen", "read"]);
  assert.deepEqual(seen, [stopped]);
  cleanup();
}

async function readFailureRemovesRegisteredListener() {
  let unlistened = false;

  await assert.rejects(
    subscribeToStatus({
      readStatus: async () => {
        throw new Error("status read failed");
      },
      listenStatus: async () => () => {
        unlistened = true;
      },
      onStatus: () => {},
    }),
    /status read failed/,
  );

  assert.equal(unlistened, true);
}

await eventDuringInitialReadIsNotLost();
await listenerIsAttachedBeforeReadingStatus();
await readFailureRemovesRegisteredListener();
console.log("connection subscription regression tests passed");
