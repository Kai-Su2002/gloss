"use client";

import { useSyncExternalStore } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { hasNativeStorage } from "./desktop-storage";

export type AIAccount = { id: string; label: string; connected: boolean; canUsePlan: boolean };
export type AIModel = { id: string; label: string };
export type AIStatus = { accounts: AIAccount[]; activeAccountId: string | null; selectedModel: string | null; models: AIModel[] };
export type AIRequest = {
  requestId: string;
  accountId: string;
  model: string;
  instructions: string;
  messages: { role: "user" | "assistant"; content: string }[];
};
export type AIDelta = { text: string };
export type AIBusy = "refresh" | "login" | "cancel" | "select" | "disconnect" | null;
export type AISnapshot = { status: AIStatus; busy: AIBusy; error: string | null; warning: string | null; success: string | null };

const empty: AIStatus = { accounts: [], activeAccountId: null, selectedModel: null, models: [] };
const initial: AISnapshot = { status: empty, busy: null, error: null, warning: null, success: null };
let snapshot = initial;
const listeners = new Set<() => void>();
function publish(patch: Partial<AISnapshot>) {
  snapshot = { ...snapshot, ...patch };
  listeners.forEach((listener) => listener());
}

// Only explicitly whitelisted metadata lives here. Never retain IPC credentials.
function accept(value: AIStatus) {
  const status: AIStatus = {
    accounts: value.accounts.map(({ id, label, connected, canUsePlan }) => ({ id, label, connected, canUsePlan })),
    activeAccountId: value.activeAccountId,
    selectedModel: value.selectedModel,
    models: value.models.map(({ id, label }) => ({ id, label })),
  };
  const changed = JSON.stringify(status) !== JSON.stringify(snapshot.status);
  publish({ status });
  if (changed && typeof window !== "undefined") window.dispatchEvent(new Event("gloss:ai-updated"));
}

export function getAIStatus(): AIStatus { return snapshot.status; }
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function useAI(): AISnapshot {
  return useSyncExternalStore(subscribe, () => snapshot, () => initial);
}

function native() {
  if (!hasNativeStorage()) throw new Error("AI connections require the Gloss desktop app. Browser preview cannot connect accounts.");
}

// Never display provider bodies or arbitrary IPC errors; these native messages
// are fixed strings, with actionable recovery and no credential material.
const safeErrors = new Set([
  "OS credential service unavailable. Unlock or enable your system keyring and try again.",
  "Cannot open the system browser",
  "Cannot bind sign-in callback",
  "Sign in timed out",
  "ChatGPT authorization was declined or failed",
  "ChatGPT session exchange failed. Please sign in again.",
  "ChatGPT session renewal unavailable",
  "ChatGPT plan use was not authorized. Reconnect and grant plan sharing permission.",
  "ChatGPT plan sharing permission is no longer granted. Reconnect your account.",
  "ChatGPT plan sharing limit reached. Review usage in ChatGPT Settings and try again later.",
  "ChatGPT plan sharing is unavailable. Review this app's access in ChatGPT Settings.",
  "ChatGPT request could not connect",
  "Cannot load ChatGPT models. Reconnect your account or try again.",
]);
function safeError(error: unknown, fallback: string): string {
  const message = error instanceof Error ? error.message : error;
  return typeof message === "string" && safeErrors.has(message) ? message : fallback;
}

// One metadata mutation at a time; cancellation is the only interrupt. Errors are
// deliberately generic: arbitrary native error strings could contain auth material.
async function operation(busy: Exclude<AIBusy, null>, failure: string, run: () => Promise<void>) {
  native();
  if (snapshot.busy) throw new Error("Another AI connection operation is in progress.");
  publish({ busy, error: null, success: null });
  try { await run(); }
  catch (cause) { const error = safeError(cause, failure); publish({ error }); throw new Error(error); }
  finally { publish({ busy: null }); }
}

async function loadModels(current: () => boolean = () => true) {
  const account = snapshot.status.accounts.find((a) => a.id === snapshot.status.activeAccountId);
  if (!account?.connected || !account.canUsePlan) return;
  // A failed refresh must not leave stale models looking ready for use.
  accept({ ...snapshot.status, models: [], selectedModel: null });
  const status = await invoke<AIStatus>("ai_models");
  if (current()) accept(status);
}

export async function refreshAI(): Promise<void> {
  if (snapshot.busy) return;
  await operation("refresh", "Could not refresh AI connections or available models. Try again before asking.", async () => {
    accept({ ...snapshot.status, models: [], selectedModel: null });
    accept(await invoke<AIStatus>("ai_status"));
    await loadModels();
  });
}

