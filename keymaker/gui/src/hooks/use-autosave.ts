import { useCallback, useEffect, useRef, useState } from "react";

/** Where a row is in the write cycle. Absent means nothing is outstanding. */
export type SaveState = "pending" | "saving" | "saved" | "error";

/** How long typing must stop before the write goes out. */
export const AUTOSAVE_DELAY = 700;

/** How long "saved" stays on screen before the row goes quiet again. */
const SETTLE_DELAY = 1600;

interface Autosave<T> {
  state: Record<string, SaveState>;
  /** Queue `payload` for `id`, restarting its debounce. */
  schedule: (id: string, payload: T) => void;
  /** Write `id` now if it has something queued. */
  flush: (id: string) => void;
  /** Write everything queued. Call before the edited rows go away. */
  flushAll: () => void;
  /** Drop what is queued for `id` without writing it. */
  cancel: (id: string) => void;
  /** Whether `id` has a write in the air or waiting behind one. */
  isBusy: (id: string) => boolean;
}

/**
 * Debounced per-row writes.
 *
 * Each row debounces on its own, so editing one never delays another, and the
 * *payload* carries everything the write needs. That is what makes a queued
 * edit safe when the surroundings change underneath it: nothing about the
 * destination is read at write time, so a save typed against one reference
 * still lands there even if the list has since been refreshed.
 */
export function useAutosave<T>(
  commit: (id: string, payload: T) => Promise<void>,
  delay: number = AUTOSAVE_DELAY,
): Autosave<T> {
  const [state, setState] = useState<Record<string, SaveState>>({});
  const queued = useRef(new Map<string, T>());
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const settles = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  /**
   * Rows with a write in the air.
   *
   * Without this, two writes for the same row can overlap — and since each one
   * is a separate process talking to the OS keychain, they can finish out of
   * order and leave the *older* value stored. One write at a time per row, with
   * the next starting only once the last has landed.
   */
  const inflight = useRef(new Set<string>());

  // Read through a ref so a caller need not memoise `commit`, and so a write
  // already in flight is never stranded by a re-render.
  const commitRef = useRef(commit);
  useEffect(() => {
    commitRef.current = commit;
  }, [commit]);

  const mark = useCallback((id: string, next: SaveState | null) => {
    setState((s) => {
      if (next === null) {
        if (!(id in s)) return s;
        const { [id]: _, ...rest } = s;
        return rest;
      }
      return { ...s, [id]: next };
    });
  }, []);

  const clearTimer = (map: typeof timers, id: string) => {
    const timer = map.current.get(id);
    if (timer !== undefined) {
      clearTimeout(timer);
      map.current.delete(id);
    }
  };

  const run = useCallback(
    (id: string) => {
      clearTimer(timers, id);
      if (!queued.current.has(id)) return;
      // A write is already going; whatever is queued will be picked up by that
      // write's completion, so it cannot be lost and cannot overtake.
      if (inflight.current.has(id)) return;

      const payload = queued.current.get(id) as T;
      queued.current.delete(id);
      inflight.current.add(id);
      mark(id, "saving");

      const settle = () => {
        clearTimer(settles, id);
        settles.current.set(
          id,
          setTimeout(() => {
            settles.current.delete(id);
            // A row edited again while settling has its own state; only retire
            // the marker if it is still the one we set.
            setState((s) => (s[id] === "saved" ? omit(s, id) : s));
          }, SETTLE_DELAY),
        );
      };

      commitRef.current(id, payload).then(
        () => {
          inflight.current.delete(id);
          if (queued.current.has(id)) {
            // Typing continued while this was in the air. The newer value is
            // the one that should end up stored, so write it now rather than
            // reporting "saved" for a value already superseded.
            mark(id, "pending");
            run(id);
            return;
          }
          mark(id, "saved");
          settle();
        },
        () => {
          inflight.current.delete(id);
          // A failed write must not strand a newer edit behind it.
          if (queued.current.has(id)) {
            mark(id, "pending");
            run(id);
            return;
          }
          // The error text belongs to the caller; the row only reports that
          // this value is not what is stored.
          mark(id, "error");
        },
      );
    },
    [mark],
  );

  const schedule = useCallback(
    (id: string, payload: T) => {
      queued.current.set(id, payload);
      clearTimer(settles, id);
      mark(id, "pending");
      clearTimer(timers, id);
      timers.current.set(id, setTimeout(() => run(id), delay));
    },
    [delay, mark, run],
  );

  const cancel = useCallback(
    (id: string) => {
      queued.current.delete(id);
      clearTimer(timers, id);
      clearTimer(settles, id);
      mark(id, null);
    },
    [mark],
  );

  const flush = useCallback((id: string) => run(id), [run]);

  const flushAll = useCallback(() => {
    for (const id of [...queued.current.keys()]) run(id);
  }, [run]);

  // Timers outlive the render that made them; never leave one behind.
  useEffect(() => {
    const pending = timers.current;
    const settling = settles.current;
    return () => {
      pending.forEach(clearTimeout);
      settling.forEach(clearTimeout);
    };
  }, []);

  /** True while a row has a write in the air or queued behind one. */
  const isBusy = useCallback(
    (id: string) => inflight.current.has(id) || queued.current.has(id),
    [],
  );

  return { state, schedule, flush, flushAll, cancel, isBusy };
}

function omit<T>(record: Record<string, T>, key: string): Record<string, T> {
  const { [key]: _, ...rest } = record;
  return rest;
}
