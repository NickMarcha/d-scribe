import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";

/** A long-running job shown in the bar at the bottom of the window. */
export interface Task {
  id: string;
  label: string;
  done: number;
  /** null until known: the bar shows as indeterminate. */
  total: number | null;
  unit: "items" | "bytes";
}

/** Maps a backend progress event to an update of the task. Return null to ignore the event. */
export interface TaskListener {
  event: string;
  map: (payload: any) => Partial<Omit<Task, "id">> | null;
}

let tasks: Task[] = [];
const subscribers = new Set<() => void>();

function emit() {
  subscribers.forEach((fn) => fn());
}

function update(id: string, changes: Partial<Task>) {
  tasks = tasks.map((t) => (t.id === id ? { ...t, ...changes } : t));
  emit();
}

/**
 * Runs `fn` as a task with a progress bar. State lives outside React, so progress keeps
 * showing when the component that started the task is unmounted (e.g. leaving Settings).
 */
export async function runTask<T>(
  id: string,
  label: string,
  listeners: TaskListener[],
  fn: () => Promise<T>,
): Promise<T> {
  tasks = [...tasks.filter((t) => t.id !== id), { id, label, done: 0, total: null, unit: "items" }];
  emit();
  const unlisteners = listeners.map((l) =>
    listen(l.event, (e) => {
      const changes = l.map(e.payload);
      if (changes) update(id, changes);
    }),
  );
  try {
    return await fn();
  } finally {
    unlisteners.forEach((u) => u.then((off) => off()));
    tasks = tasks.filter((t) => t.id !== id);
    emit();
  }
}

/** Progress events carrying `bytesDownloaded` / `totalBytes`. */
export function bytesProgress(payload: { bytesDownloaded: number; totalBytes: number | null }) {
  return { done: payload.bytesDownloaded, total: payload.totalBytes ?? null, unit: "bytes" as const };
}

function subscribe(fn: () => void) {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
}

export function useTasks(): Task[] {
  return useSyncExternalStore(subscribe, () => tasks);
}