let loginGeneration = 0;
let pendingLogin: Promise<void> | null = null;
let pendingCancel: Promise<void> | null = null;
export async function loginAI(accountId?: string): Promise<void> {
  native();
  if (snapshot.busy) throw new Error("Another AI connection operation is in progress.");
  const generation = ++loginGeneration;
  publish({ busy: "login", error: null, success: null });
  const work = (async () => {
    try {
      const status = await invoke<AIStatus>("ai_login", { accountId: accountId ?? null });
      if (generation !== loginGeneration) return;
      accept(status);
      await loadModels(() => generation === loginGeneration);
      if (generation === loginGeneration) publish({ success: "ChatGPT authorization completed." });
    } catch (cause) {
      if (generation === loginGeneration) {
        const error = safeError(cause, "ChatGPT authorization or model loading failed. You can try again.");
        publish({ error });
        throw new Error(error);
      }
    } finally {
      if (generation === loginGeneration) publish({ busy: null });
    }
  })();
  pendingLogin = work;
  try { await work; } finally { if (pendingLogin === work) pendingLogin = null; }
}

export async function cancelLoginAI(): Promise<void> {
  native();
  if (pendingCancel) return pendingCancel;
  if (snapshot.busy !== "login") return;
  ++loginGeneration; // Late login results cannot replace cancellation/newer state.
  publish({ busy: "cancel", success: null, error: null });
  const oldLogin = pendingLogin;
  const work = (async () => {
    try {
      await invoke<void>("ai_cancel_login");
      await oldLogin?.catch(() => {});
      accept(await invoke<AIStatus>("ai_status"));
      publish({ success: "Browser login canceled." });
    } catch {
      publish({ error: "Could not confirm login cancellation. Wait for the pending login before trying again." });
      // Keep subsequent login attempts serialized even when cancellation fails.
      await oldLogin?.catch(() => {});
    } finally { publish({ busy: null }); }
  })();
  pendingCancel = work;
  try { await work; } finally { pendingCancel = null; }
}

export async function selectAI(accountId: string, model?: string): Promise<void> {
  await operation("select", "Could not change the AI selection. Try again.", async () => {
    const changedAccount = snapshot.status.activeAccountId !== accountId;
    accept(await invoke<AIStatus>("ai_select", { accountId, model: model ?? null }));
    if (changedAccount) await loadModels();
  });
}

export async function disconnectAI(accountId: string): Promise<void> {
  await operation("disconnect", "Could not disconnect this account. Try again.", async () => {
    const result = await invoke<{ status: AIStatus; warning: string | null }>("ai_disconnect", { accountId });
    accept(result.status);
    publish({
      // Do not retain arbitrary backend strings that might include auth material.
      warning: result.warning ? "Disconnected locally. Remote authorization or credential deletion may be unconfirmed. Unlock your OS keyring and disconnect again; also revoke Gloss access in ChatGPT Settings." : null,
      success: "Account disconnected from Gloss.",
    });
  });
}

export async function askAI(request: AIRequest, onDelta: (text: string) => void, signal: AbortSignal): Promise<{ text: string }> {
  native();
  const aborted = () => new DOMException("The request was canceled.", "AbortError");
  if (signal.aborted) throw aborted();
  const account = snapshot.status.accounts.find((a) => a.id === request.accountId);
  if (!account?.connected || !account.canUsePlan || snapshot.status.activeAccountId !== request.accountId ||
      snapshot.status.selectedModel !== request.model || !snapshot.status.models.some((m) => m.id === request.model)) {
    throw new Error("Select a connected ChatGPT account and an available model before asking.");
  }
  const channel = new Channel<AIDelta>();
  channel.onmessage = (delta) => { if (!signal.aborted) onDelta(delta.text); };
  let started = false;
  const sendCancel = () => {
    if (started) void invoke<void>("ai_cancel", { requestId: request.requestId }).catch(() => {});
  };
  const cancel = sendCancel; // Native also remembers cancellations before registration.
  signal.addEventListener("abort", cancel, { once: true });
  try {
    // Invoke first so native sees ask before cancel where possible.
    const result = invoke<{ text: string }>("ai_ask", { request, onDelta: channel });
    started = true;
    if (signal.aborted) cancel();
    const response = await result;
    if (signal.aborted) throw aborted();
    return response; // Native resolves only on response.completed.
  } catch (error) {
    if (signal.aborted) throw aborted();
    throw new Error(safeError(error, "The ChatGPT response did not complete. No paid fallback was used."));
  } finally {
    signal.removeEventListener("abort", cancel);
    channel.onmessage = () => {};
  }
}
