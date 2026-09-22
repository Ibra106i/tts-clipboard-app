// The main window: library <-> reader, plus file import.

import { useCallback, useEffect, useState } from "react";
import { Toast } from "./Toast";
import { LibraryView } from "./LibraryView";
import { ReaderView } from "./ReaderView";
import { describeError } from "../lib/errors";
import { fileNameOf, isSupportedFile } from "../lib/files";
import { useToast } from "../hooks/useToast";
import { subscribe } from "../lib/subscribe";
import * as ipc from "../lib/ipc";
import type { Book, Chapter } from "../lib/types";

type View = "library" | "reader";

export function MainWindow() {
  const [view, setView] = useState<View>("library");
  const [library, setLibrary] = useState<Book[]>([]);
  const [selectedBook, setSelectedBook] = useState<Book | null>(null);
  const [chapters, setChapters] = useState<Chapter[]>([]);
  const [currentChapterIdx, setCurrentChapterIdx] = useState(0);
  const [isDragging, setIsDragging] = useState(false);

  const toast = useToast();
  const showToast = toast.show;

  const refreshLibrary = useCallback(async () => {
    try {
      setLibrary(await ipc.getLibrary());
    } catch (err) {
      showToast(`Could not load the library: ${describeError(err)}`);
      console.error("library load failed:", err);
    }
  }, [showToast]);

  useEffect(() => {
    let cancelled = false;
    ipc
      .getLibrary()
      .then((books) => {
        if (!cancelled) setLibrary(books);
      })
      .catch((err) => {
        if (!cancelled) showToast(`Could not load the library: ${describeError(err)}`);
        console.error("library load failed:", err);
      });
    return () => {
      cancelled = true;
    };
  }, [showToast]);

  const importPaths = useCallback(
    async (paths: string[]) => {
      for (const path of paths) {
        const fileName = fileNameOf(path);
        if (!isSupportedFile(fileName)) {
          showToast(`${fileName} is not a PDF or EPUB file`);
          continue;
        }
        try {
          await ipc.importBook(path);
          showToast(`Imported ${fileName}`);
        } catch (err) {
          showToast(`Import failed: ${describeError(err)}`);
          console.error("import failed:", err);
        }
      }
      await refreshLibrary();
    },
    [refreshLibrary, showToast],
  );

  // Busy notifications while browsing the library. The reader owns this
  // subscription itself through `usePlayback`, so registering here only in the
  // library view keeps exactly one listener per window at any time.
  useEffect(() => {
    if (view !== "library") return;
    const subscription = subscribe("tts-busy", () => {
      showToast("Finish the current playback first");
    });
    return () => subscription.dispose();
  }, [showToast, view]);

  const handleOpenBook = useCallback(
    async (book: Book) => {
      try {
        const loaded = await ipc.getBookChapters(book.id);
        setSelectedBook(book);
        setChapters(loaded);
        setCurrentChapterIdx(book.current_chapter);
        setView("reader");
      } catch (err) {
        showToast(`Failed to load book: ${describeError(err)}`);
        console.error("open book failed:", err);
      }
    },
    [showToast],
  );

  const handleBackToLibrary = useCallback(() => {
    setView("library");
    setSelectedBook(null);
    setChapters([]);
    void refreshLibrary();
  }, [refreshLibrary]);

  const handleChapterChange = useCallback((index: number) => {
    setCurrentChapterIdx(index);
  }, []);

  const handleReadingPositionChange = useCallback((index: number) => {
    setCurrentChapterIdx(index);
  }, []);

  // ── Drag and drop ────────────────────────────────────────────────
  // Kept in the container because the drop target is the whole window.

  const handleDragOver = useCallback((event: React.DragEvent) => {
    if (event.dataTransfer.types.includes("Files")) {
      event.preventDefault();
      setIsDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback(() => {
    setIsDragging(false);
  }, []);

  const handleDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault();
      setIsDragging(false);
      const names = Array.from(event.dataTransfer.files).map(
        (file) => file.name,
      );
      if (names.length > 0) {
        void importPaths(names);
      }
    },
    [importPaths],
  );

  if (view === "reader" && selectedBook) {
    return (
      <>
        <ReaderView
          book={selectedBook}
          chapters={chapters}
          currentChapterIdx={currentChapterIdx}
          onChapterChange={handleChapterChange}
          onReadingPositionChange={handleReadingPositionChange}
          onBack={handleBackToLibrary}
          onMessage={showToast}
        />
        <Toast message={toast.message} />
      </>
    );
  }

  return (
    <>
      <LibraryView
        books={library}
        onRefresh={refreshLibrary}
        onOpenBook={(book) => void handleOpenBook(book)}
        isDragging={isDragging}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
        onMessage={showToast}
      />
      <Toast message={toast.message} />
    </>
  );
}
