import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ElapsedClock } from "./ElapsedClock";

describe("ElapsedClock", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function advance(ms: number) {
    act(() => {
      vi.advanceTimersByTime(ms);
    });
  }

  it("does not run unless something is playing", () => {
    render(
      <ElapsedClock playing={false} resetKey="a" totalMs={60_000} />,
    );

    advance(5_000);

    // A stopped passage has no running time to report.
    expect(screen.getByText("0:00 / 1:00")).toBeInTheDocument();
  });

  it("advances while playing without its parent re-rendering", () => {
    render(<ElapsedClock playing resetKey="a" totalMs={60_000} />);

    advance(5_000);

    expect(screen.getByText("0:05 / 1:00")).toBeInTheDocument();
  });

  it("restarts for the next passage rather than carrying the last one's time", () => {
    const { rerender } = render(
      <ElapsedClock playing resetKey="a" totalMs={60_000} />,
    );
    advance(30_000);
    expect(screen.getByText("0:30 / 1:00")).toBeInTheDocument();

    // A different passage is a different passage, and starts at zero.
    rerender(<ElapsedClock playing resetKey="b" totalMs={60_000} />);
    expect(screen.getByText("0:00 / 1:00")).toBeInTheDocument();
  });

  it("resets when the passage stops", () => {
    const { rerender } = render(
      <ElapsedClock playing resetKey="a" totalMs={60_000} />,
    );
    advance(12_000);

    rerender(<ElapsedClock playing={false} resetKey="a" totalMs={60_000} />);
    expect(screen.getByText("0:00 / 1:00")).toBeInTheDocument();
  });
});
