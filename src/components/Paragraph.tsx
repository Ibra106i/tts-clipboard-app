// One paragraph of a chapter.
//
// Extracted and memoised because the reader re-renders four times a second while
// audio plays, and a long chapter is thousands of these. When the row was
// inlined in the map, every one of them was rebuilt - element, closure, class
// name string - on every tick, and then diffed. Memoising on the props that
// actually change means a tick only re-renders the paragraph whose reading
// state flipped, plus the two at the boundary.

import { memo } from "react";
import { BLOCK_OFFSET_ATTR } from "../lib/offsets";

export interface ParagraphProps {
  index: number;
  /** The paragraph's own text, exactly as it appears in the chapter. */
  text: string;
  /** Character offset of this paragraph within the chapter. */
  startChar: number;
  /** True while the voice is on this paragraph. */
  isReading: boolean;
  /** Called with the paragraph index when the play target is used. */
  onReadFrom?: (index: number) => void;
}

function ParagraphImpl({
  index,
  text,
  startChar,
  isReading,
  onReadFrom,
}: ParagraphProps) {
  // The parser separates blocks with a blank line and the reader splits on a
  // single newline, so every block break renders an empty paragraph. There is
  // nothing in one to read, so it must not offer to read.
  const speakable = Boolean(text.trim());

  return (
    <p
      className={
        isReading ? "chapter-paragraph is-reading" : "chapter-paragraph"
      }
      {...{ [BLOCK_OFFSET_ATTR]: startChar }}
    >
      {/*
        A real button, because that is what makes the target obvious and what a
        screen reader announces as actionable. It is kept out of the tab order on
        purpose: a focusable element per block would put a tab stop on every
        paragraph of every chapter and make a long one unusable to traverse. The
        chapter select and Speak Chapter remain the keyboard route to a position.

        The icon is an SVG rather than a glyph character on purpose. A glyph
        would be a text node inside the paragraph, and the offset mapping walks
        the chapter's text nodes to answer "which character is this?" - a stray
        character would shift every offset after it.
      */}
      {speakable ? (
        <button
          type="button"
          className="paragraph-play"
          tabIndex={-1}
          onClick={onReadFrom ? () => onReadFrom(index) : undefined}
          aria-label={`Read from paragraph ${index + 1}`}
          title="Read from here"
        >
          <svg
            className="paragraph-play-icon"
            viewBox="0 0 10 10"
            width="9"
            height="9"
            aria-hidden="true"
            focusable="false"
          >
            <path d="M1 0.5 L9 5 L1 9.5 Z" fill="currentColor" />
          </svg>
        </button>
      ) : null}
      <span className="paragraph-body">{text}</span>
    </p>
  );
}

export const Paragraph = memo(ParagraphImpl);
