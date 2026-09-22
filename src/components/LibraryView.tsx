// The library: import, browse and delete books.

import { useCallback } from "react";
import { describeError } from "../lib/errors";
import * as ipc from "../lib/ipc";
import type { Book } from "../lib/types";
import { formatPercent } from "../lib/format";

export interface LibraryViewProps {
  books: Book[];
  onRefresh: () => Promise<void>;
  onOpenBook: (book: Book) => void;
  isDragging: boolean;
  onDragOver: (event: React.DragEvent) => void;
  onDragLeave: () => void;
  onDrop: (event: React.DragEvent) => void;
  /** Where this view reports successes and failures. The container owns the
   * single toast for the window, so two views cannot show two messages. */
  onMessage: (message: string) => void;
}

export function LibraryView({
  books,
  onRefresh,
  onOpenBook,
  isDragging,
  onDragOver,
  onDragLeave,
  onDrop,
  onMessage: showToast,
}: LibraryViewProps) {

  const handleOpenFileDialog = useCallback(async () => {
    try {
      const path = await ipc.openFileDialog();
      if (!path) return;
      await ipc.importBook(path);
      await onRefresh();
      showToast("Book imported");
    } catch (err) {
      showToast(`Import failed: ${describeError(err)}`);
      console.error("import failed:", err);
    }
  }, [onRefresh, showToast]);

  const handleDeleteBook = useCallback(
    async (book: Book) => {
      const confirmed = window.confirm(
        `Delete "${book.title}"? The imported copy will be removed from disk.`,
      );
      if (!confirmed) return;
      try {
        await ipc.deleteBook(book.id);
        await onRefresh();
        showToast("Book deleted");
      } catch (err) {
        showToast(`Delete failed: ${describeError(err)}`);
        console.error("delete failed:", err);
      }
    },
    [onRefresh, showToast],
  );

  return (
    <div className="app-root" onDragOver={onDragOver} onDragLeave={onDragLeave} onDrop={onDrop}>
      <header className="app-header">
        <h1 className="app-title">📚 TTS Library</h1>
        <button
          type="button"
          className="btn-import"
          onClick={handleOpenFileDialog}
        >
          + Import Book
        </button>
      </header>

      {isDragging && (
        <div className="drop-zone">
          <div className="drop-zone-content">
            <div className="drop-icon" aria-hidden="true">
              📥
            </div>
            <div>Drop PDF or EPUB files here</div>
          </div>
        </div>
      )}

      {books.length === 0 ? (
        <div className="empty-state">
          <div className="empty-icon" aria-hidden="true">
            📖
          </div>
          <p>No books imported yet</p>
          <p className="empty-hint">
            Click &quot;Import Book&quot; or drag &amp; drop files here
          </p>
        </div>
      ) : (
        <ul className="book-grid">
          {books.map((book) => (
            <li key={book.id} className="book-card">
              <button
                type="button"
                className="book-open"
                onClick={() => onOpenBook(book)}
                aria-label={`Open ${book.title}`}
              >
                <span className="book-card-header">
                  <span className="book-title">{book.title}</span>
                  <span className={`book-badge badge-${book.file_type}`}>
                    {book.file_type.toUpperCase()}
                  </span>
                </span>
                <span className="book-meta">{book.total_chapters} chapters</span>
                <span className="book-progress">
                  <span className="book-progress-track">
                    <span
                      className="book-progress-fill"
                      style={{
                        width: formatPercent(
                          book.total_chapters > 0
                            ? (book.current_chapter / book.total_chapters) * 100
                            : 0,
                        ),
                      }}
                    />
                  </span>
                  <span className="book-progress-text">
                    Ch. {book.current_chapter + 1}/{book.total_chapters}
                  </span>
                </span>
              </button>
              <button
                type="button"
                className="btn-delete-book"
                onClick={() => void handleDeleteBook(book)}
                aria-label={`Delete ${book.title}`}
                title="Delete book"
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
