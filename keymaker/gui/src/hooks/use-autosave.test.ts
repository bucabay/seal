import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useAutosave, AUTOSAVE_DELAY } from "./use-autosave";

/**
 * These pin down the behaviour a real edit session depends on.
 *
 * The bug that prompted them: two writes for the same row could be in the air
 * at once, and because each one is a separate process talking to the OS
 * keychain, they could finish out of order and leave the *older* value stored.
 */

/** A commit that resolves when the test says so, and records what it was given. */
function controllable() {
  const calls: string[] = [];
  const resolvers: Array<() => void> = [];
  const commit = vi.fn((_id: string, payload: string) => {
    calls.push(payload);
    return new Promise<void>((resolve) => resolvers.push(resolve));
  });
  return {
    commit,
    calls,
    /** Let the nth outstanding write finish. */
    finish: async (n = 0) => {
      const resolve = resolvers.splice(n, 1)[0];
      expect(resolve, "expected a write to be outstanding").toBeTruthy();
      await act(async () => {
        resolve();
        await Promise.resolve();
      });
    },
    outstanding: () => resolvers.length,
  };
}

beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
afterEach(() => vi.useRealTimers());

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    await Promise.resolve();
  });
}

describe("useAutosave", () => {
  it("writes once, after typing stops, with the last thing typed", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    // Typing: each keystroke restarts the clock.
    for (const value of ["s", "se", "sec", "secr", "secre", "secret"]) {
      act(() => result.current.schedule("k", value));
      await advance(100);
    }
    expect(c.commit, "nothing should be written mid-typing").not.toHaveBeenCalled();

    await advance(AUTOSAVE_DELAY);
    expect(c.commit).toHaveBeenCalledTimes(1);
    expect(c.calls).toEqual(["secret"]);
  });

  it("never has two writes for the same row in the air at once", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    act(() => result.current.schedule("k", "first"));
    await advance(AUTOSAVE_DELAY);
    expect(c.outstanding()).toBe(1);

    // More typing while the first write is still going.
    act(() => result.current.schedule("k", "second"));
    await advance(AUTOSAVE_DELAY);
    expect(c.outstanding(), "the second write must wait its turn").toBe(1);
    expect(c.calls).toEqual(["first"]);

    // Once the first lands, the second goes out.
    await c.finish();
    expect(c.calls).toEqual(["first", "second"]);
  });

  it("ends with the newest value stored even when writes finish out of order", async () => {
    // The original bug, stated as the user saw it. Each write is a separate
    // process talking to the keychain, so two in the air can land in either
    // order — and the one that lands last is the one that is stored. This
    // models that by resolving the second write first.
    const resolvers: Array<() => void> = [];
    let stored: string | null = null;
    const commit = vi.fn((_id: string, payload: string) =>
      new Promise<void>((resolve) => {
        resolvers.push(() => {
          // Whatever finishes last is what the keychain ends up holding.
          stored = payload;
          resolve();
        });
      }),
    );
    const { result } = renderHook(() => useAutosave<string>(commit));

    act(() => result.current.schedule("k", "old"));
    await advance(AUTOSAVE_DELAY);
    act(() => result.current.schedule("k", "new"));
    await advance(AUTOSAVE_DELAY);

    // Let them finish in the worst order: newest first, oldest second.
    await act(async () => {
      for (const resolve of resolvers.splice(0).reverse()) resolve();
      await Promise.resolve();
    });
    // Drain anything the completions queued.
    await act(async () => {
      for (const resolve of resolvers.splice(0)) resolve();
      await Promise.resolve();
    });

    expect(
      stored,
      "an older value overwriting a newer one is the bug this guards",
    ).toBe("new");
  });

  it("coalesces everything typed during a write into one follow-up", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    act(() => result.current.schedule("k", "a"));
    await advance(AUTOSAVE_DELAY);

    for (const value of ["ab", "abc", "abcd"]) {
      act(() => result.current.schedule("k", value));
      await advance(AUTOSAVE_DELAY);
    }
    await c.finish();

    expect(c.calls, "three keystrokes should not be three writes").toEqual(["a", "abcd"]);
  });

  it("keeps rows independent", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    act(() => {
      result.current.schedule("one", "1");
      result.current.schedule("two", "2");
    });
    await advance(AUTOSAVE_DELAY);

    expect(c.calls.sort()).toEqual(["1", "2"]);
    expect(c.outstanding(), "one row must not block another").toBe(2);
  });

  it("flush writes immediately without waiting out the debounce", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    act(() => result.current.schedule("k", "typed"));
    act(() => result.current.flush("k"));
    expect(c.calls).toEqual(["typed"]);

    // And the debounce timer it cancelled does not fire a second write.
    await advance(AUTOSAVE_DELAY * 2);
    expect(c.commit).toHaveBeenCalledTimes(1);
  });

  it("flush during a write does not start a second one", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    act(() => result.current.schedule("k", "first"));
    await advance(AUTOSAVE_DELAY);
    act(() => result.current.schedule("k", "second"));
    act(() => result.current.flush("k"));

    expect(c.outstanding()).toBe(1);
    expect(c.calls).toEqual(["first"]);
  });

  it("flushAll writes every queued row", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    act(() => {
      result.current.schedule("one", "1");
      result.current.schedule("two", "2");
    });
    act(() => result.current.flushAll());
    expect(c.calls.sort()).toEqual(["1", "2"]);
  });

  it("cancel drops a queued write", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    act(() => result.current.schedule("k", "discard me"));
    act(() => result.current.cancel("k"));
    await advance(AUTOSAVE_DELAY * 2);

    expect(c.commit).not.toHaveBeenCalled();
    expect(result.current.state.k).toBeUndefined();
  });

  it("reports pending, then saving, then saved", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    act(() => result.current.schedule("k", "v"));
    expect(result.current.state.k).toBe("pending");

    await advance(AUTOSAVE_DELAY);
    expect(result.current.state.k).toBe("saving");

    await c.finish();
    expect(result.current.state.k).toBe("saved");
  });

  it("does not claim saved while a newer edit is still to go out", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    act(() => result.current.schedule("k", "first"));
    await advance(AUTOSAVE_DELAY);
    act(() => result.current.schedule("k", "second"));
    await advance(AUTOSAVE_DELAY);
    await c.finish(); // "first" lands, "second" is still queued

    expect(
      result.current.state.k,
      "saying `saved` here would mean a value that is not what is stored",
    ).not.toBe("saved");
  });

  it("a failed write is reported and does not strand a newer edit", async () => {
    const failThenSucceed = vi
      .fn<(id: string, payload: string) => Promise<void>>()
      .mockRejectedValueOnce(new Error("keychain said no"))
      .mockResolvedValue(undefined);
    const { result } = renderHook(() => useAutosave<string>(failThenSucceed));

    act(() => result.current.schedule("k", "first"));
    await advance(AUTOSAVE_DELAY);
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current.state.k).toBe("error");

    // Editing again must get through rather than sitting behind the failure.
    act(() => result.current.schedule("k", "second"));
    await advance(AUTOSAVE_DELAY);
    await act(async () => {
      await Promise.resolve();
    });
    expect(failThenSucceed).toHaveBeenCalledTimes(2);
    expect(failThenSucceed.mock.calls.at(-1)?.[1]).toBe("second");
  });

  it("isBusy says whether a row still has work outstanding", async () => {
    const c = controllable();
    const { result } = renderHook(() => useAutosave<string>(c.commit));

    expect(result.current.isBusy("k")).toBe(false);
    act(() => result.current.schedule("k", "v"));
    expect(result.current.isBusy("k")).toBe(true);

    await advance(AUTOSAVE_DELAY);
    expect(result.current.isBusy("k"), "still in the air").toBe(true);

    await c.finish();
    expect(result.current.isBusy("k")).toBe(false);
  });
});
