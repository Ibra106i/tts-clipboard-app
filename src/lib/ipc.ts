// The one place the frontend talks to the backend.
//
// Every UI component calls these functions instead of `invoke(...)`, so command
// names, argument shapes and payload types live in exactly one file. That is
// what lets the compiler catch a renamed command or a changed payload shape.

import { invoke } from "@tauri-apps/api/core";
import type { Book, Chapter, PlaybackSnapshot } from "./types";

// ── Playback ───────────────────────────────────────────────────────

/** Speak arbitrary text (used for the clipboard flow, if the UI ever drives it). */
export function speakText(text: string): Promise<void> {
  return invoke("speak_text", { text });
}

/** Speak a chapter of a book, identifying it for auto-advance. */
export function speakBookChapter(args: {
  text: string;
  bookId: string;
  chapterIndex: number;
  totalChapters: number;
}): Promise<void> {
  return invoke("speak_book_chapter", {
    text: args.text,
    bookId: args.bookId,
    chapterIndex: args.chapterIndex,
    totalChapters: args.totalChapters,
  });
}

/** Pause or resume; resolves to the new paused state. */
export function pauseResume(): Promise<boolean> {
  return invoke("pause_resume_tts");
}

/** Stop playback entirely. */
export function stop(): Promise<void> {
  return invoke("stop_tts");
}

/** Set the playback rate multiplier (0.5-4.0). */
export function setRate(rate: number): Promise<void> {
  return invoke("set_tts_rate", { rate });
}

/** Read the current playback state. Purely observational. */
export function getPlaybackState(): Promise<PlaybackSnapshot> {
  return invoke("playback_get_state");
}

// ── Library ────────────────────────────────────────────────────────

export function getLibrary(): Promise<Book[]> {
  return invoke("cmd_get_library");
}

export function getBookChapters(bookId: string): Promise<Chapter[]> {
  return invoke("cmd_get_book_chapters", { bookId });
}

export function importBook(filePath: string): Promise<Book> {
  return invoke("cmd_import_book", { filePath });
}

export function deleteBook(bookId: string): Promise<void> {
  return invoke("cmd_delete_book", { bookId });
}

export function saveReadingPosition(args: {
  bookId: string;
  chapter: number;
  position: number;
}): Promise<void> {
  return invoke("cmd_save_reading_position", {
    bookId: args.bookId,
    chapter: args.chapter,
    position: args.position,
  });
}

/** Open the native file picker; resolves to `null` when the user cancels. */
export function openFileDialog(): Promise<string | null> {
  return invoke("cmd_open_file_dialog");
}

/** Reveal the rotated log files in the OS file browser. */
export function openLogsFolder(): Promise<void> {
  return invoke("cmd_open_logs_folder");
}
