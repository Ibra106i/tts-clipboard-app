import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useFileDrop } from "./useFileDrop";
import { harness } from "../test/mocks/harness";

describe("useFileDrop", () => {
  it("passes real filesystem paths to the callback on drop", async () => {
    const onPaths = vi.fn();
    const { result } = renderHook(() => useFileDrop(onPaths));
    await waitFor(() => expect(harness.dragDropListenerCount).toBe(1));

    act(() => {
      harness.emitDragDrop({
        type: "drop",
        paths: ["C:\\Books\\novel.epub", "C:\\Books\\paper.pdf"],
      });
    });

    expect(onPaths).toHaveBeenCalledWith([
      "C:\\Books\\novel.epub",
      "C:\\Books\\paper.pdf",
    ]);
    expect(result.current.isDragging).toBe(false);
  });

  it("tracks the drag state across enter, over and leave", async () => {
    const { result } = renderHook(() => useFileDrop(() => {}));
    await waitFor(() => expect(harness.dragDropListenerCount).toBe(1));

    act(() => {
      harness.emitDragDrop({ type: "enter", paths: ["C:\\a.pdf"] });
    });
    expect(result.current.isDragging).toBe(true);

    act(() => {
      harness.emitDragDrop({ type: "leave" });
    });
    expect(result.current.isDragging).toBe(false);
  });

  it("ignores a drop with no files instead of importing nothing", async () => {
    const onPaths = vi.fn();
    renderHook(() => useFileDrop(onPaths));
    await waitFor(() => expect(harness.dragDropListenerCount).toBe(1));

    act(() => {
      harness.emitDragDrop({ type: "drop", paths: [] });
    });

    expect(onPaths).not.toHaveBeenCalled();
  });

  it("detaches the OS listener on unmount", async () => {
    const { unmount } = renderHook(() => useFileDrop(() => {}));
    await waitFor(() => expect(harness.dragDropListenerCount).toBe(1));

    unmount();
    expect(harness.dragDropListenerCount).toBe(0);
  });
});
