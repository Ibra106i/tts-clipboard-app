import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SPEEDS, usePlayback } from "./usePlayback";
import { harness } from "../test/mocks/harness";
import type { PlaybackSnapshot } from "../lib/types";

function snapshot(overrides: Partial<PlaybackSnapshot> = {}): PlaybackSnapshot {
  return {
    status: "idle",
    source: null,
    title: "",
    text_preview: "",
    spoken_chars: 0,
    total_chars: 0,
    start_char: 0,
    rate: 1,
    finished: false,
    ...overrides,
  };
}

describe("usePlayback", () => {
  it("seeds itself from the backend so a window opened mid-playback is correct", async () => {
    harness.on("playback_get_state", () =>
      snapshot({
        status: "playing",
        source: {
          kind: "book",
          book_id: "b1",
          chapter_index: 1,
          total_chapters: 4,
        },
        title: "Chapter 2",
        text_preview: "hello",
        spoken_chars: 5,
        total_chars: 10,
        rate: 1,
      }),
    );

    const { result } = renderHook(() => usePlayback());

    await waitFor(() => expect(result.current.status).toBe("playing"));
    expect(result.current.progressPercent).toBe(50);
    expect(result.current.isPlaying).toBe(true);
    expect(result.current.isActive).toBe(true);
  });

  it("tracks pushed state events without polling", async () => {
    harness.on("playback_get_state", () => snapshot());
    const { result } = renderHook(() => usePlayback());
    await waitFor(() => expect(harness.listenerCount("playback-state")).toBe(1));

    await act(async () => {
      harness.emit(
        "playback-state",
        snapshot({
          status: "paused",
          source: { kind: "clipboard" },
          spoken_chars: 25,
          total_chars: 100,
        }),
      );
    });

    expect(result.current.status).toBe("paused");
    expect(result.current.progressPercent).toBe(25);
    expect(result.current.isPaused).toBe(true);
    expect(harness.callsFor("playback_get_state")).toHaveLength(1);
  });

  it("reports playback failures through onError instead of storing them", async () => {
    harness.on("playback_get_state", () => snapshot());
    const onError = vi.fn();
    renderHook(() => usePlayback({ onError }));
    await waitFor(() => expect(harness.listenerCount("tts-busy")).toBe(1));

    await act(async () => {
      harness.emit("tts-busy", undefined);
    });

    expect(onError).toHaveBeenCalledWith("Finish the current playback first.");
  });

  it("reports a failed command as an error rather than a silent no-op", async () => {
    harness.on("playback_get_state", () => snapshot());
    harness.on("pause_resume_tts", () => {
      throw new Error("engine gone");
    });

    const onError = vi.fn();
    const { result } = renderHook(() => usePlayback({ onError }));

    await act(async () => {
      await result.current.togglePause();
    });

    expect(onError).toHaveBeenCalledWith("engine gone");
  });

  it("cycles the playback rate and tells the backend", async () => {
    harness.on("playback_get_state", () => snapshot());
    harness.on("set_tts_rate", () => undefined);

    const { result } = renderHook(() => usePlayback());
    expect(result.current.rate).toBe(SPEEDS[0]);

    await act(async () => {
      await result.current.cycleRate();
    });

    expect(harness.callsFor("set_tts_rate")[0]?.args).toEqual({
      rate: SPEEDS[1],
    });
  });

  it("registers exactly one listener per event and removes them on unmount", async () => {
    harness.on("playback_get_state", () => snapshot());
    const { unmount } = renderHook(() => usePlayback());

    await waitFor(() => expect(harness.listenerCount("playback-state")).toBe(1));
    expect(harness.listenerCount("chapter-finished")).toBe(1);
    expect(harness.listenerCount("tts-busy")).toBe(1);

    unmount();
    expect(harness.totalListenerCount()).toBe(0);
  });
});
