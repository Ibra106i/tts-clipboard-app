// The clipboard overlay window.
//
// It is a *view*: it never starts speech. The backend's global-hotkey handler
// owns the clipboard flow, and the actor's `playback-state` events drive
// everything rendered here.

import { useCallback, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { usePlayback } from "../hooks/usePlayback";
import { describeError } from "../lib/errors";
import { ElapsedClock } from "./ElapsedClock";
import { Toast } from "./Toast";
import { useToast } from "../hooks/useToast";

export function Overlay() {
  const toast = useToast();
  const showToast = toast.show;

  const playback = usePlayback({ onError: showToast });

  const closeOverlay = useCallback(async () => {
    try {
      await getCurrentWindow().hide();
    } catch (err) {
      showToast(describeError(err));
      console.error("failed to hide the overlay:", err);
    }
  }, [showToast]);

  // The backend already truncates the preview in characters; slicing here by
  // `string.length` would cut UTF-16 code units and split astral characters.
  const displayText = playback.snapshot?.text_preview ?? "";

  // Escape hides the overlay, matching the close button.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") void closeOverlay();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [closeOverlay]);

  return (
    <div className="overlay-root" role="dialog" aria-label="Clipboard speech">
      <div className="drag-bar" data-tauri-drag-region />
      <Toast message={toast.message} />
      <div className="text-area">
        {displayText || (
          <span className="placeholder">Waiting for clipboard text...</span>
        )}
      </div>
      <div className="controls">
        <button
          type="button"
          className="btn-speed"
          onClick={playback.cycleRate}
          aria-label={`Playback speed ${playback.rate} times`}
          title="Change speed"
        >
          {playback.rate}x
        </button>
        <button
          type="button"
          className="btn-play"
          onClick={playback.togglePause}
          disabled={!displayText}
          aria-label={playback.isPaused ? "Resume" : "Pause"}
          title={playback.isPaused ? "Resume" : "Pause"}
        >
          {playback.isPaused ? "▶" : "⏸"}
        </button>
        <button
          type="button"
          className="btn-close"
          onClick={closeOverlay}
          aria-label="Close overlay"
          title="Close"
        >
          ×
        </button>
      </div>
      <div
        className="progress-track"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(playback.progressPercent)}
        aria-label="Playback progress"
      >
        <div
          className="progress-fill"
          style={{ width: `${playback.progressPercent}%` }}
        />
      </div>
      <ElapsedClock
        playing={playback.isPlaying}
        resetKey={playback.playbackKey}
        totalMs={playback.estimatedTotalMs}
      />
    </div>
  );
}
