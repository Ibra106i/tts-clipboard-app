import { describe, expect, it } from "vitest";
import {
  codePointLength,
  paragraphStarts,
  pointToCharOffset,
  rangeToCharRange,
} from "./offsets";

/** A chapter-shaped container: one element per `split("\n")` segment. */
function renderChapter(text: string): HTMLElement {
  const container = document.createElement("div");
  for (const paragraph of text.split("\n")) {
    const element = document.createElement("p");
    element.textContent = paragraph;
    container.appendChild(element);
  }
  return container;
}

/** The text node of one rendered paragraph, or a loud failure if it is missing. */
function blockText(container: HTMLElement, index: number): Node {
  const node = container.children[index]?.firstChild;
  if (!node) {
    throw new Error(`paragraph ${index} has no text node to address`);
  }
  return node;
}

describe("codePointLength", () => {
  it("counts code points rather than utf16 units", () => {
    // The whole reason this module exists: `.length` would answer 4 here.
    expect("a👍b".length).toBe(4);
    expect(codePointLength("a👍b")).toBe(3);
  });

  it("counts a regional indicator pair as two characters", () => {
    expect(codePointLength("🇹🇷")).toBe(2);
    expect("🇹🇷".length).toBe(4);
  });

  it("counts plain, accented and empty text", () => {
    expect(codePointLength("")).toBe(0);
    expect(codePointLength("plain ascii")).toBe(11);
    expect(codePointLength("héllo wörld")).toBe(11);
  });
});

describe("paragraphStarts", () => {
  it("reports the offset of every paragraph in order", () => {
    expect(paragraphStarts("one\ntwo\nthree")).toEqual([0, 4, 8]);
  });

  it("survives an emoji in an earlier paragraph", () => {
    // "a👍b" is three characters but four UTF-16 units, so `.length` arithmetic
    // would report the second paragraph one character too far along and the
    // backend would begin reading a word late.
    expect(paragraphStarts("a👍b\nsecond")).toEqual([0, 4]);
  });

  it("keeps blank lines from the parser's double newline as empty paragraphs", () => {
    // The parser separates blocks with a blank line and the reader splits on a
    // single "\n", so every block break produces an empty paragraph.
    expect(paragraphStarts("first\n\nsecond")).toEqual([0, 6, 7]);
    expect("first\n\nsecond".split("\n")).toHaveLength(3);
  });

  it("never exceeds the length of the text", () => {
    const text = "one\ntwo\nthree\n";
    for (const start of paragraphStarts(text)) {
      expect(start).toBeLessThanOrEqual(codePointLength(text));
    }
  });

  it("handles empty text", () => {
    expect(paragraphStarts("")).toEqual([0]);
  });
});

describe("paragraphStarts and pointToCharOffset", () => {
  it("agree on where every paragraph starts", () => {
    // The two functions find the same position by independent routes, so they
    // have to agree on every block of a chapter that mixes scripts, blank lines
    // and emoji. The empty blocks are the interesting ones: they are where a
    // missing paragraph separator would show up.
    const text = "First 👍 block.\n\nsecond block\n\n\nThird 🇹🇷 block.";
    const container = renderChapter(text);
    const starts = paragraphStarts(text);

    Array.from(container.children).forEach((_block, index) => {
      if (container.children[index]?.firstChild) {
        expect(pointToCharOffset(container, blockText(container, index), 0)).toBe(
          starts[index],
        );
      }
    });
  });
});

