# DESIGN.md — Design System & Visual Architecture

## 1. Overview & Aesthetic Strategy

The visual identity of the **Tauri Clipboard App** balances high legibility for
long reading sessions with clear feedback for SAPI 5 audio playback state.

The colour palette is built around a warm **Mustard & Warm Dark Slate** theme.
It replaces the legacy violet accent with high-visibility amber and gold tones,
pairing vibrant highlights with muted golden-olive surfaces for secondary
controls, overlays, and paragraph active states.

## 2. Colour Palette & Semantic Tokens

### Palette

| Role | Hex | RGB |
|------|-----|-----|
| Bright Amber | `#ffa700` | 255, 167, 0 |
| Warm Amber | `#cc7b00` | 204, 123, 0 |
| Gold | `#d0a800` | 208, 168, 0 |
| Muted Gold | `#bf9a00` | 191, 154, 0 |
| Olive Gold | `#9c7e00` | 156, 126, 0 |

### Tokens (`src/App.css`)

```css
:root {
  /* Surface layers — dark slate with warm golden undertones */
  --bg-app: #0f0e0a;
  --bg-surface: #171510;
  --bg-surface-hover: #221f17;
  --bg-surface-active: #2e2a1f;

  /* Borders and dividers */
  --border-subtle: rgba(255, 255, 255, 0.07);
  --border-amber: rgba(204, 123, 0, 0.4);
  --border-focus: #ffa700;

  /* Primary accent */
  --accent-primary: #ffa700;
  --accent-primary-hover: #cc7b00;
  --accent-secondary: #d0a800;
  --accent-muted: #bf9a00;

  /* Reading and highlight states */
  --reader-active-bg: rgba(156, 126, 0, 0.18);
  --reader-active-border: #ffa700;
  --reader-hover-bg: rgba(255, 255, 255, 0.02);

  /* Typography */
  --text-main: #fcfaf4;
  --text-muted: #a39e8c;
  --text-accent: #ffa700;

  /* Layout */
  --reader-max-width: 70ch;
  --radius-sm: 4px;
  --radius-md: 8px;
  --radius-lg: 12px;
}
```

### Contrast, measured

Ratios were computed rather than assumed. Every pair below is WCAG AA compliant.

| Pair | Ratio | Verdict |
|------|-------|---------|
| `--text-main` on `--bg-app` | 18.50 | AAA |
| `--text-main` on `--bg-surface` | 17.48 | AAA |
| `--text-muted` on `--bg-surface` | 6.80 | AA |
| `--text-muted` on `--bg-surface-hover` | 6.13 | AA |
| `--accent-primary` as text on `--bg-surface` | 9.36 | AAA |
| `--text-ink` on `--accent-primary` fill | 9.91 | AAA |
| `--border-focus` on `--bg-app` | 9.91 | AAA |

**One deliberate departure from the original spec.** The spec's amber fills were
implicitly paired with white labels. That combination measures **1.95:1** and
fails AA for text by a wide margin, so it is not used anywhere. Labels on an
amber fill use `--text-ink` (near-black) at 9.91:1 instead. Amber fills read as
light, so their label colour follows the fill rather than the theme.

## 3. Reader View Architecture & DOM Constraints

### Typography and layout

- Measure is fixed at `70ch`, centred, to hold the eye steady over a long session.
  This replaced a `720px` measure, which was narrower on a large font and wider
  on a small one ? so the line length moved with whatever the reader had chosen
  system-wide, which is the opposite of what a fixed measure is for.
- `line-height: 1.7`, `margin-bottom: 1.25rem` between paragraphs.

### Character-offset discipline

The reader maps clicks and selections to **Unicode code points**, while the DOM
reports UTF-16 offsets. An emoji is one code point and two UTF-16 units, so any
stray character inside a paragraph shifts every offset after it.

- Every block keeps `data-start-char`.
- **No text-based icon glyphs or HTML entities inside reader paragraphs.** Icons
  are inline SVG, which contributes no text node. A `▶` character inside a button
  is sufficient to corrupt the mapping.
- Active-paragraph styling is applied through a class on the block. It never
  adds, removes, or wraps a child node.

```css
.chapter-paragraph.is-reading {
  background-color: var(--reader-active-bg);
  border-left: 3px solid var(--reader-active-border);
  padding-left: 1rem;
  border-radius: var(--radius-sm);
}
```

Note the class name is `chapter-paragraph`, matching the component and its
tests, rather than the `reader-paragraph` used in early drafts.

## 4. Overlay Mini-Player

- Background `#171510` with `backdrop-filter: blur(12px)` and a `#9c7e00` border.
- Progress gradient from `#ffa700` to `#d0a800`, on a track of `--accent-muted`
  at 20% opacity.
- Controls: `#ffa700` fill on hover, `--accent-primary-hover` while pressed.

## 5. Accessibility

Decisions that are part of the system rather than a later pass.

| Decision | Why |
|----------|-----|
| Focus rings use ink on amber controls | An amber outline on an amber fill disappears ? exactly where a keyboard user most needs to see it. An ink ring with an amber outer ring separates the two. |
| `:focus-visible`, not `:focus` | A pointer click leaves no ring behind; keyboard focus always does. The previous `:focus` rule removed the outline outright, leaving no visible indicator at all. |
| Delete button appears on focus | A control revealed only on hover is unreachable by keyboard. |
| Reduced-motion honoured | The drop bounce and toast slide are decorative. |
| Forced-colors fallback | Under a forced palette the app's tokens are replaced by the user's, which otherwise left the play button invisible and the reading paragraph unstyled. |
| Pressed states change colour, not geometry | A transform moves surrounding content and makes a button shift under the pointer. |
| Status colours clear of the amber ramp | A red error must never be mistaken for the accent. |

## 6. Performance & Render Integrity

To stay smooth under the 250 ms state pushes from the Rust/SAPI thread:

- Paragraph transitions use compositor-friendly properties only —
  `background-color`, `border-color`, `opacity`. No `transform` on rows, since a
  transition there would run during every tick.
- The active-paragraph indicator switches purely through a class change, so a
  tick redraws the two paragraphs whose state flipped and nothing else.
- `content-visibility: auto` with `contain-intrinsic-size` lets the browser skip
  layout and paint for paragraphs scrolled out of view.