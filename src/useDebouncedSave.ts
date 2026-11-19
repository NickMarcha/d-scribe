import { useEffect, useRef, type DependencyList } from "react";

/**
 * Calls `save` `delayMs` after `deps` last changed, once `ready` is true. A save still waiting
 * when the component unmounts (e.g. leaving Settings right after a change) runs immediately.
 */
export function useDebouncedSave(save: () => void, deps: DependencyList, ready: boolean, delayMs = 400) {
  const pending = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (!ready) return;
    pending.current = save;
    const timer = setTimeout(() => {
      pending.current = null;
      save();
    }, delayMs);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, ...deps]);

  useEffect(
    () => () => {
      pending.current?.();
      pending.current = null;
    },
    [],
  );
}
