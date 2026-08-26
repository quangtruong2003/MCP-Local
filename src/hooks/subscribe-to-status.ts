export async function subscribeToStatus<T>({
  readStatus,
  listenStatus,
  onStatus,
  onEvent = () => {},
}: {
  readStatus: () => Promise<T>;
  listenStatus: (onStatus: (status: T) => void) => Promise<() => void>;
  onStatus: (status: T) => void;
  onEvent?: () => void;
}): Promise<() => void> {
  let eventReceived = false;
  const unlisten = await listenStatus((status) => {
    eventReceived = true;
    onStatus(status);
    onEvent();
  });
  let status: T;
  try {
    status = await readStatus();
  } catch (err) {
    // The listener is already registered; drop it before surfacing the
    // failure so the caller is not left with a dangling subscription.
    unlisten();
    throw err;
  }
  // If an event already arrived while the read was pending, that event (and
  // anything after it) is newer than this snapshot. Applying the snapshot
  // here could overwrite it, so rely on the event stream instead.
  if (!eventReceived) {
    onStatus(status);
  }
  return unlisten;
}
