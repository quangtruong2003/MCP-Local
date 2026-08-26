import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, parseErrorMessage } from "../api";
import type { ConnectionStatus } from "../api";
import { subscribeToStatus } from "./subscribe-to-status";

export function useConnection(
  reportError: (m: string) => void,
  onConnectionEvent?: () => void,
) {
  const [connection, setConnection] = useState<ConnectionStatus | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void (async () => {
      try {
        unlisten = await subscribeToStatus({
          readStatus: api.connectionStatus,
          listenStatus: (onStatus) =>
            listen<ConnectionStatus>("connection-status", (e) => onStatus(e.payload)),
          onStatus: (status) => {
            if (!disposed) setConnection(status);
          },
          onEvent: onConnectionEvent,
        });
      } catch (err) {
        reportError(parseErrorMessage(err));
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
    // Mount-once subscription; handlers are stable enough for this shell.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const refreshConnection = useCallback(async () => {
    try {
      setConnection(await api.connectionStatus());
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }, [reportError]);

  return { connection, refreshConnection, setConnection };
}
