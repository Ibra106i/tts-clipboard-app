# Release Notes — v1.0.0

This release takes the app from an audited 3.5/10 prototype to a tested, hardened
1.0 across 35 remediation commits. Highlights below; see the git log for the full
per-commit record.

## Highlights

### Correctness
- **Read from anywhere in a chapter** — clicking a paragraph starts speech at
  the first sentence at or after it, rather than always at the top of the
  chapter. Offsets are Unicode characters end to end: the DOM reports UTF-16
  offsets, so every position crossing into the backend is re-counted in code
  points, and a range that lands between the halves of a surrogate pair is
  snapped back to a real character boundary.
- **Read a selected passage** — highlighting text offers to read exactly the
  characters selected, as its own job so the progress bar measures the
  selection rather than the chapter. A click is snapped to a sentence because
  the speech engine cannot begin mid-utterance; a selection is not, because the
  user chose those words.
- **Unicode-safe chunking** — text is split by character count, not bytes, so
  multi-byte input (CJK, emoji, accents) can no longer panic or corrupt output.
  A dedicated actor, exhaustive property-style tests, and per-script fixtures
  guard the invariant.
- **Single playback owner** — playback moved to a dedicated actor thread that
  owns both the SAPI engine and all playback state. This removes the
  `unsafe impl Send/Sync` on the COM object, five independently-locked fields,
  and a class of lock-inversion deadlocks.
- **One progress unit** — all progress is measured in spoken characters;
  the mixed unit/percentage model is gone, and the backend reports one
  canonical `PlaybackState` snapshot per change.
- **Hotkey no longer double-speaks** — one clipboard read, one speak call per
  hotkey press; the overlay is a pure view of playback state.
- **Chapter auto-advance** driven by typed backend events instead of a window
  `CustomEvent` state machine or polling that advanced playback.
- **Correct PDF text decoding** via `lopdf::extract_text`; EPUB text is
  entity-decoded and inline tags no longer break sentences mid-flow.

### Persistence & data safety
- **The reading position is now real.** `Book.current_position` was written as
  `0` by every call site and never read back; it now records the character
  offset a read began at, is restored when a book is reopened, and is written
  when the reader leaves a chapter as well as the book. A position that does
  not fit the book — a shorter edition replacing a longer one — is dropped with
  a log line rather than stored, so a stale offset can never point past the end
  of a chapter.
- **Atomic `library.json` writes** (temp file + rename) — a crash mid-write can
  no longer destroy the library.
- **Transactional imports** — failed PDF/EPUB imports roll back fully; no stray
  files remain in `books/`.
- **Dedupe + size guards** — duplicate content is rejected via content hash,
  and oversized files are rejected before any parsing work.

### Concurrency
- Tests prove concurrent playback commands cannot deadlock, and that the
  speech engine is created and stays on the playback thread.
- Dialog and import work moved off the async runtime's threads.

### Security
- **Least-privilege capabilities** — the webview no longer holds fs, clipboard
  or dialog plugin permissions; all I/O goes through audited Rust commands.
- **Content Security Policy enabled** — script access is same-origin only.
- Structured error model replaces opaque string errors end to end.

### Architecture & maintainability
- One `TextRange` type and one command serve both ways of starting mid-chapter.
  A click and a selection differ only in the fields they send, which keeps the
  IPC layer at thirteen wrappers and thirteen registered commands.
- The 700-line `App.tsx` split into focused components, a shared `usePlayback`
  hook, a typed IPC layer shared by both halves, and leak-free async listener
  lifecycle.
- Platform boundary made explicit (Windows vs stub backend).
- Dead template assets removed, `.gitignore` covers Rust/Tauri build output,
  README rewritten, dependencies pruned and refreshed.

### Quality gates
- 101 Rust unit tests + 91 frontend tests, clippy and rustfmt clean, strict
  TypeScript, oxlint, `npm audit` clean, release profile tuned, and a CI
  workflow running typecheck, lint, tests, and a Windows Tauri build.
- Always-on logging observable in release builds.

## Known limitations
- Windows-only (SAPI 5); other platforms report `unsupported_platform`.
- Large PDFs with complex layouts may still extract imperfectly.
- Clicking a paragraph and reading a selection are pointer gestures. Full
  keyboard access to a position is not offered, because a focusable element per
  paragraph would put a tab stop on every block of every chapter; the chapter
  select and the Speak Chapter button remain the keyboard route.
- Reopening a book restores the chapter and the character offset, but does not
  scroll the text to it — the reader's scroll container is not scripted yet.
- A selection that spans a chapter boundary cannot be read: chapter text is
  stored per chapter, so the offer does not appear.
