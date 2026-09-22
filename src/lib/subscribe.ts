// Event-subscription lifecycle, in one place.
//
// `listen()` is asynchronous, which is the whole problem: a component can
// unmount before the promise resolves, and the naive
//
//     const unlistens: (() => void)[] = []
//     setup().then(() => unlistens.forEach(fn => fn()))
//
// pattern then either leaks the listener (cleanup ran on an empty array) or,
// in React StrictMode where effects run twice, ends up with two live listeners
// for the same event. Duplicated listeners mean duplicated toasts, duplicated
// playback and duplicated state transitions.

import { listen } from "@tauri-apps/api/event";

export interface Subscription {
  /** Remove every listener, exactly once, whenever registration finished. */
  dispose(): void;
}

interface PendingRegistration {
  unlisten?: () => void;
  cancelled: boolean;
}

/**
 * Register several listeners as one unit.
 *
 * Every registration is tracked. `dispose()` is idempotent and safe to call at
 * any moment, including while registrations are still in flight: a listener
 * whose registration resolves after `dispose()` is removed immediately rather
 * than being left attached.
 */
export function subscribeAll(
  registrations: Array<() => Promise<() => void>>,
): Subscription {
  const pending: PendingRegistration[] = registrations.map(() => ({
    cancelled: false,
  }));
  let disposed = false;

  registrations.forEach((register, index) => {
    const entry = pending[index];
    void register()
      .then((unlisten) => {
        if (disposed || entry.cancelled) {
          unlisten();
          return;
        }
        entry.unlisten = unlisten;
      })
      .catch((error: unknown) => {
        // A failed registration must not take the window down, but it must not
        // be silent either: without this the UI would quietly stop updating.
        console.error("event listener registration failed:", error);
      });
  });

  return {
    dispose() {
      if (disposed) return;
      disposed = true;
      for (const entry of pending) {
        entry.cancelled = true;
        entry.unlisten?.();
        entry.unlisten = undefined;
      }
      pending.length = 0;
    },
  };
}

/** Convenience wrapper for a single listener. */
export function subscribe<T>(
  event: string,
  handler: (payload: T) => void,
): Subscription {
  return subscribeAll([
    () => listen<T>(event, (message) => handler(message.payload)),
  ]);
}
