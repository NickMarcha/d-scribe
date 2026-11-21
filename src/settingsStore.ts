import { useEffect, useState } from "react";
import { load, type Store } from "@tauri-apps/plugin-store";

/**
 * settings.json, shared by every screen. Write with `setSetting` so screens that read a key
 * with `useSetting` see the change right away (Session stays mounted while Settings edits).
 */
let store: Promise<Store> | null = null;
const subscribers = new Map<string, Set<(value: unknown) => void>>();

export function settingsStore(): Promise<Store> {
  return (store ??= load("settings.json", { defaults: {}, autoSave: true }));
}

export async function getSetting<T>(key: string): Promise<T | undefined> {
  return (await settingsStore()).get<T>(key);
}

/** Saves a setting and passes the new value to every `useSetting(key)`. */
export async function setSetting(key: string, value: unknown) {
  const s = await settingsStore();
  await s.set(key, value);
  await s.save();
  subscribers.get(key)?.forEach((fn) => fn(value));
}

/** The current value of a setting, kept up to date. Undefined until loaded or when unset. */
export function useSetting<T>(key: string): T | undefined {
  const [value, setValue] = useState<T | undefined>(undefined);
  useEffect(() => {
    let changed = false;
    const onChange = (v: unknown) => {
      changed = true;
      setValue(v as T);
    };
    if (!subscribers.has(key)) subscribers.set(key, new Set());
    subscribers.get(key)!.add(onChange);
    getSetting<T>(key).then((v) => {
      if (!changed) setValue(v);
    });
    return () => {
      subscribers.get(key)!.delete(onChange);
    };
  }, [key]);
  return value;
}
