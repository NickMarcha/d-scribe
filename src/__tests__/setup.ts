import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup, configure } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";

// Autosave in the app waits up to 800 ms, so give async queries some room.
configure({ asyncUtilTimeout: 4000 });

// jsdom lacks these.
class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;
Element.prototype.scrollIntoView = function () {};
HTMLMediaElement.prototype.play = function () {
  return Promise.resolve();
};
HTMLMediaElement.prototype.pause = function () {};
HTMLMediaElement.prototype.load = function () {};
if (!navigator.clipboard) {
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText: () => Promise.resolve(), readText: () => Promise.resolve("") },
    configurable: true,
  });
}

afterEach(async () => {
  cleanup();
  // Unmounting unsubscribes from events asynchronously; let that finish before the IPC goes away.
  await new Promise((r) => setTimeout(r, 0));
  clearMocks();
  vi.restoreAllMocks();
});
