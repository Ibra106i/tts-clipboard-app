# Release Notes — v1.0.0

This release takes the app from an audited 3.5/10 prototype to a tested, hardened
1.0 across 35 remediation commits. Highlights below; see the git log for the full
per-commit record.

## Highlights

### Correctness
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
- **Atomic `library.json` writes** (temp file + rename) — a crash mid-write can
  no longer destroy the library.
- **Transactional imports** — failed PDF/EPUB imports roll back fully; no stray
  files remain in `books/`.
- **Dedupe + size guards** — duplicate content is rejected via content hash,
  and oversized files are rejected before any parsing work.
- Reading-position persistence and book deletion behavior preserved.

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
- The 700-line `App.tsx` split into focused components, a shared `usePlayback`
  hook, a typed IPC layer shared by both halves, and leak-free async listener
  lifecycle.
- Platform boundary made explicit (Windows vs stub backend).
- Dead template assets removed, `.gitignore` covers Rust/Tauri build output,
  README rewritten, dependencies pruned and refreshed.

### Quality gates
- 78 Rust unit tests + 51 frontend tests, clippy and rustfmt clean, strict
  TypeScript, oxlint, `npm audit` clean, release profile tuned, and a CI
  workflow running typecheck, lint, tests, and a Windows Tauri build.
- Always-on logging observable in release builds.

## Known limitations
- Windows-only (SAPI 5); other platforms report `unsupported_platform`.
- Large PDFs with complex layouts may still extract imperfectly.
