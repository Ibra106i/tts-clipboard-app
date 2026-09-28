# TTS Clipboard & Document Reader

A Windows desktop application that reads documents and clipboard text aloud,
built with Tauri v2, React 19, TypeScript and Windows SAPI 5.

## Features

- **Global hotkey** (Ctrl+Shift+Space): speaks the current clipboard text in a
  floating overlay, no matter which app you are in.
- **Library**: import PDF and EPUB documents, browse them, resume where you
  left off, and delete what you no longer want.
- **Read from anywhere**: click a paragraph to start reading at the sentence
  from there onwards, or select a passage and read exactly that.
- **Playback**: play, pause, resume, stop, variable speech rate, live progress
  with elapsed/estimated-total time, and automatic progression to the next
  chapter.
- **Tray**: closing the main window keeps the app in the tray; the hotkey and
  playback keep working.
- **Observability**: rotating logs on disk (never containing clipboard or
  document content), viewable from the tray menu.

## Development setup

Prerequisites:

- Windows 10/11 (speech uses SAPI 5; other platforms compile but report an
  explicit unsupported-platform error at runtime)
- Node.js 20+
- Rust (stable) with the MSVC toolchain; `rust-toolchain.toml` pins the channel
- The Visual Studio C++ Build Tools (required by the MSVC toolchain)

Install and run:

```bash
npm install
npm run tauri dev
```

The first Rust build takes a few minutes; subsequent builds are incremental.

## Commands

| Command                | What it does                                  |
| ---------------------- | --------------------------------------------- |
| `npm run tauri dev`    | Run the desktop app with hot reload           |
| `npm run dev`          | Frontend only (in a browser, without Tauri)   |
| `npm run build`        | Production frontend bundle (`dist/`)          |
| `npm run typecheck`    | TypeScript project check                      |
| `npm run lint`         | oxlint with warnings denied                   |
| `npm test`             | Vitest unit tests (jsdom + Testing Library)   |
| `npm run test:rust`    | Rust unit tests only (`cargo test`)           |
| `npm run fmt:rust:check` | Check Rust formatting (`cargo fmt --check`) |
| `npm run lint:rust`    | Clippy with warnings denied                   |
| `npm run verify`       | Everything CI runs, in one command            |
| `npm run tauri build`  | Release installer/binary                      |

`npm run verify` is the gate: typecheck, oxlint, Vitest, rustfmt, clippy and
`cargo test`. CI runs the same six steps individually.

Rust-side checks can also be run directly from `src-tauri/`:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

## Architecture overview

```
src/                       React frontend
  components/              MainWindow, LibraryView, ReaderView, Overlay, Toast
  hooks/usePlayback.ts     single subscription to playback state for every window
  lib/ipc.ts               typed wrappers over the Tauri commands
  lib/offsets.ts           DOM/character offset mapping, in code points
  lib/errors.ts            normalises backend structured errors for display
src-tauri/                 Rust backend
  src/playback/            the playback actor
    actor.rs               the one thread that owns the engine and state machine
    state.rs               platform-agnostic playback state machine (pure, unit-tested)
    speaker_windows.rs     SAPI 5 engine, created and used on the actor thread
  src/text.rs              character-based chunking and sentence alignment
  src/models.rs            the persisted schema, including the read range
  src/library.rs           import/dedup/persistence (atomic JSON writes)
  src/parser.rs            PDF (lopdf) and EPUB (epub + scraper) extraction
  src/error.rs             structured error model surfaced to the frontend
```

Key invariants:

- **One writer per subsystem.** Playback state is owned by the actor thread;
  commands and UI only observe snapshots. Persistence goes through
  `library.rs` with atomic temp-file-then-rename writes.
- **Unicode correctness.** Text is chunked and measured in characters, never
  bytes or UTF-16 units; APIs are named after the unit they use. This holds
  across the bridge too: the DOM hands out UTF-16 offsets, so
  `lib/offsets.ts` re-counts every position in code points before sending it.
  The `codePointLength` tests are the guard, not a formality.
- **No hidden state machines.** Windows communicate through typed backend
  events (`playback-state`, `chapter-finished`), not browser CustomEvents.
- **Errors are structured and surfaced.** Every failure has a code, a
  user-facing message and optional detail; nothing is swallowed.

## Data locations

User data lives in the OS app-data directory for `com.tts.clipboard-app`:

- `library.json` — catalogue of imported books (atomic writes)
- `books/` — copies of imported documents

## Troubleshooting

- **No sound**: check that a default voice exists in Windows settings
  (Time & language → Speech). The app reports a structured error at startup if
  the speech engine cannot be created.
- **Hotkey does nothing**: another application may own Ctrl+Shift+Space; the
  conflict is logged. Logs are accessible from the tray menu.