describe("pointToCharOffset", () => {
  it("maps a point at the start of a paragraph to that paragraph's offset", () => {
    const container = renderChapter("first paragraph\nsecond paragraph");
    expect(pointToCharOffset(container, blockText(container, 1), 0)).toBe(16);
  });

  it("maps a point part-way through a paragraph in code points", () => {
    const container = renderChapter("alpha beta");
    // "alpha " is six characters, and a range reports that as a UTF-16 offset
    // too — but only because no astral character precedes it here.
    expect(pointToCharOffset(container, blockText(container, 0), 6)).toBe(6);
  });

  it("counts an emoji once rather than twice when walking to a later node", () => {
    // Three characters plus the separator: the emoji must not be counted as two.
    const container = renderChapter("a👍b\nsecond");
    expect(pointToCharOffset(container, blockText(container, 1), 0)).toBe(4);
  });

  it("accumulates code points across several paragraphs", () => {
    // "one 👍" is 5 characters, "two" is 3, plus two separators.
    const container = renderChapter("one 👍\ntwo\nthree");
    expect(pointToCharOffset(container, blockText(container, 2), 0)).toBe(10);
  });

  it("returns null for a node outside the container", () => {
    const container = renderChapter("inside");
    const outside = document.createElement("p");
    outside.textContent = "elsewhere";
    document.body.appendChild(outside);

    const node = outside.firstChild;
    if (!node) {
      throw new Error("expected the outside paragraph to have text");
    }
    expect(pointToCharOffset(container, node, 0)).toBeNull();
    outside.remove();
  });

  it("ignores the empty paragraphs the parser leaves between blocks", () => {
    // Zero-length blocks must not add to the offset beyond the separator they
    // stand for, or every paragraph after a block break reads one word late.
    const container = renderChapter("first\n\nsecond");
    expect(pointToCharOffset(container, blockText(container, 2), 0)).toBe(7);
  });

  it("snaps an offset that splits a surrogate pair back to the character start", () => {
    const container = renderChapter("a👍b");
    // Offset 2 is the low surrogate of the emoji; the answer must be a real
    // character boundary, never a position inside the pair.
    expect(pointToCharOffset(container, blockText(container, 0), 2)).toBe(1);
  });
});

describe("rangeToCharRange", () => {
  function rangeOver(node: Node, start: number, end: number): Range {
    const range = document.createRange();
    range.setStart(node, start);
    range.setEnd(node, end);
    return range;
  }

  it("maps a selection within one paragraph to a character span", () => {
    const container = renderChapter("alpha beta");
    const span = rangeToCharRange(
      container,
      rangeOver(blockText(container, 0), 0, 5),
    );
    expect(span).toEqual({ start: 0, end: 5 });
  });

  it("counts an emoji once across a selection", () => {
    const container = renderChapter("a👍b");
    // UTF-16 1..4 is the emoji and the b; in characters that is 1..3.
    const span = rangeToCharRange(
      container,
      rangeOver(blockText(container, 0), 1, 4),
    );
    expect(span).toEqual({ start: 1, end: 3 });
  });

  it("accumulates across paragraphs for a selection spanning two of them", () => {
    const container = renderChapter("one\ntwo");
    const range = document.createRange();
    range.setStart(blockText(container, 0), 1);
    range.setEnd(blockText(container, 1), 2);
    const span = rangeToCharRange(container, range);
    expect(span).toEqual({ start: 1, end: 6 });
  });

  it("handles a selection dragged right to left", () => {
    // A user selecting backwards produces a selection whose anchor is after
    // its focus. The browser normalises the Range it reports, so the span must
    // still come out in reading order rather than inverted. The container has
    // to be in the document for the selection to register any ranges at all.
    const container = renderChapter("alpha beta");
    document.body.appendChild(container);
    const node = blockText(container, 0);
    const selection = window.getSelection();
    if (!selection) throw new Error("expected a selection");
    selection.setBaseAndExtent(node, 8, node, 2);

    const range = selection.getRangeAt(0);
    expect(rangeToCharRange(container, range)).toEqual({ start: 2, end: 8 });
    container.remove();
  });

  it("reports nothing for a collapsed selection", () => {
    const container = renderChapter("alpha beta");
    const range = document.createRange();
    range.setStart(blockText(container, 0), 3);
    range.collapse(true);
    expect(rangeToCharRange(container, range)).toBeNull();
  });

  it("reports nothing for a selection reaching outside the container", () => {
    const container = renderChapter("inside");
    const outside = document.createElement("p");
    outside.textContent = "elsewhere";
    document.body.appendChild(outside);
    const node = outside.firstChild;
    if (!node) throw new Error("expected the outside paragraph to have text");

    expect(rangeToCharRange(container, rangeOver(node, 0, 3))).toBeNull();
    outside.remove();
  });
});
