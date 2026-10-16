"use client";

import { forwardRef, useImperativeHandle, useRef, useState } from "react";
import { ArrowUp } from "lucide-react";
import { ARTICLE_QUESTIONS } from "@/lib/actions";
import { openKeyMenu, startGloss } from "@/lib/ask";
import { docPayload, type ContextScope } from "@/lib/doc";
import { useAI } from "@/lib/desktop-ai";
import { DESKTOP } from "@/lib/platform";
import { useReader } from "./context";
import { WebSwitch } from "./WebSwitch";

export type ComposerHandle = { focus: () => void };

/**
 * Ask about the whole text. The answer is written in the margin beside the
 * paragraph you were reading when you asked, so it stays where your head was.
 */
export const Composer = forwardRef<ComposerHandle, { currentBlock: () => string | null; suggestions?: string[] }>(function Composer(
  { currentBlock, suggestions = ARTICLE_QUESTIONS },
  ref
) {
  const { ix } = useReader();
  const [q, setQ] = useState("");
  const [focused, setFocused] = useState(false);
  const [scope, setScope] = useState<ContextScope>("nearby");
  const { status } = useAI();
  const account = status.accounts.find((a) => a.id === status.activeAccountId);
  const ready = account?.connected && account.canUsePlan && status.selectedModel && status.models.some((m) => m.id === status.selectedModel);
  const at = currentBlock() ?? undefined;
  let disclosure = "";
  if (DESKTOP) {
    try {
      const doc = docPayload(ix, at, scope);
      disclosure = `${doc.partial ? "Partial document" : "Document"} · ¶${doc.paras[0]?.n ?? 0}–¶${doc.paras.at(-1)?.n ?? 0} will be sent to OpenAI with your question. 32,000-character limit.`;
    } catch (error) { disclosure = error instanceof Error ? error.message : "Cannot prepare context."; }
  }
  const inputRef = useRef<HTMLInputElement>(null);
  useImperativeHandle(ref, () => ({ focus: () => inputRef.current?.focus() }), []);

  const ask = (question: string) => {
    const text = question.trim();
    if (!text) return;
    if (DESKTOP && !ready) { setQ(text); openKeyMenu(); return; }
    startGloss(ix, { at: DESKTOP ? at : currentBlock() ?? undefined, action: "ask", question: text, scope: DESKTOP ? scope : undefined });
    setQ("");
    inputRef.current?.blur();
  };

  return (
    <div className="composer focus-hide pointer-events-none sticky bottom-4 z-30 mt-20 sm:bottom-6">
      {DESKTOP && <div className="pointer-events-auto mx-auto mb-2 max-w-[38rem] rounded-xl border border-rule bg-raised px-3 py-2 text-[11px] leading-snug text-ink-3 shadow-card">
        <label className="flex items-center gap-2">Send context
          <select aria-label="Document context to send" value={scope} onChange={(e) => setScope(e.target.value as ContextScope)} className="min-w-0 rounded-md bg-sunk px-2 py-1 text-ink">
            <option value="nearby">Nearby paragraphs</option>
            <option value="document">Current document (bounded)</option>
          </select>
        </label>
        <p className="mt-1" role="status">{disclosure}</p>
      </div>}
      {focused && !q && (
        <div className="pointer-events-auto mb-2 flex flex-wrap justify-center gap-1.5" style={{ animation: "rise 0.3s var(--ease-out) both" }}>
          {suggestions.map((s) => (
            <button
              key={s}
              type="button"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => ask(s)}
              className={`rounded-full border border-rule ${DESKTOP ? "bg-raised" : "bg-raised/95"} px-3 py-1.5 text-[12.5px] text-ink-2 shadow-card backdrop-blur transition-colors hover:border-rule-strong hover:text-ink`}
            >
              {s}
            </button>
          ))}
        </div>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          ask(q);
        }}
        className={`pointer-events-auto mx-auto flex max-w-[34rem] items-center gap-2 rounded-[16px] border ${DESKTOP ? "bg-raised" : "bg-raised/92"} py-1.5 pl-4 pr-1.5 shadow-pop backdrop-blur-md transition-[border-color,max-width] duration-300 ease-[var(--ease-out)] ${
          focused ? "max-w-[38rem] border-rule-strong" : "border-rule"
        }`}
      >
        <input
          ref={inputRef}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              inputRef.current?.blur();
            }
          }}
          placeholder={DESKTOP ? "Ask about this paper" : "Ask about the whole text"}
          aria-label={DESKTOP ? "Ask about this paper" : "Ask anything about the whole text"}
          className="h-9 min-w-0 flex-1 bg-transparent text-[14.5px] text-ink outline-none focus-visible:outline-none placeholder:text-ink-3"
        />
        {!focused && !q && <kbd className="hidden sm:inline">/</kbd>}
        {!DESKTOP && <WebSwitch />}
        <button
          type={DESKTOP && !ready ? "button" : "submit"}
          onClick={DESKTOP && !ready ? openKeyMenu : undefined}
          disabled={!q.trim() && !(DESKTOP && !ready)}
          aria-label={DESKTOP && !ready ? "Connect ChatGPT" : "Ask"}
          data-tip={DESKTOP && !ready ? "Connect ChatGPT" : "Ask"}
          data-tip-kbd={"\u21b5"}
          className="grid size-9 shrink-0 place-items-center rounded-[11px] bg-ink text-bg transition-opacity disabled:opacity-25"
        >
          {DESKTOP && !ready ? <span className="px-1 text-[10px]">AI</span> : <ArrowUp size={17} strokeWidth={2.3} />}
        </button>
      </form>
    </div>
  );
});
