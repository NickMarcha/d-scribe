// Drives the real <App /> through what a user sees: roles and visible text only.

import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, vi } from "vitest";

export type User = ReturnType<typeof userEvent.setup>;

/** Renders a fresh copy of the app. Modules are reset so module-level state (tasks) starts clean. */
export async function renderApp() {
  vi.resetModules();
  const { default: App } = await import("../App");
  const user = userEvent.setup();
  render(<App />);
  // The recordings list (or its empty state) shows once the first backend calls are done.
  await screen.findByRole("heading", { name: "Recordings" });
  return user;
}

/** Real wait, for asserting that something does NOT happen (e.g. past the 800 ms autosave). */
export async function settle(ms = 1200) {
  // React flushes work queued inside act() only when act() returns, so wait in short steps.
  const end = Date.now() + ms;
  while (Date.now() < end) {
    await act(() => new Promise((r) => setTimeout(r, 25)));
  }
}

export async function goToTab(user: User, tab: "Session" | "Settings") {
  await user.click(screen.getByRole("button", { name: tab }));
}

/** From the recordings list, opens the recording whose row mentions `place` (e.g. "Alpha server"). */
export async function openRecording(user: User, place: RegExp | string) {
  const name = typeof place === "string" ? new RegExp(place) : place;
  await user.click(await screen.findByRole("button", { name }));
  await screen.findByRole("button", { name: "Recordings" });
}

/** From an open recording, goes back to the list. */
export async function backToList(user: User) {
  await user.click(screen.getByRole("button", { name: "Recordings" }));
  await screen.findByRole("heading", { name: "Recordings" });
}

export async function clickTranscribe(user: User) {
  const button = await screen.findByRole("button", { name: "Transcribe" });
  await waitFor(() => expect(button).toBeEnabled());
  await user.click(button);
}

export async function showTranscriptTab(user: User, tab: "Read" | "Edit" | "Stats" | "AI") {
  await user.click(screen.getByRole("button", { name: tab }));
}

/**
 * Expands a Settings section if it isn't open. `probe` is text only visible when it is open.
 */
export async function openSettingsSection(user: User, title: string, probe: string | RegExp) {
  if (screen.queryByText(probe)) return;
  await user.click(await screen.findByRole("button", { name: new RegExp(title) }));
  await screen.findByText(probe);
}

export async function openManageModels(user: User) {
  await openSettingsSection(user, "Manage models", "Remote sources");
}

export async function openSelectedModels(user: User) {
  await openSettingsSection(user, "Selected models", /Pick a model for each language/);
}

/**
 * The smallest element around the model name `name` that also shows its install state
 * ("Installed", "Not installed", ...) in Settings > Manage models.
 */
export function modelRow(name: string): HTMLElement {
  const candidates = screen.getAllByText(name, { exact: true });
  for (const c of candidates) {
    let el: HTMLElement | null = c;
    while (el && !within(el).queryByText(/^(Installed|Not installed|Available)$/)) el = el.parentElement;
    if (el && el !== document.body) return el;
  }
  throw new Error(`No model row for ${name}`);
}

/** Clicks `button` if the user could (it exists and is enabled). Returns whether it was clicked. */
export async function clickIfPossible(user: User, button: HTMLElement | null | undefined) {
  if (!button || (button as HTMLButtonElement).disabled) return false;
  await user.click(button);
  return true;
}

export { screen, waitFor, within, act };
