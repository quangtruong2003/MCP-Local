import { useMemo } from "react";
import { toast } from "sonner";

/** Thin typed wrapper over sonner; stable identity so it is safe as a
 * dependency of memoized callbacks. */
export function useToast() {
  return useMemo(
    () => ({
      info: (message: string) => toast.info(message),
      success: (message: string) => toast.success(message),
      error: (message: string) => toast.error(message),
    }),
    [],
  );
}
