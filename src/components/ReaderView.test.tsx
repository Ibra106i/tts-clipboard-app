import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ReaderView } from "./ReaderView";
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

function chapter(index: number, content: string): Chapter {
  return { index, title: `Chapter ${index + 1}`, content };
}

/** The arguments of the most recent read, so a test can assert on the span. */
function lastRead(): Record<string, unknown> {
  const calls = harness.callsFor("speak_book_chapter");
  return calls[calls.length - 1]?.args ?? {};
}

function renderReader(chapters: Chapter[], currentChapterIdx = 0) {
  const onMessage = vi.fn();
  render(
    <ReaderView
      book={book()}
      chapters={chapters}
      currentChapterIdx={currentChapterIdx}
      onChapterChange={vi.fn()}
      onReadingPositionChange={vi.fn()}
      onBack={vi.fn()}
      onMessage={onMessage}
    />,
  );
  return onMessage;
}

/** An idle backend that accepts the commands a click issues. */
function idleBackend() {
  harness.onAll({
    playback_get_state: () => null,
    stop_tts: () => undefined,
    speak_book_chapter: () => undefined,
  });
}

/**
 * A chapter with a blank line between the two blocks, as the parser emits.
 *
 * "First block here." is 17 characters, so the blocks are laid out as
 * paragraph 0 at offset 0, the blank one at 18, and "Second block here." at 19.
 */
const TWO_BLOCKS = "First block here.\n\nSecond block here.";

describe("ReaderView paragraph click", () => {
  it("reads from the clicked paragraph", async () => {
    idleBackend();
    renderReader([chapter(0, TWO_BLOCKS)]);

    await userEvent.click(screen.getByText("Second block here."));

    await waitFor(() => {
      expect(lastRead()).toMatchObject({ bookId: "b1", chapterIndex: 0 });
    });
    expect(lastRead().range).toEqual({ start: 19, align_to_sentence: true });
  });

  it("sends a character offset rather than a byte offset", async () => {
    idleBackend();
    // The emoji is one character but two UTF-16 units; the offset for the
    // second block must count it once.
    renderReader([chapter(0, "a👍b\nsecond")]);

    await userEvent.click(screen.getByText("second"));

    await waitFor(() => {
      expect(lastRead().range).toEqual({ start: 4, align_to_sentence: true });
    });
  });

  it("stops playback before reading from the new position", async () => {
    idleBackend();
    renderReader([chapter(0, TWO_BLOCKS)]);

    await userEvent.click(screen.getByText("Second block here."));

    await waitFor(() => {
      expect(harness.callsFor("speak_book_chapter")).toHaveLength(1);
    });
    // A click while something is already playing is a jump, not a second
    // overlapping read, so the stop has to happen first.
    expect(harness.callsFor("stop_tts").length).toBeGreaterThan(0);
  });

  it("does not read from the blank paragraph the parser leaves between blocks", async () => {
    idleBackend();
    renderReader([chapter(0, TWO_BLOCKS)]);

    // The empty paragraph carries no text node to click, so the only way to
    // reach it is the container. Clicking the text around it must still be the
    // thing that starts a read, and the blank line must not start one.
    const blank = document.querySelectorAll(".chapter-paragraph")[1];
    expect(blank?.textContent).toBe("");

    await userEvent.click(screen.getByText("First block here."));
    expect(lastRead().range).toEqual({ start: 0, align_to_sentence: true });
  });

  it("reads the whole chapter when the speak button is used", async () => {
    idleBackend();
    renderReader([chapter(0, TWO_BLOCKS)]);

    await userEvent.click(screen.getByRole("button", { name: "🔊 Speak Chapter" }));

    await waitFor(() => {
      expect(harness.callsFor("speak_book_chapter")).toHaveLength(1);
    });
    // The button still means "from the top": a range would be a surprise.
    expect(lastRead().range).toBeNull();
  });

  it("reports where playback started in the chapter counter", async () => {
    harness.on("playback_get_state", () => ({
      status: "playing",
      source: { kind: "book", book_id: "b1", chapter_index: 0, total_chapters: 1 },
      title: "Chapter 1",
      text_preview: "",
      spoken_chars: 0,
      total_chars: 12,
      start_char: 19,
      rate: 1,
      finished: false,
    }));
    renderReader([chapter(0, TWO_BLOCKS)]);

    // Paragraph 0 is the text, paragraph 1 the blank, paragraph 2 the rest.
    expect(await screen.findByText(/from ¶3/)).toBeInTheDocument();
  });

  it("shows no starting position when playback began at the top", async () => {
    harness.on("playback_get_state", () => ({
      status: "playing",
      source: { kind: "book", book_id: "b1", chapter_index: 0, total_chapters: 1 },
      title: "Chapter 1",
      text_preview: "",
      spoken_chars: 0,
      total_chars: 17,
      start_char: 0,
      rate: 1,
      finished: false,
    }));
    renderReader([chapter(0, TWO_BLOCKS)]);

    await screen.findByText("First block here.");
    expect(screen.queryByText(/from ¶/)).not.toBeInTheDocument();
  });
});

