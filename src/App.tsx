import { useEffect, useRef, useState, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./App.css";

// ── Types ──────────────────────────────────────────────────────────

interface Book {
  id: string;
  title: string;
  file_path: string;
  file_type: string;
  imported_at: string;
  current_chapter: number;
  current_position: number;
  total_chapters: number;
}

interface Chapter {
  index: number;
  title: string;
  content: string;
}

// ── Constants ──────────────────────────────────────────────────────

const SPEEDS = [1, 1.25, 1.5, 2] as const;

function formatTime(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  return `${min}:${sec.toString().padStart(2, "0")}`;
}

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
  const progressTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const startTimeRef = useRef<number>(0);
  const pausedAtRef = useRef<number>(0);
  const totalCharsRef = useRef<number>(0);

  const showToast = useCallback((msg: string, duration = 2000) => {
    setToast(msg);
    if (toastTimer.current) clearTimeout(toastTimer.current);
    toastTimer.current = setTimeout(() => setToast(""), duration);
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
      const baseDuration = (totalChars / 15) * 1000;
      setEstimatedTotal(baseDuration);

      progressTimer.current = setInterval(async () => {
        try {
          const [current, total] = await invoke<[number, number]>(
            "get_speech_position"
          );
          const pct = total > 0 ? Math.min((current / total) * 100, 100) : 0;
          setProgress(pct);
          setElapsed(Date.now() - startTimeRef.current);
        } catch {
          setElapsed(Date.now() - startTimeRef.current);
          const est = baseDuration;
          setProgress(
            Math.min(((Date.now() - startTimeRef.current) / est) * 100, 99)
          );
        }
      }, 100);
    },
    [stopProgressPolling]
  );

  useEffect(() => {
    const unlistens: (() => void)[] = [];
    const setup = async () => {
      unlistens.push(
        await listen<{ text: string; timestamp: number }>(
          "speak-trigger",
          async (event) => {
            const { text: t } = event.payload;
            setText(t);
            setIsPaused(false);
            setProgress(0);
            setElapsed(0);
            try {
              await invoke("speak_text", { text: t });
              startProgressPolling(t.length);
            } catch (err) {
              if (err === "busy") {
                showToast("Finish current playback first");
              } else {
                showToast("TTS failed to initialize");
              }
              console.error("speak_text error:", err);
            }
          }
        )
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
    setup();
    return () => {
      unlistens.forEach((fn) => fn());
      stopProgressPolling();
    };
  }, [startProgressPolling, stopProgressPolling, showToast]);

  const togglePause = async () => {
    try {
      const paused = await invoke<boolean>("pause_resume_tts");
      setIsPaused(paused);
      if (paused) {
        pausedAtRef.current = Date.now();
        stopProgressPolling();
      } else {
        startProgressPolling(totalCharsRef.current, true);
      }
    } catch (err) {
      console.error("pause_resume error:", err);
    }
  };

  const cycleSpeed = async () => {
    const next = (speedIdx + 1) % SPEEDS.length;
    setSpeedIdx(next);
    try {
      await invoke("set_tts_rate", { rate: SPEEDS[next] });
    } catch (err) {
      console.error("set_tts_rate error:", err);
    }
  };

  const closeOverlay = async () => {
    stopProgressPolling();
    try {
      await getCurrentWindow().hide();
    } catch (err) {
      console.error("hide error:", err);
    }
  };

  const displayText = text.length > 500 ? text.slice(0, 500) + "..." : text;

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
      const books = await invoke<Book[]>("cmd_get_library");
      setLibrary(books);
    } catch (err) {
      console.error("Failed to load library:", err);
    }
  }, []);

  useEffect(() => {
    refreshLibrary();
  }, [refreshLibrary]);

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
      const baseDuration = (totalChars / 15) * 1000;
      setEstimatedTotal(baseDuration);

      progressTimer.current = setInterval(async () => {
        try {
          const [current, total, modeStr, isChunkDone] = await invoke<
            [number, number, string, boolean]
          >("get_speech_position");
          const pct = total > 0 ? Math.min((current / total) * 100, 100) : 0;
          setProgress(pct);
          setElapsed(Date.now() - startTimeRef.current);

          // Auto-advance: if playing a book and all chunks are done
          if (
            isChunkDone &&
            modeStr.startsWith("book:") &&
            autoAdvanceRef.current
          ) {
            autoAdvanceRef.current = false;
            stopProgressPolling();
            // Trigger auto-advance via event
            window.dispatchEvent(
              new CustomEvent("auto-advance-chapter", { detail: modeStr })
            );
          }
        } catch {
          setElapsed(Date.now() - startTimeRef.current);
          const est = baseDuration;
          setProgress(
            Math.min(((Date.now() - startTimeRef.current) / est) * 100, 99)
          );
        }
      }, 100);
    },
    [stopProgressPolling]
  );

  // Listen for auto-advance events
  useEffect(() => {
    const handler = async (e: Event) => {
      const modeStr = (e as CustomEvent).detail as string;
      // Parse "book:<id>:<chapter>"
      const parts = modeStr.split(":");
      if (parts.length < 3) return;
      const bookId = parts[1];
      const chapterIdx = parseInt(parts[2], 10);
      const nextIdx = chapterIdx + 1;

      if (!selectedBook || selectedBook.id !== bookId) return;
      if (nextIdx >= chapters.length) {
        showToast("Book finished!");
        return;
      }

      // Auto-advance to next chapter
      setCurrentChapterIdx(nextIdx);
      setIsPaused(false);
      setProgress(0);
      setElapsed(0);

      try {
        await invoke("cmd_save_reading_position", {
          bookId,
          chapter: nextIdx,
          position: 0,
        });
      } catch {}

      // Speak the next chapter
      const nextChapter = chapters.find((c) => c.index === nextIdx);
      if (nextChapter) {
        try {
          await invoke("speak_book_chapter", {
            text: nextChapter.content,
            bookId,
            chapterIndex: nextIdx,
            totalChapters: chapters.length,
          });
          autoAdvanceRef.current = true;
          startProgressPolling(nextChapter.content.length);
        } catch (err) {
          console.error("auto-advance speak error:", err);
        }
      }
    };

    window.addEventListener("auto-advance-chapter", handler);
    return () => window.removeEventListener("auto-advance-chapter", handler);
  }, [selectedBook, chapters, showToast, startProgressPolling]);

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
      await invoke("cmd_import_book", { filePath });
      await refreshLibrary();
      showToast("Book imported!");
    } catch (err) {
      showToast(`Import failed: ${err}`);
      console.error("Import error:", err);
    }
  };

  const handleOpenFileDialog = async () => {
    try {
      const path = await invoke<string | null>("cmd_open_file_dialog");
      if (path) await handleImport(path);
    } catch (err) {
      console.error("File dialog error:", err);
    }
  };

  const handleDeleteBook = async (bookId: string) => {
    try {
      await invoke("cmd_delete_book", { bookId });
      await refreshLibrary();
      showToast("Book deleted");
    } catch (err) {
      showToast(`Delete failed: ${err}`);
    }
  };

  const handleOpenBook = async (book: Book) => {
    try {
      const chs = await invoke<Chapter[]>("cmd_get_book_chapters", {
        bookId: book.id,
      });
      setSelectedBook(book);
      setChapters(chs);
      setCurrentChapterIdx(book.current_chapter);
      setIsPaused(false);
      setProgress(0);
      setElapsed(0);
      setView("reader");
    } catch (err) {
      showToast(`Failed to load book: ${err}`);
    }
  };

  const handleBackToLibrary = async () => {
    stopProgressPolling();
    autoAdvanceRef.current = false;
    try {
      await invoke("stop_tts");
    } catch {}
    if (selectedBook) {
      try {
        await invoke("cmd_save_reading_position", {
          bookId: selectedBook.id,
          chapter: currentChapterIdx,
          position: 0,
        });
      } catch {}
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
      await invoke("stop_tts");
    } catch {}

    setIsPaused(false);
    setProgress(0);
    setElapsed(0);

    try {
      await invoke("speak_book_chapter", {
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
      const paused = await invoke<boolean>("pause_resume_tts");
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
      await invoke("set_tts_rate", { rate: SPEEDS[next] });
    } catch (err) {
      console.error("set_tts_rate error:", err);
    }
  };

  const handleChapterChange = async (newIdx: number) => {
    stopProgressPolling();
    autoAdvanceRef.current = false;
    try {
      await invoke("stop_tts");
    } catch {}
    setIsPaused(false);
    setProgress(0);
    setElapsed(0);
    setCurrentChapterIdx(newIdx);
    if (selectedBook) {
      try {
        await invoke("cmd_save_reading_position", {
          bookId: selectedBook.id,
          chapter: newIdx,
          position: 0,
        });
      } catch {}
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
  const [windowLabel, setWindowLabel] = useState<string>("");

  useEffect(() => {
    try {
      const label = getCurrentWindow().label;
      setWindowLabel(label);
    } catch {
      setWindowLabel("main");
    }
  }, []);

  if (windowLabel === "overlay") {
    return <OverlayApp />;
  }
  return <MainApp />;
}

export default App;
