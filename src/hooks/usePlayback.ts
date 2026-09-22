// Playback, as one hook, for every window.
//
// Before this, both the overlay and the reader kept their own copy of playback
// state, their own polling loop, their own pause flag and their own error
// handling, and the two drifted. Everything a window needs to render or drive
// playback now lives here:
//
// * the latest `PlaybackSnapshot` pushed by the backend,
// * a local elapsed-time clock that only runs while playback is running,
// * rate selection, pause/resume, stop and start,
// * busy and error reporting.
//
// The hook is a *reader* of backend state. It never polls, and it never starts
// playback as a side effect of rendering.

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as ipc from "../lib/ipc";
import { describeError } from "../lib/errors";
import { subscribeAll } from "../lib/subscribe";
import { estimateDurationMs, percentOf } from "../lib/format";
import type {
  ChapterFinished,
  PlaybackSnapshot,
  PlaybackStatus,
} from "../lib/types";

/** Rate multipliers offered by the UI, in cycle order. */
export const SPEEDS = [1, 1.25, 1.5, 2] as const;
const DEFAULT_RATE_INDEX = 0;

export interface StartChapterArgs {
  text: string;
  bookId: string;
  chapterIndex: number;
  totalChapters: number;
}

export interface UsePlaybackOptions {
  /**
   * Called when a book chapter finishes, with the backend's structured
   * payload. The consumer decides whether to continue.
   */
  onChapterFinished?: (payload: ChapterFinished) => void;
  /**
   * Called with a human-readable message whenever a playback operation fails.
   * The hook reports the failure at the point it happens rather than storing
   * it, so the UI never has to react to it in an effect.
   */
  onError?: (message: string) => void;
}

export interface UsePlaybackResult {
  snapshot: PlaybackSnapshot | null;
  status: PlaybackStatus;
  /** 0-100, from characters spoken over characters total. */
  progressPercent: number;
  estimatedTotalMs: number;
  elapsedMs: number;
  rate: number;
  isPlaying: boolean;
  isPaused: boolean;
  /** True while this window should show its progress UI. */
  isActive: boolean;
  cycleRate: () => Promise<void>;
  togglePause: () => Promise<void>;
  stop: () => Promise<void>;
  startChapter: (args: StartChapterArgs) => Promise<void>;
}

export function usePlayback(
  options: UsePlaybackOptions = {},
): UsePlaybackResult {
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [rateIndex, setRateIndex] = useState(DEFAULT_RATE_INDEX);
  const [elapsedMs, setElapsedMs] = useState(0);

  // Callbacks are kept in a ref (updated after render, never during) so that
  // changing them does not re-register listeners, and so the reporting path
  // never depends on render order.
  const callbacksRef = useRef(options);
  useEffect(() => {
    callbacksRef.current = options;
  }, [options]);

  const reportError = useCallback((message: string) => {
    callbacksRef.current.onError?.(message);
  }, []);

  // Elapsed time is measured locally and only while playing. It is a clock,
  // not control flow: it never asks the backend anything.
  const startedAtRef = useRef(0);
  const accumulatedRef = useRef(0);
  const status = snapshot?.status ?? "idle";
  const isPlaying = status === "playing";
  const isPaused = status === "paused";

  useEffect(() => {
    if (!isPlaying) {
      accumulatedRef.current += Date.now() - startedAtRef.current;
      return;
    }
    startedAtRef.current = Date.now();
    const timer = setInterval(() => {
      setElapsedMs(accumulatedRef.current + (Date.now() - startedAtRef.current));
    }, 250);
    return () => clearInterval(timer);
  }, [isPlaying]);

  // One draw of the initial state, so a window opened mid-playback is not
  // blank until the next event.
  useEffect(() => {
    let cancelled = false;
    ipc
      .getPlaybackState()
      .then((initial) => {
        if (!cancelled) setSnapshot(initial);
      })
      .catch((err) => {
        if (!cancelled) reportError(describeError(err));
      });
    return () => {
      cancelled = true;
    };
  }, [reportError]);

  // Backend -> UI events. Registration is asynchronous, so it goes through
  // `subscribeAll`, which guarantees that a listener registering after unmount
  // is removed immediately and that StrictMode's double-mount cannot leave two
  // live listeners for the same event.
  useEffect(() => {
    const subscription = subscribeAll([
      () =>
        listen<PlaybackSnapshot>("playback-state", (event) => {
          setSnapshot(event.payload);
          if (event.payload.source === null || event.payload.finished) {
            accumulatedRef.current = 0;
            setElapsedMs(0);
          }
        }),
      () =>
        listen<ChapterFinished>("chapter-finished", (event) => {
          callbacksRef.current.onChapterFinished?.(event.payload);
        }),
      () =>
        listen("tts-busy", () => {
          reportError("Finish the current playback first.");
        }),
      () =>
        listen("clipboard-empty", () => {
          reportError("Copy some text first.");
        }),
    ]);

    return () => subscription.dispose();
  }, [reportError]);

  const cycleRate = useCallback(async () => {
    setRateIndex((current) => (current + 1) % SPEEDS.length);
    const next = (rateIndex + 1) % SPEEDS.length;
    try {
      await ipc.setRate(SPEEDS[next]);
    } catch (err) {
      reportError(describeError(err));
    }
  }, [rateIndex, reportError]);

  const togglePause = useCallback(async () => {
    try {
      await ipc.pauseResume();
    } catch (err) {
      reportError(describeError(err));
    }
  }, [reportError]);

  const stop = useCallback(async () => {
    try {
      await ipc.stop();
    } catch (err) {
      reportError(describeError(err));
    }
  }, [reportError]);

  const startChapter = useCallback(
    async (args: StartChapterArgs) => {
      // Replace whatever is playing rather than failing with "busy".
      try {
        await ipc.stop();
      } catch (err) {
        // Stopping an idle player is the state we wanted anyway, so this is
        // not an error the user needs to see; record it in the log only.
        console.warn("stop before start failed:", err);
      }
      try {
        await ipc.speakBookChapter(args);
      } catch (err) {
        reportError(describeError(err));
      }
    },
    [reportError],
  );

  const rate = SPEEDS[rateIndex];
  const progressPercent = snapshot
    ? percentOf(snapshot.spoken_chars, snapshot.total_chars)
    : 0;
  const estimatedTotalMs = snapshot
    ? estimateDurationMs(snapshot.total_chars, snapshot.rate)
    : 0;

  return {
    snapshot,
    status,
    progressPercent,
    estimatedTotalMs,
    elapsedMs,
    rate,
    isPlaying,
    isPaused,
    isActive: snapshot?.source !== null && snapshot?.source !== undefined,
    cycleRate,
    togglePause,
    stop,
    startChapter,
  };
}
