import { useEffect, useRef } from "react";

export const CATALOG_REFRESH_INTERVAL_MS = 60_000;

/** Keep visible catalog pages current without polling hidden windows or overlapping focus events. */
export function useCatalogRefresh(refresh: () => Promise<unknown>, enabled: boolean): void {
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    if (!enabled) return;
    let stopped = false;
    let running = false;
    const update = () => {
      if (stopped || running || document.visibilityState !== "visible") return;
      running = true;
      void Promise.resolve().then(() => stopped ? undefined : refreshRef.current())
        .catch(() => undefined)
        .finally(() => { running = false; });
    };
    let timer: number;
    const schedule = () => {
      timer = window.setTimeout(() => {
        if (stopped) return;
        update();
        schedule();
      }, CATALOG_REFRESH_INTERVAL_MS);
    };
    schedule();
    window.addEventListener("focus", update);
    document.addEventListener("visibilitychange", update);
    return () => {
      stopped = true;
      window.clearTimeout(timer);
      window.removeEventListener("focus", update);
      document.removeEventListener("visibilitychange", update);
    };
  }, [enabled]);
}
