// Shared types mirroring the Rust IPC contract in `src-tauri/src/models.rs`
// and `src-tauri/src/playback/state.rs`.
//
// These are the only place the frontend describes shapes coming across the
// bridge. If you change a serde attribute on the backend, change it here too
// (the Rust side has wire-shape tests that will fail first).

/** A book as stored in the library. */
export interface Book {
  id: string;
  title: string;
  file_path: string;
  file_type: string;
  imported_at: string;
  current_chapter: number;
  current_position: number;
  total_chapters: number;
}

/** One addressable unit of a document. */
export interface Chapter {
  index: number;
  title: string;
  content: string;
}

export type PlaybackStatus = "idle" | "playing" | "paused";

/** Discriminated union matching `PlaybackSource` on the Rust side. */
export type PlaybackSource =
  | { kind: "clipboard" }
  | {
      kind: "book";
      book_id: string;
      chapter_index: number;
      total_chapters: number;
    };

/**
 * What the UI needs to render playback.
 *
 * Units are explicit and must stay that way:
 * - `spoken_chars` / `total_chars` are **Unicode characters**, not bytes and
 *   not JavaScript UTF-16 code units. Anything that displays a percentage must
 *   divide these two directly, never `string.length`.
 */
export interface PlaybackSnapshot {
  status: PlaybackStatus;
  source: PlaybackSource | null;
  title: string;
  text_preview: string;
  spoken_chars: number;
  total_chars: number;
  rate: number;
  finished: boolean;
}

/** Payload of the `chapter-finished` event. */
export interface ChapterFinished {
  book_id: string;
  chapter_index: number;
  total_chapters: number;
}
