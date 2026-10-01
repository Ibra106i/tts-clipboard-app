// Inline SVG icons.
//
// These are SVG rather than emoji or glyph characters for three reasons:
//
// 1. Emoji render differently per platform and cannot be styled with the design
//    tokens, so a button that is amber on Windows is blue on macOS.
// 2. A glyph character inside a reader paragraph would be a text node, and the
//    offset mapping walks text nodes — one stray character shifts every offset
//    after it. Every icon here is used outside paragraph content, but an SVG
//    removes the whole class of mistake rather than relying on remembering.
// 3. They inherit `currentColor`, so an icon follows the label it sits beside.
//
// All of them are drawn on a 16x16 grid with a 1.5px stroke, so they read as
// one family rather than a collection.

export type IconName =
  | "library"
  | "import"
  | "book"
  | "back"
  | "speaker"
  | "play"
  | "pause"
  | "close";

interface IconProps {
  name: IconName;
  /** Rendered size in pixels. Defaults to 16, the grid size. */
  size?: number;
  /**
   * Set when the icon is the only content of a control. A decorative icon
   * beside a text label is hidden from assistive technology instead, because
   * announcing "speaker" before "Read this" adds nothing.
   */
  title?: string;
  className?: string;
}

const PATHS: Record<IconName, string> = {
  // Two books, one leaning — a library rather than a single volume.
  library: "M2 3h4v10H2zM7 3l4-1 1 10-4 1zM13.5 4.5l.6-.1",
  // A box with an arrow going into it.
  import: "M8 1.5v7M5 5.5L8 8.5l3-3M2 10.5v2a1 1 0 001 1h10a1 1 0 001-1v-2",
  // An open book.
  book: "M2 3h4.5c1 0 1.5.5 1.5 1.5V13c0-1-.5-1.5-1.5-1.5H2zM14 3H9.5C8.5 3 8 3.5 8 4.5V13c0-1 .5-1.5 1.5-1.5H14z",
  back: "M10 3L5 8l5 5",
  speaker: "M3 6.5h2.5L9 3.5v9L5.5 9.5H3zM11.5 6a3 3 0 010 4M13.5 4.5a5.5 5.5 0 010 7",
  play: "M5 3.2l7 4.8-7 4.8z",
  // Two bars with a gap.
  pause: "M5.5 3.5v9M10.5 3.5v9",
  close: "M4 4l8 8M12 4l-8 8",
};

export function Icon({ name, size = 16, title, className }: IconProps) {
  const decorative = title === undefined;
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      // A decorative icon is hidden from assistive technology. A named one is
      // exposed as an image with that name.
      aria-hidden={decorative ? true : undefined}
      role={decorative ? undefined : "img"}
      focusable="false"
    >
      {title !== undefined ? <title>{title}</title> : null}
      <path d={PATHS[name]} fill={name === "play" ? "currentColor" : "none"} />
    </svg>
  );
}