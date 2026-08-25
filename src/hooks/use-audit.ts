import { useCallback, useState } from "react";
import { api, parseErrorMessage } from "../api";
import type { AuditEvent } from "../api";

export function useAudit(reportError: (m: string) => void) {
  const [auditEvents, setAuditEvents] = useState<AuditEvent[]>([]);

  const refreshAudit = useCallback(async () => {
    try {
      setAuditEvents(await api.recentAudit());
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }, [reportError]);

  return { auditEvents, refreshAudit };
}