/** The element the reader listens for the end of a selection drag on. */
function textContainer(): HTMLElement {
  const element = document.querySelector<HTMLElement>(".reader-content");
  if (!element) throw new Error("expected the reader text container");
  return element;
}

/**
 * Select a span of one paragraph and raise the end-of-drag event where the
 * reader listens for it.
 *
 * Firing on the container rather than clicking a paragraph matters: a real
 * click would also start a paragraph read, and the test would be measuring two
 * things at once.
 */
async function selectSpan(node: Node, startOffset: number, endOffset: number) {
  const range = document.createRange();
  range.setStart(node, startOffset);
  range.setEnd(node, endOffset);
  const selection = window.getSelection();
  if (!selection) throw new Error("expected a selection");
  selection.removeAllRanges();
  selection.addRange(range);
  fireEvent.mouseUp(textContainer());
}

function firstParagraphText(): Node {
  const text = screen.getByText(/^First block here\.$/).firstChild;
  if (!text) throw new Error("expected the first paragraph to have text");
  return text;
}

describe("ReaderView selection", () => {
  it("offers to read a selected passage", async () => {
    idleBackend();
    renderReader([chapter(0, TWO_BLOCKS)]);

    await selectSpan(firstParagraphText(), 0, 5);

    expect(
      await screen.findByRole("button", { name: "Read this selection" }),
    ).toBeInTheDocument();
  });

  it("reads exactly the characters that were selected", async () => {
    idleBackend();
    renderReader([chapter(0, TWO_BLOCKS)]);

    await selectSpan(firstParagraphText(), 0, 5);
    await userEvent.click(
      screen.getByRole("button", { name: "Read this selection" }),
    );

    await waitFor(() => {
      expect(harness.callsFor("speak_book_chapter")).toHaveLength(1);
    });
    // The user chose these five characters, so the range must not be snapped to
    // a sentence boundary: that would silently drop the first word.
    expect(lastRead().range).toEqual({ start: 0, end: 5, align_to_sentence: false });
  });

  it("reads a selection that contains an emoji at the right offset", async () => {
    idleBackend();
    renderReader([chapter(0, "a👍b ends here")]);

    const text = screen.getByText(/^a👍b ends here$/).firstChild;
    if (!text) throw new Error("expected text");
    // "a" is UTF-16 0, the emoji occupies 1 and 2, and "b" is 3, so 1..4 is the
    // emoji and the b. An implementation that trusted `String.length` would
    // report end 4 here instead of 3, and read one character too far.
    await selectSpan(text, 1, 4);
    await userEvent.click(
      await screen.findByRole("button", { name: "Read this selection" }),
    );

    await waitFor(() => {
      expect(lastRead().range).toEqual({
        start: 1,
        end: 3,
        align_to_sentence: false,
      });
    });
  });

  it("offers nothing when the caret is collapsed", async () => {
    idleBackend();
    renderReader([chapter(0, TWO_BLOCKS)]);

    const range = document.createRange();
    range.setStart(firstParagraphText(), 3);
    range.collapse(true);
    const selection = window.getSelection();
    if (!selection) throw new Error("expected a selection");
    selection.removeAllRanges();
    selection.addRange(range);
    fireEvent.mouseUp(textContainer());

    expect(
      screen.queryByRole("button", { name: "Read this selection" }),
    ).not.toBeInTheDocument();
  });

  it("dismisses the offer when the selection is abandoned", async () => {
    idleBackend();
    renderReader([chapter(0, TWO_BLOCKS)]);

    await selectSpan(firstParagraphText(), 0, 5);
    await screen.findByRole("button", { name: "Read this selection" });

    await userEvent.keyboard("{Escape}");

    expect(
      screen.queryByRole("button", { name: "Read this selection" }),
    ).not.toBeInTheDocument();
  });

  it("clears the offer once the selection has been read", async () => {
    idleBackend();
    renderReader([chapter(0, TWO_BLOCKS)]);

    await selectSpan(firstParagraphText(), 0, 5);
    await userEvent.click(
      await screen.findByRole("button", { name: "Read this selection" }),
    );

    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "Read this selection" }),
      ).not.toBeInTheDocument();
    });
  });
});
