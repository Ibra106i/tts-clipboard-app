// The elapsed-time readout, isolated so its tick re-renders only itself.
//
// This clock used to live in `usePlayback`, which meant every window that read
// playback state re-rendered four times a second. In the reader that is four
// full rebuilds of every paragraph in the chapter, every second, for the sake
// of two numbers - the lag you feel when audio is playing. Nothing else in
// either window changes on this cadence, so nothing else should be redrawn.

import { useEffect, useState } from "react";
import { formatTime } from "../lib/format";

const TICK_MS = 250;

export interface ElapsedClockProps {
  /** True while the voice is running. The clock only advances when it is. */
  playing: boolean;
  /**
   * Changes whenever a new read begins, so the clock restarts for the new
   * passage rather than carrying the previous one's time into it. Derived from
   * the job's identity, not from a counter, so it is correct across windows and
   * across a stop that happens to be followed by an identical read.
   */
  resetKey: string;
  /** Estimated length of the passage, in milliseconds. */
  totalMs: number;
}

export function ElapsedClock({
  playing,
  resetKey,
  totalMs,
}: ElapsedClockProps) {
  const [elapsedMs, setElapsedMs] = useState(0);

  // A new passage starts at zero, and so does one that has stopped: the
  // readout describes the passage that is playing, not the last one played.
  useEffect(() => {
    setElapsedMs(0);
  }, [resetKey, playing]);

  useEffect(() => {
    if (!playing) return;
    const startedAt = Date.now();
    // Capture the offset already on the clock so a re-render mid-passage does
    // not restart the count.
    const base = elapsedMs;
    const timer = setInterval(() => {
      setElapsedMs(base + (Date.now() - startedAt));
    }, TICK_MS);
    return () => clearInterval(timer);
    // `elapsedMs` is intentionally absent: reading it here would tear down and
    // rebuild the interval on every tick, which is the work being avoided.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing, resetKey]);

  return (
    <div className="time-display">
      {formatTime(elapsedMs)} / {formatTime(totalMs)}
    </div>
  );
}
