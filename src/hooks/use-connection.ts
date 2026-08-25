import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, parseErrorMessage } from "../api";
import type { ConnectionStatus } from "../api";

export function useConnection(
  reportError: (m: string) => void,
  onConnectionEvent: () => void,
) {
  const [connection, setConnection] = useState<ConnectionStatus | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void (async () => {
      try {
        const status = await api.connectionStatus();
        if (!disposed) setConnection(status);
        unlisten = await listen<ConnectionStatus>("connection-status", (e) => {
          setConnection(e.payload);
          onConnectionEvent();
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
