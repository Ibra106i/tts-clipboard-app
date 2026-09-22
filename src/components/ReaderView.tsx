// The reader: chapter text, chapter selection and playback controls.

import { useCallback, useEffect, useRef } from "react";
import { usePlayback } from "../hooks/usePlayback";
import { describeError } from "../lib/errors";
import { formatTime } from "../lib/format";
import * as ipc from "../lib/ipc";
import type { Book, Chapter } from "../lib/types";

export interface ReaderViewProps {
  book: Book;
  chapters: Chapter[];
  currentChapterIdx: number;
  onChapterChange: (index: number) => void;
  onReadingPositionChange: (index: number) => void;
  onBack: () => void;
  /** Where this view reports successes and failures; the container owns the
   * single toast for the window. */
  onMessage: (message: string) => void;
}

export function ReaderView({
  book,
  chapters,
  currentChapterIdx,
  onChapterChange,
  onReadingPositionChange,
  onBack,
  onMessage: showToast,
}: ReaderViewProps) {

  // Whether playback should keep going into the next chapter. Held in a ref so
  // the chapter-finished callback does not need to be re-created (and the
  // listener re-registered) every time play state changes.
  const autoAdvanceRef = useRef(false);
  const chaptersRef = useRef(chapters);
  const currentRef = useRef(currentChapterIdx);
  // Assigned once `startChapter` exists; the indirection keeps the
  // chapter-finished callback independent of it, so the event listener is not
  // torn down and re-registered on every render.
  const startChapterRef = useRef<(chapter: Chapter) => Promise<void>>(
    async () => {},
  );
  useEffect(() => {
    chaptersRef.current = chapters;
  }, [chapters]);
  useEffect(() => {
    currentRef.current = currentChapterIdx;
  }, [currentChapterIdx]);

  const handleChapterFinished = useCallback(
    (payload: { book_id: string; chapter_index: number }) => {
      if (!autoAdvanceRef.current) return;
      if (payload.book_id !== book.id) return;
      if (payload.chapter_index !== currentRef.current) return;

      const next = payload.chapter_index + 1;
      const list = chaptersRef.current;
      if (next >= list.length) {
        autoAdvanceRef.current = false;
        showToast("Book finished");
        return;
      }

      onChapterChange(next);
      onReadingPositionChange(next);

      const nextChapter = list.find((chapter) => chapter.index === next);
      if (!nextChapter) return;
      void ipc
        .saveReadingPosition({ bookId: book.id, chapter: next, position: 0 })
        .catch((err) => {
          showToast(`Could not save your place: ${describeError(err)}`);
        });
      void startChapterRef.current(nextChapter);
    },
    [
      book.id,
      onChapterChange,
      onReadingPositionChange,
      showToast,
    ],
  );

  const playback = usePlayback({
    onError: showToast,
    onChapterFinished: handleChapterFinished,
  });

  const startChapter = useCallback(
    async (chapter: Chapter) => {
      autoAdvanceRef.current = true;
      await playback.startChapter({
        text: chapter.content,
        bookId: book.id,
        chapterIndex: chapter.index,
        totalChapters: chapters.length,
      });
    },
    [book.id, chapters.length, playback],
  );

  useEffect(() => {
    startChapterRef.current = startChapter;
  }, [startChapter]);

  const currentChapter = chapters.find(
    (chapter) => chapter.index === currentChapterIdx,
  );
  const chapterText = currentChapter?.content ?? "";

  // Plain function: it is only used as a click handler, and memoizing it would
  // depend on a chapter object derived from props on every render.
  const handleSpeak = () => {
    if (!currentChapter) return;
    void startChapter(currentChapter);
  };

  const handleBack = useCallback(async () => {
    autoAdvanceRef.current = false;
    await playback.stop();
    void ipc
      .saveReadingPosition({
        bookId: book.id,
        chapter: currentChapterIdx,
        position: 0,
      })
      .catch((err) => {
        showToast(`Could not save your place: ${describeError(err)}`);
      });
    onBack();
  }, [book.id, currentChapterIdx, onBack, playback, showToast]);

  const handleChapterSelect = useCallback(
    (index: number) => {
      autoAdvanceRef.current = false;
      void playback.stop();
      onChapterChange(index);
      onReadingPositionChange(index);
    },
    [onChapterChange, onReadingPositionChange, playback],
  );

  return (
    <div className="app-root">
      <header className="reader-header">
        <button type="button" className="btn-back" onClick={handleBack}>
          ← Library
        </button>
        <span className="reader-title">{book.title}</span>
        <span className="chapter-counter">
          Ch. {currentChapterIdx + 1}/{chapters.length}
        </span>
        <select
          className="chapter-select"
          value={currentChapterIdx}
          aria-label="Select chapter"
          onChange={(event) => handleChapterSelect(Number(event.target.value))}
        >
          {chapters.map((chapter) => (
            <option key={chapter.index} value={chapter.index}>
              {chapter.title}
            </option>
          ))}
        </select>
      </header>

      <div className="reader-content">
        {chapterText ? (
          <div className="chapter-text">
            {chapterText.split("\n").map((paragraph, i) => (
              <p key={i} className="chapter-paragraph">
                {paragraph}
              </p>
            ))}
          </div>
        ) : (
          <div className="empty-state">
            <p>No content available for this chapter</p>
          </div>
        )}
      </div>

      <div className="reader-controls">
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
          disabled={!chapterText}
          aria-label={playback.isPaused ? "Resume" : "Pause"}
          title={playback.isPaused ? "Resume" : "Pause"}
        >
          {playback.isPaused ? "▶" : "⏸"}
        </button>
        <button
          type="button"
          className="btn-speak"
          onClick={handleSpeak}
          disabled={!chapterText}
        >
          🔊 Speak Chapter
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
      <div className="time-display">
        {formatTime(playback.elapsedMs)} / {formatTime(playback.estimatedTotalMs)}
      </div>
    </div>
  );
}
