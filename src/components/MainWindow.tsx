// The main window: library <-> reader, plus file import.

import { useCallback, useEffect, useState } from "react";
import { Toast } from "./Toast";
import { LibraryView } from "./LibraryView";
import { ReaderView } from "./ReaderView";
import { describeError } from "../lib/errors";
import { fileNameOf, isSupportedFile } from "../lib/files";
import { useToast } from "../hooks/useToast";
import { useFileDrop } from "../hooks/useFileDrop";
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
  const [startPosition, setStartPosition] = useState(0);

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
        // The saved position belongs to the chapter it was saved against, so
        // it is only meaningful alongside that chapter. Moving to another
        // chapter clears it, which is what the handlers below do.
        setStartPosition(book.current_position);
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
    setStartPosition(0);
    void refreshLibrary();
  }, [refreshLibrary]);

  // Both of these change which chapter is showing, which is also what makes a
  // position recorded against the previous one meaningless.
  const handleChapterChange = useCallback((index: number) => {
    setCurrentChapterIdx(index);
    setStartPosition(0);
  }, []);

  const handleReadingPositionChange = useCallback((index: number) => {
    setCurrentChapterIdx(index);
    setStartPosition(0);
  }, []);

  // Drag and drop is handled by the OS via Tauri, not by the browser's HTML5
  // drop event, which cannot see filesystem paths inside a webview.
  const { isDragging } = useFileDrop(
    useCallback((paths: string[]) => void importPaths(paths), [importPaths]),
  );

  if (view === "reader" && selectedBook) {
    return (
      <>
        <ReaderView
          book={selectedBook}
          chapters={chapters}
          currentChapterIdx={currentChapterIdx}
          startPosition={startPosition}
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
        onMessage={showToast}
      />
      <Toast message={toast.message} />
    </>
  );
}
