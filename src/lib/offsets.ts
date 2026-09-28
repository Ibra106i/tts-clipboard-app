// Mapping between what the reader has on screen and the character offsets the
// backend speaks in.
//
// The unit rule, and why it is not negotiable: `chapter.content` is a Rust
// `String`, and every offset crossing the bridge is a **Unicode character** (a
// code point) — the same unit `text::Chunk::start_char` and
// `PlaybackSnapshot::spoken_chars` are measured in. JavaScript strings are
// sequences of UTF-16 code units, and `String.prototype.length` counts *those*.
// For any character outside the Basic Multilingual Plane — an emoji, a regional
// indicator pair, a supplementary CJK ideograph — the two counts differ, and
// they differ by one per character.
//
// So `a👍b` has a `.length` of 4 and a character count of 3. Every offset in
// this file is a character count, and no function here may use `.length` to
// produce one. The test `counts code points rather than utf16 units` is the gate
// that keeps it that way.

/**
 * Number of Unicode characters (code points) in `value`.
 *
 * Implemented over UTF-16 code units rather than `[...value].length` so that
 * measuring a whole chapter allocates nothing: these offsets are computed for
 * every paragraph on every render of a long chapter.
 */
export function codePointLength(value: string): number {
  let count = 0;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    // A high surrogate is the first half of a character outside the Basic
    // Multilingual Plane. Its low surrogate follows in the next unit and is
    // not a character of its own, so both units count as one.
    if (code >= 0xd800 && code <= 0xdbff) {
      index += 1;
    }
    count += 1;
  }
  return count;
}

/**
 * Character offset of the start of each paragraph, for text rendered as one
 * element per `split("\n")` segment.
 *
 * The result always has one entry per segment, in order, and each entry is the
 * offset of that segment's first character within the whole chapter. Joining the
 * segments with `"\n"` reproduces the input exactly, so the offsets are exact
 * rather than approximate.
 *
 * The document parser separates blocks with a blank line, and the reader splits
 * on a single `"\n"`, so blank lines appear in the output as empty paragraphs
 * with nothing to read. Their offsets are still reported correctly; it is up to
 * the caller to skip them.
 */
export function paragraphStarts(chapterText: string): number[] {
  const segments = chapterText.split("\n");
  const starts: number[] = [];
  let offset = 0;

  for (const segment of segments) {
    starts.push(offset);
    // The separator itself is one character, and is counted like any other.
    offset += codePointLength(segment) + 1;
  }

  return starts;
}

/**
 * Character offset of a DOM point within the chapter text, or `null` when the
 * point is not inside `container`.
 *
 * A `Range` reports its endpoints as a text node plus a UTF-16 offset into that
 * node, so both have to be converted: the node walk accumulates code points, and
 * the in-node offset is re-measured in code points as well. Walking with a
 * `TreeWalker` rather than counting paragraph elements is what makes this work
 * for selections that start or end part-way through a paragraph.
 *
 * A point that lands between two characters never has an in-node offset inside
 * a surrogate pair, but if one is produced anyway the result is snapped back to
 * the start of that pair, because half a character is not a position anyone can
 * read from.
 */
export function pointToCharOffset(
  container: HTMLElement,
  targetNode: Node,
  targetOffset: number,
): number | null {
  if (!container.contains(targetNode)) {
    return null;
  }

  // The walk is per block rather than per text node, and that is the whole
  // trick. The reader renders a chapter as one element per `split("\n")`
  // segment, which means the newline characters are consumed by the split and
  // are **not in the DOM at all**. A walk over text nodes alone is therefore
  // short by one per paragraph and can never recover the chapter's offsets.
  // Each block contributes its own text plus the separator it stands for.
  let total = 0;
  for (const block of Array.from(container.children)) {
    if (!block.contains(targetNode)) {
      total += codePointLength(block.textContent ?? "") + 1;
      continue;
    }
    return total + offsetWithin(block, targetNode, targetOffset);
  }

  return null;
}

/** Code-point offset of `targetNode` within the text of a single block. */
function offsetWithin(block: Element, targetNode: Node, targetOffset: number): number {
  const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
  let total = 0;
  let current = walker.nextNode();

  while (current) {
    if (current === targetNode) {
      return total + utf16OffsetToCodePoints(current.nodeValue ?? "", targetOffset);
    }
    total += codePointLength(current.nodeValue ?? "");
    current = walker.nextNode();
  }

  // A range endpoint may be the block element itself rather than a text node,
  // which places it before the block's own text.
  return total;
}

/** Convert a UTF-16 offset within `value` into a code-point offset. */
function utf16OffsetToCodePoints(value: string, utf16Offset: number): number {
  const clamped = Math.max(0, Math.min(utf16Offset, value.length));
  return codePointLength(value.slice(0, snappedToCharBoundary(value, clamped)));
}

/** Back up from an offset that may sit between the halves of a surrogate pair. */
function snappedToCharBoundary(value: string, utf16Offset: number): number {
  if (utf16Offset <= 0 || utf16Offset >= value.length) {
    return utf16Offset;
  }
  const code = value.charCodeAt(utf16Offset);
  // A low surrogate here means the offset split a pair, so move back to the
  // start of that character: half of one is not a position anyone can read from.
  const isLowSurrogate = code >= 0xdc00 && code <= 0xdfff;
  return isLowSurrogate ? utf16Offset - 1 : utf16Offset;
}

/** A span of chapter text, addressed in characters. */
export interface CharRange {
  start: number;
  end: number;
}

/**
 * The character span a DOM `Range` covers within the chapter text, or `null`
 * when the range is unusable: collapsed, empty of text, or reaching outside
 * `container`.
 *
 * A selection dragged right-to-left arrives with its anchor after its focus, so
 * the two ends are compared and swapped here rather than at every call site.
 * `end` is exclusive, so `end - start` is the number of characters selected.
 */
export function rangeToCharRange(container: HTMLElement, range: Range): CharRange | null {
  const start = pointToCharOffset(container, range.startContainer, range.startOffset);
  const end = pointToCharOffset(container, range.endContainer, range.endOffset);

  if (start === null || end === null) {
    return null;
  }

  const first = Math.min(start, end);
  const last = Math.max(start, end);
  if (first === last) {
    return null;
  }
  return { start: first, end: last };
}
