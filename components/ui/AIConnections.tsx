"use client";

import { useEffect } from "react";
import { Sparkles } from "lucide-react";
import { cancelLoginAI, disconnectAI, loginAI, refreshAI, selectAI, useAI } from "@/lib/desktop-ai";
import { hasNativeStorage } from "@/lib/desktop-storage";
import { Popover, barBtn } from "./Popover";

const control = "w-full rounded-lg border border-rule bg-bg px-2.5 py-2 text-[12.5px] text-ink disabled:opacity-50";
const button = "rounded-lg border border-rule px-2.5 py-1.5 text-[12px] text-ink-2 transition-colors hover:bg-sunk hover:text-ink disabled:opacity-50 disabled:pointer-events-none";
// Operations report errors to the shared store, never to unmounted panel state.
const run = (promise: Promise<void>) => { void promise.catch(() => {}); };

function ConnectionsPanel() {
  const { status, busy, error, warning, success } = useAI();
  const native = hasNativeStorage();
  const account = status.accounts.find((a) => a.id === status.activeAccountId);
  const ready = account?.connected && account.canUsePlan;

  useEffect(() => {
    if (hasNativeStorage()) run(refreshAI());
  }, []);

  return (
    <div className="max-h-[calc(100dvh-140px)] space-y-3.5 overflow-y-auto overscroll-contain bg-raised text-[12.5px] leading-snug">
      <div>
        <h2 className="font-medium text-ink">AI connections</h2>
        <div className="mt-2 flex items-center justify-between rounded-lg bg-sunk px-2.5 py-2">
          <span className="font-medium text-ink">ChatGPT subscription</span>
          <span className="text-[11px] text-rubric-text">Active provider</span>
        </div>
        <div className="mt-2 flex justify-between px-2.5 text-[11.5px] text-ink-3">
          <span>API Key · Coming later</span>
          <span>Local model · Coming later</span>
        </div>
      </div>

      {!native ? (
        <p className="rounded-lg border border-rule p-2.5 text-ink-3">Native AI is unavailable in browser preview. Open the Gloss desktop app to connect ChatGPT. No accounts or models are simulated here.</p>
      ) : (
        <>
          {status.accounts.length > 0 && (
            <label className="block space-y-1.5 text-ink-3">
              <span>ChatGPT account</span>
              <select className={control} value={status.activeAccountId ?? ""} disabled={!!busy} onChange={(e) => run(selectAI(e.target.value))}>
                <option value="" disabled>Select an account</option>
                {status.accounts.map((a) => <option key={a.id} value={a.id}>{a.label}{!a.connected ? " · Reauthorize" : !a.canUsePlan ? " · Plan unavailable" : ""}</option>)}
              </select>
            </label>
          )}
          {account && (
            <p className="text-[11.5px] text-ink-3">{!account.connected ? "Saved registration. Reauthorize to reconnect." : !account.canUsePlan ? "Connected, but this account cannot currently use its ChatGPT plan in Gloss." : "Connected with your ChatGPT plan. Availability and usage limits are set by ChatGPT."}</p>
          )}
          <label className="block space-y-1.5 text-ink-3">
            <span>Model</span>
            <select className={control} value={status.selectedModel ?? ""} disabled={!!busy || !ready || !status.models.length} onChange={(e) => account && run(selectAI(account.id, e.target.value))}>
              <option value="" disabled>{!ready ? "Connect an eligible account first" : !status.models.length ? "No models loaded" : "Select a model"}</option>
              {status.models.map((model) => <option key={model.id} value={model.id}>{model.label}</option>)}
            </select>
          </label>
          <div className="flex flex-wrap gap-2">
            <button type="button" className={button} disabled={!!busy} onClick={() => run(loginAI())}>Continue with ChatGPT</button>
            {account && <button type="button" className={button} disabled={!!busy} onClick={() => run(loginAI(account.id))}>Reauthorize saved account</button>}
            <button type="button" className={button} disabled={!!busy} onClick={() => run(refreshAI())}>{ready ? "Refresh models" : "Refresh status"}</button>
            {account && <button type="button" className={button} disabled={!!busy} onClick={() => run(disconnectAI(account.id))}>Disconnect</button>}
            {busy === "login" && <button type="button" className={button} onClick={() => run(cancelLoginAI())}>Cancel pending login</button>}
          </div>
          <div aria-live="polite" className="space-y-2 text-[11.5px] text-ink-3">
            {busy && <p role="status">{busy === "login" ? "Complete authorization in your system browser. You can close this panel without canceling login." : busy === "cancel" ? "Canceling browser login…" : busy === "refresh" ? "Refreshing connections and models…" : busy === "disconnect" ? "Disconnecting account…" : "Updating selection…"}</p>}
            {success && <p role="status">{success}</p>}
            {error && <p role="alert" className="text-rubric-text">{error}</p>}
            {warning && <p role="alert" className="text-rubric-text">{warning}</p>}
          </div>
        </>
      )}

      <div className="space-y-2 border-t border-rule pt-3 text-[11.5px] text-ink-3">
        <p>When you ask, your prompt and relevant document text are sent to OpenAI’s cloud through your ChatGPT account. Connecting alone does not send documents.</p>
        <p>No automatic paid API fallback. Gloss will stop if your plan or model is unavailable.</p>
        <p>Check official ChatGPT usage and limits in your ChatGPT account in your system browser. This version cannot open the usage page for you.</p>
      </div>
    </div>
  );
}

export function AIConnections() {
  return (
    <Popover label="AI connections" tip="AI connections" openOn="gloss:ai" width={320} triggerClass={barBtn} trigger={<><Sparkles size={14} /><span>AI</span></>}>
      {() => <ConnectionsPanel />}
    </Popover>
  );
}
