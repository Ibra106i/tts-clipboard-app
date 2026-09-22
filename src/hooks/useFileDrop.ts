// Native drag-and-drop for the main window.
//
// The previous implementation used the browser's HTML5 drop event and read
// `file.path`, which does not exist in a Tauri webview: the browser only ever
// exposes a File object, so the "path" was always `undefined` and imports were
// attempted with a bare file name. Tauri disables the HTML5 drop path by
// default for exactly this reason and provides `onDragDropEvent`, which
// delivers real filesystem paths from the OS.

import { useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { subscribeAll } from "../lib/subscribe";

/** The subset of Tauri's drag-drop payload this app cares about. */
type DragDropPayload =
  | { type: "enter"; paths: string[] }
  | { type: "over" }
  | { type: "drop"; paths: string[] }
  | { type: "leave" };

export interface UseFileDropResult {
  /** True while the OS reports files hovering over the window. */
  isDragging: boolean;
}

/**
 * Call `onPaths` with the real paths of dropped files.
 *
 * Registration is asynchronous, so it goes through `subscribeAll`: unmounting
 * while the registration is in flight cannot leave a listener attached.
 */
export function useFileDrop(onPaths: (paths: string[]) => void): UseFileDropResult {
  const [isDragging, setIsDragging] = useState(false);

  useEffect(() => {
    const subscription = subscribeAll([
      () =>
        getCurrentWebview().onDragDropEvent((event) => {
          const payload = event.payload as DragDropPayload;
          switch (payload.type) {
            case "enter":
            case "over":
              setIsDragging(true);
              break;
            case "leave":
              setIsDragging(false);
              break;
            case "drop":
              setIsDragging(false);
              if (payload.paths.length > 0) {
                onPaths(payload.paths);
              }
              break;
          }
        }),
    ]);

    return () => subscription.dispose();
  }, [onPaths]);

  return { isDragging };
}
