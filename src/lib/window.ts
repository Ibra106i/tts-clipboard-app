// Which Tauri window is this webview rendering?

import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * The window label, or `"main"` when there is no Tauri window (tests, or the
 * bundle opened directly in a browser). The label is fixed for the lifetime of
 * the webview, so this is safe to call once per mount.
 */
export function resolveWindowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch (err) {
    console.warn("window label unavailable, assuming main:", err);
    return "main";
  }
}
