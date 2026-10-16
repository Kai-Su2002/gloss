"use client";

import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { FileUp, Loader2, X } from "lucide-react";
import { importFromFile, keep } from "@/lib/import";
import { flushStore } from "@/lib/store";

export function LocalImport({ onClose }: { onClose: () => void }) {
  const input = useRef<HTMLInputElement>(null);
  const running = useRef(false);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<string[]>([]);
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !running.current) onClose();
    };
    const beforeClose = (event: Event) => {
      if (running.current) event.preventDefault();
    };
    window.addEventListener("keydown", key);
    window.addEventListener("gloss:before-close", beforeClose);
    input.current?.focus();
    return () => {
      document.body.style.overflow = previous;
      window.removeEventListener("keydown", key);
      window.removeEventListener("gloss:before-close", beforeClose);
    };
  }, [onClose]);

  async function add(files: File[]) {
    if (running.current || !files.length) return;
    running.current = true;
    setBusy(true);
    setErrors([]);
    let saved = 0;
    try {
      for (const [index, file] of files.entries()) {
        const label = `${index + 1} of ${files.length} · ${file.name}`;
        setStatus(label);
        const result = await importFromFile(file, (_progress, detail) => setStatus(`${label} · ${detail ?? "Reading"}`));
        const error = result.ok ? await keep(result.article, result.file) : result.error;
        if (error) setErrors((items) => [...items, `${file.name}: ${error}`]);
        else saved++;
      }
      await flushStore();
      setStatus(`${saved} ${saved === 1 ? "paper" : "papers"} saved to your local library.`);
    } catch (error) {
      setErrors((items) => [...items, String(error)]);
    } finally {
      running.current = false;
      setBusy(false);
    }
  }

  return createPortal(
    <div className="fixed inset-0 z-[80] grid place-items-center p-6">
      <div className="absolute inset-0 bg-ink/20 backdrop-blur-[2px]" onClick={() => !busy && onClose()} />
      <section role="dialog" aria-modal="true" aria-labelledby="import-title" className="relative w-full max-w-[560px] rounded-[22px] border border-rule bg-raised p-8 shadow-pop">
        <button type="button" aria-label="Close import" disabled={busy} onClick={onClose} className="absolute right-4 top-4 grid size-9 place-items-center rounded-full text-ink-3 hover:bg-sunk disabled:opacity-40"><X size={18} /></button>
        <p className="mb-3 text-[12px] font-medium uppercase tracking-[0.13em] text-rubric-text">A library of your own</p>
        <h2 id="import-title" className="font-serif text-[34px] leading-tight text-ink">Bring your papers.</h2>
        <p className="mt-3 text-[14px] leading-relaxed text-ink-3">Import PDFs, Markdown or text. Originals, highlights and notes stay on this Mac. No account required.</p>
        <div onDragOver={(event) => { event.preventDefault(); setDragging(true); }} onDragLeave={() => setDragging(false)} onDrop={(event) => {
          event.preventDefault(); setDragging(false); void add(Array.from(event.dataTransfer.files));
        }} className={`mt-6 rounded-[16px] border border-dashed p-8 text-center ${dragging ? "border-ink bg-sunk" : "border-rule-strong"}`}>
          {busy ? <Loader2 className="mx-auto animate-spin text-ink-3" size={28} /> : <FileUp className="mx-auto text-ink-3" size={28} />}
          <p className="mt-3 text-[14px] text-ink-2">{busy ? "Reading your papers…" : "Drop files here, or choose from your Mac"}</p>
          <input ref={input} type="file" multiple accept=".pdf,.txt,.md,.markdown" aria-label="Choose papers" disabled={busy} className="mt-4 block w-full text-[12px] text-ink-3 file:mr-3 file:rounded-full file:border-0 file:bg-ink file:px-4 file:py-2 file:text-bg" onChange={(event) => {
            void add(Array.from(event.target.files ?? [])); event.target.value = "";
          }} />
        </div>
        <p role="status" className="mt-4 break-words text-[13px] text-ink-2">{status}</p>
        {errors.length > 0 && <ul role="alert" className="mt-3 max-h-40 space-y-2 overflow-auto text-[13px] text-rubric-text">{errors.map((error, index) => <li key={index}>{error}</li>)}</ul>}
        <p className="mt-4 text-[12px] leading-relaxed text-ink-3">Text PDFs up to 400 pages. Scanned PDFs need OCR before import. Optional ChatGPT Q&amp;A sends chosen text to OpenAI only when you ask. Cloud sync is not included.</p>
      </section>
    </div>, document.body,
  );
}
