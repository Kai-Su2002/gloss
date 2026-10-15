import { invoke, isTauri } from "@tauri-apps/api/core";
import { DESKTOP } from "./platform";

export const hasNativeStorage = () => DESKTOP && isTauri();

// Keep state, document, and file writes ordered, including rapid edits before close.
let pending: Promise<unknown> = Promise.resolve();
export function desktopWrite(command: string, args: Record<string, unknown>): Promise<void> {
  const write = pending.catch(() => {}).then(() => invoke<void>(command, args));
  pending = write;
  return write;
}

export async function desktopRead<T>(command: string, args: Record<string, unknown>): Promise<T> {
  await pending.catch(() => {});
  return invoke<T>(command, args);
}

export function storageError(error: unknown) {
  window.dispatchEvent(new CustomEvent("gloss:storage-error", { detail: String(error) }));
}
