import { describe, expect, it } from "vitest";
import { subscribe, subscribeAll } from "./subscribe";
import { harness } from "../test/mocks/harness";

describe("subscribeAll", () => {
  it("registers each listener once and removes it on dispose", async () => {
    const subscription = subscribeAll([
      () => harness.listen("a", () => {}),
      () => harness.listen("b", () => {}),
    ]);

    await Promise.resolve();
    await Promise.resolve();
    expect(harness.listenerCount("a")).toBe(1);
    expect(harness.listenerCount("b")).toBe(1);
    expect(harness.totalListenerCount()).toBe(2);

    subscription.dispose();
    expect(harness.totalListenerCount()).toBe(0);

    // Idempotent: a second dispose must not throw or double-remove.
    subscription.dispose();
    expect(harness.totalListenerCount()).toBe(0);
  });

  it("removes a listener whose registration resolves after dispose", async () => {
    // The StrictMode unmount-before-resolve race: dispose runs first, the
    // promise settles afterwards and must clean itself up.
    let resolve: ((unlisten: () => void) => void) | undefined;
    const slow = new Promise<() => void>((r) => {
      resolve = r;
    });

    const subscription = subscribeAll([() => slow]);
    subscription.dispose();

    expect(harness.totalListenerCount()).toBe(0);
    resolve?.((() => {}) as () => void);
    await slow;

    // The listener was registered after cleanup and was therefore torn down
    // immediately; nothing is left attached.
    expect(harness.totalListenerCount()).toBe(0);
  });

  it("does not throw or leave listeners when one registration fails", async () => {
    const subscription = subscribeAll([
      () => Promise.reject(new Error("bridge unavailable")),
      () => harness.listen("ok", () => {}),
    ]);

    await Promise.resolve();
    await Promise.resolve();
    expect(harness.listenerCount("ok")).toBe(1);

    subscription.dispose();
    expect(harness.totalListenerCount()).toBe(0);
  });

  it("delivers payloads to the handler", async () => {
    const seen: number[] = [];
    const subscription = subscribe<number>("progress", (value) => {
      seen.push(value);
    });

    await Promise.resolve();
    await Promise.resolve();
    harness.emit("progress", 42);

    expect(seen).toEqual([42]);
    subscription.dispose();
  });

  it("removes and does not re-add under a double-mount", async () => {
    // Emulates StrictMode: mount, unmount, mount again.
    const first = subscribeAll([() => harness.listen("x", () => {})]);
    await Promise.resolve();
    await Promise.resolve();
    first.dispose();

    const second = subscribeAll([() => harness.listen("x", () => {})]);
    await Promise.resolve();
    await Promise.resolve();

    expect(harness.listenerCount("x")).toBe(1);
    second.dispose();
  });
});
