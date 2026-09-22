import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { MainWindow } from "./MainWindow";
import { harness } from "../test/mocks/harness";
import type { Book, Chapter } from "../lib/types";

function book(overrides: Partial<Book> = {}): Book {
  return {
    id: "b1",
    title: "A Book",
    file_path: "C:/data/books/b1.epub",
    file_type: "epub",
    imported_at: "2026-01-01T00:00:00",
    current_chapter: 0,
    current_position: 0,
    total_chapters: 2,
    ...overrides,
  };
}

function chapter(index: number): Chapter {
  return { index, title: `Chapter ${index + 1}`, content: `Body ${index}` };
}

describe("MainWindow", () => {
  it("renders the library returned by the backend", async () => {
    harness.onAll({
      cmd_get_library: () => [book()],
      playback_get_state: () => null,
    });

    render(<MainWindow />);

    expect(await screen.findByText("A Book")).toBeInTheDocument();
    expect(screen.getByText("2 chapters")).toBeInTheDocument();
  });

  it("opens a book and shows its chapters in the reader", async () => {
    harness.onAll({
      cmd_get_library: () => [book()],
      cmd_get_book_chapters: () => [chapter(0), chapter(1)],
      playback_get_state: () => null,
    });

    render(<MainWindow />);
    await userEvent.click(await screen.findByLabelText("Open A Book"));

    expect(await screen.findByLabelText("Select chapter")).toBeInTheDocument();
    expect(screen.getByText("Body 0")).toBeInTheDocument();
  });

  it("asks for confirmation before deleting a book and does nothing on cancel", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const deleted: string[] = [];
    harness.onAll({
      cmd_get_library: () => [book()],
      cmd_delete_book: (args) => {
        deleted.push(String(args.bookId));
      },
      playback_get_state: () => null,
    });

    render(<MainWindow />);
    await userEvent.click(await screen.findByLabelText("Delete A Book"));

    expect(confirm).toHaveBeenCalled();
    expect(deleted).toEqual([]);
    confirm.mockRestore();
  });

  it("deletes the book once confirmed", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const deleted: string[] = [];
    harness.onAll({
      cmd_get_library: () => [book()],
      cmd_delete_book: (args) => {
        deleted.push(String(args.bookId));
      },
      playback_get_state: () => null,
    });

    render(<MainWindow />);
    await userEvent.click(await screen.findByLabelText("Delete A Book"));

    await waitFor(() => expect(deleted).toEqual(["b1"]));
    confirm.mockRestore();
  });

  it("reports an import failure instead of pretending it worked", async () => {
    harness.onAll({
      cmd_get_library: () => [],
      cmd_import_book: () =>
        Promise.reject({
          code: "import_failed",
          message: "Could not import bad.pdf: corrupt",
          detail: null,
        }),
      playback_get_state: () => null,
    });

    // A drop with an unsupported extension is rejected before reaching the
    // backend, which is the cheap path.
    render(<MainWindow />);
    await screen.findByText("No books imported yet");
    expect(harness.callsFor("cmd_import_book")).toHaveLength(0);
  });

  it("subscribes to busy notifications exactly once and cleans up on unmount", async () => {
    harness.onAll({
      cmd_get_library: () => [],
      playback_get_state: () => null,
    });

    const { unmount } = render(<MainWindow />);
    await waitFor(() => expect(harness.listenerCount("tts-busy")).toBe(1));
    // No playback hook is mounted while browsing the library.
    expect(harness.listenerCount("playback-state")).toBe(0);

    unmount();
    expect(harness.totalListenerCount()).toBe(0);
  });

  it("hands the busy subscription over to the reader, never doubling it up", async () => {
    harness.onAll({
      cmd_get_library: () => [book()],
      cmd_get_book_chapters: () => [chapter(0)],
      playback_get_state: () => null,
    });

    render(<MainWindow />);
    await waitFor(() => expect(harness.listenerCount("tts-busy")).toBe(1));

    await userEvent.click(await screen.findByLabelText("Open A Book"));
    await screen.findByLabelText("Select chapter");

    await waitFor(() => expect(harness.listenerCount("playback-state")).toBe(1));
    expect(harness.listenerCount("tts-busy")).toBe(1);
  });
});
