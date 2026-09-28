import { describe, expect, it } from "vitest";
import * as ipc from "./ipc";
import { backendError, harness } from "../test/mocks/harness";

describe("typed IPC layer", () => {
  it("sends the library command with no arguments", async () => {
    harness.on("cmd_get_library", () => []);
    await ipc.getLibrary();
    expect(harness.callsFor("cmd_get_library")).toHaveLength(1);
  });

  it("maps camelCase chapter arguments onto the Rust parameter names", async () => {
    harness.on("speak_book_chapter", () => undefined);
    await ipc.speakBookChapter({
      text: "hello",
      bookId: "book-1",
      chapterIndex: 3,
      totalChapters: 10,
    });

    expect(harness.callsFor("speak_book_chapter")[0]?.args).toEqual({
      text: "hello",
      bookId: "book-1",
      chapterIndex: 3,
      totalChapters: 10,
      // A whole-chapter read sends an explicit null rather than omitting the
      // key, so the backend always sees the same argument shape.
      range: null,
    });
  });

  it("sends the character range unchanged when reading part of a chapter", async () => {
    harness.on("speak_book_chapter", () => undefined);
    await ipc.speakBookChapter({
      text: "hello",
      bookId: "book-1",
      chapterIndex: 3,
      totalChapters: 10,
      range: { start: 4_231, end: 5_904, align_to_sentence: false },
    });

    expect(harness.callsFor("speak_book_chapter")[0]?.args).toMatchObject({
      range: { start: 4_231, end: 5_904, align_to_sentence: false },
    });
  });

  it("keeps reading-position units on the caller's side", async () => {
    harness.on("cmd_save_reading_position", () => undefined);
    await ipc.saveReadingPosition({ bookId: "b", chapter: 2, position: 0 });
    expect(harness.callsFor("cmd_save_reading_position")[0]?.args).toEqual({
      bookId: "b",
      chapter: 2,
      position: 0,
    });
  });

  it("returns the picker result untouched, including cancellation", async () => {
    harness.on("cmd_open_file_dialog", () => null);
    await expect(ipc.openFileDialog()).resolves.toBeNull();

    harness.on("cmd_open_file_dialog", () => "C:/books/a.epub");
    await expect(ipc.openFileDialog()).resolves.toBe("C:/books/a.epub");
  });

  it("forwards playback state untouched", async () => {
    harness.on("playback_get_state", () => ({ status: "idle", source: null }));
    const snapshot = await ipc.getPlaybackState();
    expect(harness.callsFor("playback_get_state")).toHaveLength(1);
    expect(snapshot.status).toBe("idle");
  });

  it("surfaces backend rejections instead of swallowing them", async () => {
    harness.on("speak_text", () =>
      Promise.reject(backendError("busy", "Finish first"))
    );
    await expect(ipc.speakText("x")).rejects.toMatchObject({ code: "busy" });
  });
});
