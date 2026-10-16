import { useEffect, useState, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Library } from "@/components/library/Library";
import { Reader } from "@/components/reader/Reader";
import { Tooltips } from "@/components/ui/Tooltips";
import { desktopRead, hasNativeStorage, storageError } from "@/lib/desktop-storage";
import { sampleById } from "@/lib/samples";
import { applySettings, SETTINGS_KEY } from "@/lib/settings";
import { flushStore, useStore } from "@/lib/store";
import { refreshAI } from "@/lib/desktop-ai";
import "@/app/globals.css";
import "@/app/library/library.css";
import "@fontsource-variable/newsreader/opsz.css";
import "@fontsource-variable/newsreader/opsz-italic.css";
import "@fontsource-variable/schibsted-grotesk";
import "@fontsource-variable/literata";
import "@fontsource-variable/atkinson-hyperlegible-next";
import "./style.css";

function subscribeRoute(listener: () => void) {
  window.addEventListener("hashchange", listener);
  return () => window.removeEventListener("hashchange", listener);
}

function App() {
  const route = useSyncExternalStore(subscribeRoute, () => window.location.hash.slice(1));
  const [error, setError] = useState("");
  useEffect(() => {
    const onError = (event: Event) => setError((event as CustomEvent<string>).detail);
    window.addEventListener("gloss:storage-error", onError);
    if (hasNativeStorage()) void refreshAI().catch(() => {});
    return () => window.removeEventListener("gloss:storage-error", onError);
  }, []);
  const id = route.startsWith("/read/") ? decodeURIComponent(route.slice(6)) : null;
  return <>
    {!hasNativeStorage() && <div className="desktop-preview" role="status">Desktop UI preview · browser storage only. The installed app uses local SQLite.</div>}
    {error && <div role="alert" className="fixed bottom-4 left-4 right-4 z-[100] rounded-xl border border-rule-strong bg-raised p-4 text-[13px] text-rubric-text shadow-pop">Could not save your library: {error}. Keep this window open and free disk space, then try again.</div>}
    {id ? <Reader key={id} id={id} initial={sampleById(id)} /> : <Library />}
    <Tooltips />
  </>;
}

async function start() {
  if (hasNativeStorage()) {
    // SQLite is authoritative. localStorage is only a synchronous UI cache.
    for (const key of ["gloss:v1", SETTINGS_KEY]) {
      const value = await desktopRead<string | null>("load_state", { key });
      if (value === null) localStorage.removeItem(key);
      else {
        JSON.parse(value); // Do not overwrite corrupt native state with an empty library.
        localStorage.setItem(key, value);
      }
    }
    const window = getCurrentWindow();
    let closing = false;
    await window.onCloseRequested(async (event) => {
      event.preventDefault();
      if (closing) return;
      if (!globalThis.window.dispatchEvent(new Event("gloss:before-close", { cancelable: true }))) return;
      closing = true;
      try {
        await flushStore();
        await window.destroy();
      } catch (error) {
        closing = false;
        storageError(error);
      }
    });
  }
  useStore.getState().hydrate();
  applySettings(useStore.getState().settings);
  createRoot(document.getElementById("root")!).render(<App />);
}

void start().catch((error) => {
  const root = document.getElementById("root")!;
  root.textContent = `Gloss could not open its local library. Your database has not been reset. ${String(error)}`;
  root.setAttribute("role", "alert");
});
