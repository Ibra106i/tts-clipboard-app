import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Icon } from "./Icon";

// A reader paragraph's character offset is resolved by walking the block's text
// nodes, so any text character inside one shifts every offset after it. This
// test exists because a glyph icon (`▶`) was once used there and silently
// corrupted the mapping - the offsets stayed plausible and pointed at the wrong
// words. It guards the whole family, not just one control.
describe("Icon", () => {
  const NAMES = [
    "library",
    "import",
    "book",
    "back",
    "speaker",
    "play",
    "pause",
    "close",
  ] as const;

  it("contributes no text content", () => {
    for (const name of NAMES) {
      const { container } = render(<Icon name={name} />);
      // `textContent` is what the offset mapping counts. An SVG has a `<path>`,
      // not a text node, so this must be empty.
      expect(container.textContent, `icon "${name}" added text`).toBe("");
    }
  });

  it("renders as SVG rather than a glyph character", () => {
    for (const name of NAMES) {
      const { container } = render(<Icon name={name} />);
      expect(container.querySelector("svg")).not.toBeNull();
      expect(container.querySelector("path")).not.toBeNull();
    }
  });

  it("hides a decorative icon from assistive technology", () => {
    const { container } = render(<Icon name="play" />);
    // Asserted on the attribute rather than a role: `aria-hidden` is what
    // actually removes it from the accessibility tree, and a role lookup does
    // not prove that.
    expect(container.querySelector("svg")?.getAttribute("aria-hidden")).toBe(
      "true",
    );
  });

  it("names an icon that is the only content of a control", () => {
    render(<Icon name="close" title="Close overlay" />);
    expect(
      screen.getByRole("img", { name: "Close overlay" }),
    ).toBeInTheDocument();
  });

  it("inherits the surrounding colour rather than carrying one", () => {
    const { container } = render(<Icon name="play" />);
    const svg = container.querySelector("svg");
    // `currentColor` is what lets one icon work on an amber fill and a dark
    // surface without a second asset.
    expect(svg?.getAttribute("stroke")).toBe("currentColor");
    expect(svg?.getAttribute("fill")).toBe("none");
  });
});