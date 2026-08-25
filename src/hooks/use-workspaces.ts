import { useCallback, useState } from "react";
import { api, parseErrorMessage } from "../api";
import type { Workspace } from "../api";

export function useWorkspaces(reportError: (m: string) => void) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);

  const reloadWorkspaces = useCallback(async () => {
    try {
      setWorkspaces(await api.listWorkspaces());
    } catch (err) {
      reportError(parseErrorMessage(err));
    }
  }, [reportError]);

  return { workspaces, reloadWorkspaces };
}
