import { useEffect, useRef, useState, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { estimateDurationMs, formatTime, percentOf } from "./lib/format";
import { describeError } from "./lib/errors";
import * as ipc from "./lib/ipc";
import type {
  Book,
  Chapter,
  ChapterFinished,
  PlaybackSnapshot,
} from "./lib/types";
import "./App.css";

// ── Constants ──────────────────────────────────────────────────────

const SPEEDS = [1, 1.25, 1.5, 2] as const;

// ── Overlay App (Clipboard TTS) ────────────────────────────────────

function OverlayApp() {
  const [text, setText] = useState("");
  const [isPaused, setIsPaused] = useState(false);
  const [speedIdx, setSpeedIdx] = useState(0);
  const [progress, setProgress] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [estimatedTotal, setEstimatedTotal] = useState(0);
  const [toast, setToast] = useState("");

  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Elapsed time is measured here, locally, and only while the backend says
  // playback is actually running. It is a clock, not control flow: it never
  // asks the backend anything and can never advance playback.
  const [isPlaying, setIsPlaying] = useState(false);
  const startedAtRef = useRef<number>(0);
  const accumulatedRef = useRef<number>(0);

  const showToast = useCallback((msg: string, duration = 2000) => {
    setToast(msg);
    if (toastTimer.current) clearTimeout(toastTimer.current);
    toastTimer.current = setTimeout(() => setToast(""), duration);
  }, []);

  useEffect(() => {
    if (!isPlaying || isPaused) return;
    startedAtRef.current = Date.now();
    const timer = setInterval(() => {
      setElapsed(
        accumulatedRef.current + (Date.now() - startedAtRef.current)
      );
    }, 250);
    return () => {
      clearInterval(timer);
      accumulatedRef.current += Date.now() - startedAtRef.current;
    };
  }, [isPlaying, isPaused]);

  // The overlay is a *view*. It never starts speech; the backend's hotkey
  // handler is the single owner of the clipboard flow, and the actor's
  // `playback-state` events are the only thing that drives this UI.
  useEffect(() => {
    const unlistens: (() => void)[] = [];
    const setup = async () => {
      unlistens.push(
        await listen<PlaybackSnapshot>("playback-state", (event) => {
          const snapshot = event.payload;
          setText(snapshot.text_preview);
          setIsPaused(snapshot.status === "paused");
          setProgress(percentOf(snapshot.spoken_chars, snapshot.total_chars));
          setEstimatedTotal(
            estimateDurationMs(snapshot.total_chars, snapshot.rate)
          );

          const isThisJob = snapshot.source?.kind === "clipboard";
          setIsPlaying(isThisJob && snapshot.status === "playing");
          if (!isThisJob || snapshot.finished) {
            accumulatedRef.current = 0;
            setElapsed(0);
          }
        })
      );
      unlistens.push(
        await listen("clipboard-empty", () => {
          showToast("Copy some text first!");
        })
      );
      unlistens.push(
        await listen("tts-busy", () => {
          showToast("Finish current playback first");
        })
      );
    };
    void setup();
    return () => {
      unlistens.forEach((fn) => fn());
    };
  }, [showToast]);

  const togglePause = async () => {
    try {
      const paused = await ipc.pauseResume();
      setIsPaused(paused);
    } catch (err) {
      showToast(describeError(err));
      console.error("pause_resume error:", err);
    }
  };

  const cycleSpeed = async () => {
    const next = (speedIdx + 1) % SPEEDS.length;
    setSpeedIdx(next);
    try {
      await ipc.setRate(SPEEDS[next]);
    } catch (err) {
      showToast(describeError(err));
      console.error("set_tts_rate error:", err);
    }
  };

  const closeOverlay = async () => {
    try {
      await getCurrentWindow().hide();
    } catch (err) {
      showToast(describeError(err));
      console.error("hide error:", err);
    }
  };

  // The backend already truncates the preview in characters; slicing here by
  // `string.length` would cut UTF-16 code units and split astral characters.
  const displayText = text;

  return (
    <div className="overlay-root">
      <div className="drag-bar" data-tauri-drag-region />
      {toast && <div className="toast">{toast}</div>}
      <div className="text-area">
        {displayText || (
          <span className="placeholder">Waiting for clipboard text...</span>
        )}
      </div>
      <div className="controls">
        <button className="btn-speed" onClick={cycleSpeed} title="Change speed">
          {SPEEDS[speedIdx]}x
        </button>
        <button
          className="btn-play"
          onClick={togglePause}
          disabled={!text}
          title={isPaused ? "Resume" : "Pause"}
        >
          {isPaused ? "▶" : "⏸"}
        </button>
        <button className="btn-close" onClick={closeOverlay} title="Close">
          ×
        </button>
      </div>
      <div className="progress-track">
        <div className="progress-fill" style={{ width: `${progress}%` }} />
      </div>
      <div className="time-display">
        {formatTime(elapsed)} / {formatTime(estimatedTotal)}
      </div>
    </div>
  );
}

// ── Main App (Library + Reader) ────────────────────────────────────

type View = "library" | "reader";

function MainApp() {
  const [view, setView] = useState<View>("library");
  const [library, setLibrary] = useState<Book[]>([]);
  const [selectedBook, setSelectedBook] = useState<Book | null>(null);
  const [chapters, setChapters] = useState<Chapter[]>([]);
  const [currentChapterIdx, setCurrentChapterIdx] = useState(0);
  const [isPaused, setIsPaused] = useState(false);
  const [speedIdx, setSpeedIdx] = useState(0);
  const [progress, setProgress] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [estimatedTotal, setEstimatedTotal] = useState(0);
  const [toast, setToast] = useState("");
  const [isDragging, setIsDragging] = useState(false);

  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const progressTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const startTimeRef = useRef<number>(0);
  const pausedAtRef = useRef<number>(0);
  const totalCharsRef = useRef<number>(0);
  const autoAdvanceRef = useRef(false);

  const showToast = useCallback((msg: string, duration = 2000) => {
    setToast(msg);
    if (toastTimer.current) clearTimeout(toastTimer.current);
    toastTimer.current = setTimeout(() => setToast(""), duration);
  }, []);

  const refreshLibrary = useCallback(async () => {
    try {
      const books = await ipc.getLibrary();
      setLibrary(books);
    } catch (err) {
      console.error("Failed to load library:", err);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const books = await ipc.getLibrary();
        if (!cancelled) setLibrary(books);
      } catch (err) {
        console.error("Failed to load library:", err);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, []);

  const stopProgressPolling = useCallback(() => {
    if (progressTimer.current) {
      clearInterval(progressTimer.current);
      progressTimer.current = null;
    }
  }, []);

  const startProgressPolling = useCallback(
    (totalChars: number, resume = false) => {
      stopProgressPolling();
      totalCharsRef.current = totalChars;
      if (!resume) {
        startTimeRef.current = Date.now();
      } else {
        const pausedDuration = Date.now() - pausedAtRef.current;
        startTimeRef.current += pausedDuration;
      }
      setEstimatedTotal(estimateDurationMs(totalChars));

      progressTimer.current = setInterval(async () => {
        try {
          // Characters out of characters. The source is a discriminated union,
          // so nothing has to be parsed out of a `"book:<id>:<chapter>"`
          // string to recover identity.
          const snapshot: PlaybackSnapshot = await ipc.getPlaybackState();
          setProgress(percentOf(snapshot.spoken_chars, snapshot.total_chars));
          setElapsed(Date.now() - startTimeRef.current);
        } catch (err) {
          console.error("progress read failed:", err);
        }
      }, 100);
    },
    [stopProgressPolling]
  );

  // Auto-advance is driven by a typed backend event, not by a browser
  // CustomEvent. The old design smuggled a book id and chapter index through a
  // `"book:<id>:<chapter>"` string on `window`, and reconstructed state by
  // parsing it back out; the payload is now a structured object with the same
  // identity the backend used.
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setup = async () => {
      unlisten = await listen<ChapterFinished>(
        "chapter-finished",
        (event) => {
          void (async () => {
            const { book_id: bookId, chapter_index: chapterIdx } = event.payload;
            if (!autoAdvanceRef.current) return;
            if (!selectedBook || selectedBook.id !== bookId) return;

            autoAdvanceRef.current = false;
            stopProgressPolling();

            const nextIdx = chapterIdx + 1;
            if (nextIdx >= chapters.length) {
              showToast("Book finished!");
              return;
            }

            setCurrentChapterIdx(nextIdx);
            setIsPaused(false);
            setProgress(0);
            setElapsed(0);

            try {
              await ipc.saveReadingPosition({
                bookId,
                chapter: nextIdx,
                position: 0,
              });
            } catch (err) {
              showToast(`Could not save your place: ${describeError(err)}`);
            }

            const nextChapter = chapters.find((c) => c.index === nextIdx);
            if (!nextChapter) return;

            try {
              await ipc.speakBookChapter({
                text: nextChapter.content,
                bookId,
                chapterIndex: nextIdx,
                totalChapters: chapters.length,
              });
              autoAdvanceRef.current = true;
              startProgressPolling(nextChapter.content.length);
            } catch (err) {
              showToast(`Could not continue: ${describeError(err)}`);
              console.error("auto-advance speak error:", err);
            }
          })();
        }
      );
    };

    void setup();
    return () => unlisten?.();
  }, [
    selectedBook,
    chapters,
    showToast,
    startProgressPolling,
    stopProgressPolling,
  ]);

  // Listen for tts-busy
  useEffect(() => {
    const unlistens: (() => void)[] = [];
    const setup = async () => {
      unlistens.push(
        await listen("tts-busy", () => {
          showToast("Finish current playback first");
        })
      );
    };
    setup();
    return () => unlistens.forEach((fn) => fn());
  }, [showToast]);

  const handleImport = async (filePath: string) => {
    try {
      await ipc.importBook(filePath);
      await refreshLibrary();
      showToast("Book imported!");
    } catch (err) {
      showToast(`Import failed: ${describeError(err)}`);
      console.error("Import error:", err);
    }
  };

  const handleOpenFileDialog = async () => {
    try {
      const path = await ipc.openFileDialog();
      if (path) await handleImport(path);
    } catch (err) {
      console.error("File dialog error:", err);
    }
  };

  const handleDeleteBook = async (bookId: string) => {
    try {
      await ipc.deleteBook(bookId);
      await refreshLibrary();
      showToast("Book deleted");
    } catch (err) {
      showToast(`Delete failed: ${describeError(err)}`);
    }
  };

  const handleOpenBook = async (book: Book) => {
    try {
      const chs = await ipc.getBookChapters(book.id);
      setSelectedBook(book);
      setChapters(chs);
      setCurrentChapterIdx(book.current_chapter);
      setIsPaused(false);
      setProgress(0);
      setElapsed(0);
      setView("reader");
    } catch (err) {
      showToast(`Failed to load book: ${describeError(err)}`);
    }
  };

  const handleBackToLibrary = async () => {
    stopProgressPolling();
    autoAdvanceRef.current = false;
    try {
      await ipc.stop();
    } catch (err) {
      console.warn("stop_tts failed:", err);
    }
    if (selectedBook) {
      try {
        await ipc.saveReadingPosition({
          bookId: selectedBook.id,
          chapter: currentChapterIdx,
          position: 0,
        });
      } catch (err) {
        console.warn("save reading position failed:", err);
      }
    }
    setView("library");
    setSelectedBook(null);
    setChapters([]);
    await refreshLibrary();
  };

  const handleSpeakChapter = async () => {
    if (!selectedBook) return;
    const chapter = chapters.find((c) => c.index === currentChapterIdx);
    if (!chapter) return;

    // Stop any current playback first
    try {
      await ipc.stop();
    } catch (err) {
      console.warn("stop_tts failed:", err);
    }

    setIsPaused(false);
    setProgress(0);
    setElapsed(0);

    try {
      await ipc.speakBookChapter({
        text: chapter.content,
        bookId: selectedBook.id,
        chapterIndex: currentChapterIdx,
        totalChapters: chapters.length,
      });
      autoAdvanceRef.current = true;
      startProgressPolling(chapter.content.length);
    } catch (err) {
      showToast("TTS failed to start");
      console.error("speak error:", err);
    }
  };

  const handlePauseResume = async () => {
    try {
      const paused = await ipc.pauseResume();
      setIsPaused(paused);
      if (paused) {
        pausedAtRef.current = Date.now();
        stopProgressPolling();
      } else {
        const chapter = chapters.find((c) => c.index === currentChapterIdx);
        if (chapter) {
          startProgressPolling(chapter.content.length, true);
        }
      }
    } catch (err) {
      console.error("pause/resume error:", err);
    }
  };

  const handleSpeedChange = async () => {
    const next = (speedIdx + 1) % SPEEDS.length;
    setSpeedIdx(next);
    try {
      await ipc.setRate(SPEEDS[next]);
    } catch (err) {
      console.error("set_tts_rate error:", err);
    }
  };

  const handleChapterChange = async (newIdx: number) => {
    stopProgressPolling();
    autoAdvanceRef.current = false;
    try {
      await ipc.stop();
    } catch (err) {
      console.warn("stop_tts failed:", err);
    }
    setIsPaused(false);
    setProgress(0);
    setElapsed(0);
    setCurrentChapterIdx(newIdx);
    if (selectedBook) {
      try {
        await ipc.saveReadingPosition({
          bookId: selectedBook.id,
          chapter: newIdx,
          position: 0,
        });
      } catch (err) {
        console.warn("save reading position failed:", err);
      }
    }
  };

  // Drag and drop
  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(true);
  };

  const handleDragLeave = () => {
    setIsDragging(false);
  };

  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
    const files = Array.from(e.dataTransfer.files);
    for (const file of files) {
      const ext = file.name.split(".").pop()?.toLowerCase();
      if (ext === "pdf" || ext === "epub") {
        // @ts-expect-error Tauri file path
        const path = file.path || file.name;
        await handleImport(path);
      } else {
        showToast("Only PDF and EPUB files are supported");
      }
    }
  };

  // ── Library View ───────────────────────────────────────────────

  if (view === "library") {
    return (
      <div
        className="app-root"
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        {toast && <div className="toast">{toast}</div>}

        <header className="app-header">
          <h1 className="app-title">📚 TTS Library</h1>
          <button className="btn-import" onClick={handleOpenFileDialog}>
            + Import Book
          </button>
        </header>

        {isDragging && (
          <div className="drop-zone">
            <div className="drop-zone-content">
              <div className="drop-icon">📥</div>
              <div>Drop PDF or EPUB files here</div>
            </div>
          </div>
        )}

        {library.length === 0 ? (
          <div className="empty-state">
            <div className="empty-icon">📖</div>
            <p>No books imported yet</p>
            <p className="empty-hint">
              Click "Import Book" or drag & drop files here
            </p>
          </div>
        ) : (
          <div className="book-grid">
            {library.map((book) => (
              <div
                key={book.id}
                className="book-card"
                onClick={() => handleOpenBook(book)}
              >
                <div className="book-card-header">
                  <span className="book-title">{book.title}</span>
                  <span className={`book-badge badge-${book.file_type}`}>
                    {book.file_type.toUpperCase()}
                  </span>
                </div>
                <div className="book-meta">
                  {book.total_chapters} chapters
                </div>
                <div className="book-progress">
                  <div className="book-progress-track">
                    <div
                      className="book-progress-fill"
                      style={{
                        width: `${
                          book.total_chapters > 0
                            ? (book.current_chapter / book.total_chapters) * 100
                            : 0
                        }%`,
                      }}
                    />
                  </div>
                  <span className="book-progress-text">
                    Ch. {book.current_chapter + 1}/{book.total_chapters}
                  </span>
                </div>
                <button
                  className="btn-delete-book"
                  onClick={(e) => {
                    e.stopPropagation();
                    handleDeleteBook(book.id);
                  }}
                  title="Delete book"
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }

  // ── Reader View ────────────────────────────────────────────────

  const currentChapter = chapters.find((c) => c.index === currentChapterIdx);
  const chapterText = currentChapter?.content || "";

  return (
    <div className="app-root">
      {toast && <div className="toast">{toast}</div>}

      <header className="reader-header">
        <button className="btn-back" onClick={handleBackToLibrary}>
          ← Library
        </button>
        <span className="reader-title">{selectedBook?.title}</span>
        <span className="chapter-counter">
          Ch. {currentChapterIdx + 1}/{chapters.length}
        </span>
        <select
          className="chapter-select"
          value={currentChapterIdx}
          onChange={(e) => handleChapterChange(Number(e.target.value))}
        >
          {chapters.map((ch) => (
            <option key={ch.index} value={ch.index}>
              {ch.title}
            </option>
          ))}
        </select>
      </header>

      <div className="reader-content">
        {chapterText ? (
          <div className="chapter-text">
            {chapterText.split("\n").map((para, i) => (
              <p key={i} className="chapter-paragraph">
                {para}
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
          className="btn-speed"
          onClick={handleSpeedChange}
          title="Change speed"
        >
          {SPEEDS[speedIdx]}x
        </button>
        <button
          className="btn-play"
          onClick={handlePauseResume}
          disabled={!chapterText}
          title={isPaused ? "Resume" : "Pause"}
        >
          {isPaused ? "▶" : "⏸"}
        </button>
        <button
          className="btn-speak"
          onClick={handleSpeakChapter}
          disabled={!chapterText}
        >
          🔊 Speak Chapter
        </button>
      </div>

      <div className="progress-track">
        <div className="progress-fill" style={{ width: `${progress}%` }} />
      </div>
      <div className="time-display">
        {formatTime(elapsed)} / {formatTime(estimatedTotal)}
      </div>
    </div>
  );
}

// ── Root Router ────────────────────────────────────────────────────

function App() {
  // The window label is fixed for the lifetime of the webview, so it is
  // derived once during the first render rather than pushed in through an
  // effect (which would cause a cascading render on every mount).
  const [windowLabel] = useState<string>(() => {
    try {
      return getCurrentWindow().label;
    } catch {
      return "main";
    }
  });

  if (windowLabel === "overlay") {
    return <OverlayApp />;
  }
  return <MainApp />;
}

export default App;
