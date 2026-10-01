// The reader: chapter text, chapter selection and playback controls.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePlayback } from "../hooks/usePlayback";
import { Paragraph } from "./Paragraph";
import { describeError } from "../lib/errors";
import { ElapsedClock } from "./ElapsedClock";
import { Icon } from "./Icon";
import * as ipc from "../lib/ipc";
import {
  paragraphStartsFrom,
  rangeToCharRange,
  type CharRange,
} from "../lib/offsets";
import type { Book, Chapter, TextRange } from "../lib/types";

export interface ReaderViewProps {
  book: Book;
  chapters: Chapter[];
  currentChapterIdx: number;
  /**
   * Character offset within the current chapter to resume from, taken from the
   * book record. Zero means the chapter starts at the top.
   */
  startPosition?: number;
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
  startPosition = 0,
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
  // The character offset the reader should resume from if it is closed. Seeded
  // from the book record, then replaced whenever a read starts somewhere
  // specific, and reset when the chapter changes.
  const resumeAtRef = useRef(startPosition);
  useEffect(() => {
    resumeAtRef.current = startPosition;
  }, [startPosition]);
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
    async (chapter: Chapter, range?: TextRange) => {
      autoAdvanceRef.current = true;
      // Where this read began, so leaving the reader resumes there rather than
      // at the top of the chapter. A whole-chapter read means the top.
      resumeAtRef.current = range?.start ?? 0;
      await playback.startChapter({
        text: chapter.content,
        bookId: book.id,
        chapterIndex: chapter.index,
        totalChapters: chapters.length,
        range,
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
  // The chapter split into paragraphs, and the character offset of each within
  // the chapter. The backend measures in characters, so these are code-point
  // offsets too.
  //
  // Memoised on `chapterText` alone, deliberately: this depends on nothing but
  // the text. It used to be recomputed on every render, and the backend pushes a
  // snapshot four times a second while speaking, so a chapter was split twice
  // and swept end to end through `codePointLength` four times a second -
  // millions of loop iterations and megabytes of transient garbage a second on
  // a long chapter, all of it thrown away before the next render.
  //
  // (An earlier comment here claimed the React Compiler owned memoisation in
  // this component. It does not - it is not installed, so nothing was being
  // memoised. These hooks are doing that work.)
  const paragraphs = useMemo(() => chapterText.split("\n"), [chapterText]);
  const starts = useMemo(() => paragraphStartsFrom(paragraphs), [paragraphs]);

  // Reading from a paragraph reads the current chapter from that offset.
  //
  // `useCallback` here is load-bearing, not decoration. This is the only handler
  // passed to the memoised `Paragraph`, so a fresh identity each render would
  // fail every row's shallow comparison and rebuild the whole chapter on all
  // four backend ticks a second - exactly what the row is there to prevent. The
  // values it needs are therefore read through refs, which are updated as the
  // chapter and its offsets change but do not change the handler's identity.
  const latestRef = useRef({ chapter: currentChapter, paragraphs, starts });
  useEffect(() => {
    latestRef.current = { chapter: currentChapter, paragraphs, starts };
  });
  const handleParagraphClick = useCallback(
    (index: number) => {
      const { chapter, paragraphs: blocks, starts: offsets } = latestRef.current;
      if (!chapter) return;
      // A click that lands on whitespace between blocks has nothing to read.
      if (!blocks[index]?.trim()) return;
      void startChapter(chapter, {
        start: offsets[index] ?? 0,
        align_to_sentence: true,
      });
    },
    [startChapter],
  );

  // Which paragraph playback began in: the last one starting at or before the
  // offset the backend reports. Negative means it began at the top.
  const startedAt = playback.snapshot?.start_char ?? 0;
  let startParagraph = -1;
  for (let index = 0; index < starts.length; index += 1) {
    if ((starts[index] ?? 0) > startedAt) break;
    startParagraph = index;
  }
  if (startedAt <= 0) startParagraph = -1;

  // Which paragraph is being read right now. `spoken_chars` counts from the
  // start of the job, so the absolute position is that plus the job's own
  // start. Following the highlight as it moves is what makes a click's effect
  // visible: without it the only feedback is audio and a progress bar.
  const readingAt = startedAt + (playback.snapshot?.spoken_chars ?? 0);
  let readingParagraph = -1;
  for (let index = 0; index < starts.length; index += 1) {
    if ((starts[index] ?? 0) > readingAt) break;
    readingParagraph = index;
  }
  const isReading =
    playback.isPlaying || playback.isPaused ? readingParagraph : -1;

  // The span the user has selected, ready to be read on request. Null whenever
  // there is nothing to offer, which is the common case: most of the time the
  // caret is collapsed or the selection lives outside the chapter text.
  const [selected, setSelected] = useState<CharRange | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);

  // Plain functions rather than `useCallback`, deliberately: none of these are
  // passed to a memoised child, so wrapping them would only add dependency
  // lists to maintain without changing what re-renders.
  const clearSelection = () => {
    setSelected(null);
    window.getSelection()?.removeAllRanges();
  };

  const handleSelect = () => {
    const container = contentRef.current;
    const selection = window.getSelection();
    if (!container || !selection || selection.rangeCount === 0) {
      setSelected(null);
      return;
    }
    // A click with no drag leaves a collapsed caret. There is nothing to read,
    // and resolving offsets for it would walk the chapter for a result that is
    // thrown away one line later - on every single click in the text.
    if (selection.isCollapsed) {
      setSelected(null);
      return;
    }
    const range = rangeToCharRange(container, selection.getRangeAt(0));
    // A span of no characters, or one with nothing but whitespace in it, is
    // not something to read; offering it would be a dead button.
    if (!range || !chapterText.slice(range.start, range.end).trim()) {
      setSelected(null);
      return;
    }
    setSelected(range);
  };

  const handleReadSelection = () => {
    if (!currentChapter || !selected) return;
    void startChapter(currentChapter, {
      start: selected.start,
      end: selected.end,
      // The user chose these characters; reading them from a sentence boundary
      // instead would silently drop the first word or two of their selection.
      align_to_sentence: false,
    });
    clearSelection();
  };

  // Escape dismisses the offer without reading, so a selection can be abandoned.
  useEffect(() => {
    if (!selected) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setSelected(null);
        window.getSelection()?.removeAllRanges();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selected]);

  const handleSpeak = () => {
    if (!currentChapter) return;
    void startChapter(currentChapter);
  };

  // Clicking a paragraph reads from the sentence at or after its start.
  const handleBack = useCallback(async () => {
    autoAdvanceRef.current = false;
    await playback.stop();
    void ipc
      .saveReadingPosition({
        bookId: book.id,
        chapter: currentChapterIdx,
        // Where the read actually began, not a hard-coded zero: closing the
        // reader after clicking into a chapter has to come back to that spot.
        position: resumeAtRef.current,
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
      // Record where the reader was before it moved, so the chapter it left
      // still opens at the right paragraph next time.
      void ipc
        .saveReadingPosition({
          bookId: book.id,
          chapter: currentChapterIdx,
          position: resumeAtRef.current,
        })
        .catch((err) => {
          showToast(`Could not save your place: ${describeError(err)}`);
        });
      onChapterChange(index);
      onReadingPositionChange(index);
    },
    [
      book.id,
      currentChapterIdx,
      onChapterChange,
      onReadingPositionChange,
      playback,
      showToast,
    ],
  );

  return (
    <div className="app-root">
      <header className="reader-header">
        <button type="button" className="btn-back" onClick={handleBack}>
          <Icon name="back" size={14} />
          Library
        </button>
        <span className="reader-title">{book.title}</span>
        <span className="chapter-counter">
          Ch. {currentChapterIdx + 1}/{chapters.length}
          {startParagraph >= 0 ? (
            <span className="chapter-counter-start"> · from ¶{startParagraph + 1}</span>
          ) : null}
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

      <div
        className="reader-content"
        ref={contentRef}
        onMouseUp={handleSelect}
      >
        {chapterText ? (
          <div className="chapter-text">
            {paragraphs.map((paragraph, i) => (
              <Paragraph
                key={i}
                index={i}
                text={paragraph}
                startChar={starts[i] ?? 0}
                isReading={isReading === i}
                onReadFrom={handleParagraphClick}
              />
            ))}
          </div>
        ) : (
          <div className="empty-state">
            <p>No content available for this chapter</p>
          </div>
        )}
      </div>

      {selected ? (
        <div className="selection-pill" role="group" aria-label="Read selection">
          <span className="selection-pill-length">
            {selected.end - selected.start} characters
          </span>
          <button
            type="button"
            className="selection-pill-action"
            onClick={handleReadSelection}
            aria-label="Read this selection"
          >
            <Icon name="speaker" size={14} />
            Read this
          </button>
        </div>
      ) : null}

      <div className="reader-controls">
        {/* Icon-only controls carry their accessible name through `title`; beside a
            text label the icon is decorative and stays hidden. */}
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
          <Icon
            name={playback.isPaused ? "play" : "pause"}
            size={18}
            title={playback.isPaused ? "Resume" : "Pause"}
          />
        </button>
        <button
          type="button"
          className="btn-speak"
          onClick={handleSpeak}
          disabled={!chapterText}
        >
          <Icon name="speaker" />
          Speak Chapter
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
