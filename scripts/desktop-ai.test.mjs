import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer } from "vite";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";

// Use the project's TypeScript loader without installing another test framework.
const server = await createServer({ configFile: false, server: { middlewareMode: true }, appType: "custom" });
let doc;
try { doc = await server.ssrLoadModule("/lib/doc.ts"); }
finally { await server.close(); }
const { indexArticle, docPayload, citationMatches } = doc;
const article = (texts) => ({ id: "paper", title: "Context boundaries", blocks: texts.map((text, i) => ({ id: `b${i + 1}`, type: "p", runs: [{ text }] })) });

test("nearby context includes a multi-paragraph selection and exactly two neighbors on each side", () => {
  const ix = indexArticle(article(Array.from({ length: 11 }, (_, i) => `Distinct paragraph ${i + 1}`)));
  const context = docPayload(ix, "b4", "nearby", "b6");
  assert.deepEqual(context.paras.map((p) => p.n), [2, 3, 4, 5, 6, 7, 8]);
  assert.equal(context.partial, true);
  assert.deepEqual(docPayload(ix, "b1", "nearby").paras.map((p) => p.n), [1, 2, 3]);
  assert.deepEqual(docPayload(ix, "b11", "nearby").paras.map((p) => p.n), [9, 10, 11]);
});

test("desktop budget includes the full selected range or refuses it, never silently truncates it", () => {
  const ix = indexArticle(article(["a".repeat(13000), "b".repeat(19000), "c"]));
  assert.equal(docPayload(ix, "b1", "nearby", "b2").paras.reduce((sum, p) => sum + p.text.length, 0), 32000);
  assert.throws(() => docPayload(ix, "b1", "nearby", "b3"), /exceeds/);
  assert.throws(() => docPayload(indexArticle(article(["x".repeat(32001)])), "b1", "document"), /exceeds/);
  const window = docPayload(ix, "b3", "document");
  assert.deepEqual(window.paras.map((p) => p.n), [2, 3]);
  assert.equal(window.partial, true);
  assert.equal(docPayload(ix).paras.length, 3); // Web retains its larger budget.
});

test("a document that fits is sent in full, with table text and original paragraph numbers", () => {
  const paper = article(["One", "Two"]);
  paper.blocks.unshift({ id: "heading", type: "h2", runs: [{ text: "Not a paragraph" }] });
  paper.blocks.push({ id: "table", type: "table", runs: [], rows: [["A", "B"], ["7", "19"]] });
  const payload = docPayload(indexArticle(paper), "b2", "document");
  assert.deepEqual(payload.paras, [{ n: 1, text: "One" }, { n: 2, text: "Two" }, { n: 3, text: "A | B\n7 | 19" }]);
  assert.ok(!payload.partial);
});

test("citations reject unsent, changed, moved and invented paragraphs, including range endpoints", () => {
  const paper = article(["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta"]);
  const ix = indexArticle(paper);
  const context = { doc: docPayload(ix, "b1", "nearby"), blocks: { 1: "b1", 2: "b2", 3: "b3" } };
  assert.equal(citationMatches(ix, context, 1, 3), true);
  assert.equal(citationMatches(ix, context, 4), false); // Exists, but was not sent.
  assert.equal(citationMatches(ix, context, 1, 4), false);
  assert.equal(citationMatches(ix, context, 2, 1), false);
  const changed = structuredClone(paper);
  changed.blocks[2].runs[0].text = "Different Gamma";
  assert.equal(citationMatches(indexArticle(changed), context, 1, 3), false);
  assert.equal(citationMatches(indexArticle(changed), context, 1), true);
  const moved = structuredClone(paper);
  [moved.blocks[0], moved.blocks[1]] = [moved.blocks[1], moved.blocks[0]];
  assert.equal(citationMatches(indexArticle(moved), context, 1), false);
  assert.equal(citationMatches(ix, context, 999999), false);
});

test("desktop follow-ups retain the first answer and saved source; partial failures remain incomplete after reload", async () => {
  globalThis.window = Object.assign(new EventTarget(), { crypto: globalThis.crypto });
  globalThis.isTauri = true;
  const memory = new Map();
  globalThis.localStorage = { getItem: (k) => memory.get(k) ?? null, setItem: (k, v) => memory.set(k, v), removeItem: (k) => memory.delete(k) };
  const loader = await createServer({ configFile: false, define: { "process.env.NEXT_PUBLIC_DESKTOP": JSON.stringify("1") }, server: { middlewareMode: true }, appType: "custom" });
  try {
    const { indexArticle } = await loader.ssrLoadModule("/lib/doc.ts");
    const { useStore, flushStore } = await loader.ssrLoadModule("/lib/store.ts");
    const { refreshAI } = await loader.ssrLoadModule("/lib/desktop-ai.ts");
    const { startGloss, followUp } = await loader.ssrLoadModule("/lib/ask.ts");
    const status = { accounts: [{ id: "fixture", label: "Fixture", connected: true, canUsePlan: true }], activeAccountId: "fixture", selectedModel: "fixture-model", models: [{ id: "fixture-model", label: "Fixture model" }] };
    const requests = [];
    let fail = false;
    mockIPC(async (command, args) => {
      if (command === "ai_status" || command === "ai_models") return status;
      if (command === "save_state") return;
      if (command === "ai_ask") {
        requests.push(args.request);
        args.onDelta.onmessage({ text: "First answer [¶2]" });
        if (fail) throw "untrusted provider body with secret material";
        return { text: "First answer [¶2]" };
      }
      throw new Error(`Unexpected fixture command: ${command}`);
    });
    useStore.setState({ hydrated: true });
    await refreshAI();
    const paper = article(["Alpha", "Beta"]);
    const ix = indexArticle(paper);
    const id = startGloss(ix, { action: "ask", at: "b2", question: "What is Beta?" });
    const latest = () => useStore.getState().glosses[paper.id].find((g) => g.id === id).turns.at(-1);
    const settled = async () => {
      for (let i = 0; i < 100; i++) {
        if (!latest().streaming) return;
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
      assert.fail("Answer did not settle");
    };
    await settled();
    // Reordering the live paper must not change the source of this conversation.
    followUp(indexArticle({ ...paper, blocks: [...paper.blocks].reverse() }), id, "Explain further");
    await settled();
    assert.equal(requests[1].messages[1].content, "First answer [¶2]");
    assert.equal(requests[1].messages.at(-1).content, "Explain further");
    assert.match(requests[1].messages[0].content, /¶2/);
    assert.match(requests[1].instructions, /\[¶1\] Alpha/);
    fail = true;
    followUp(ix, id, "Another question");
    await settled();
    assert.equal(latest().content, "First answer [¶2]");
    assert.match(latest().error, /did not complete/);
    assert.ok(!latest().error.includes("secret"));
    await flushStore();
    useStore.setState({ hydrated: false, glosses: {} });
    useStore.getState().hydrate();
    assert.equal(latest().content, "First answer [¶2]");
    assert.match(latest().error, /did not complete/);
    await flushStore();
  } finally {
    clearMocks();
    await loader.close();
    delete globalThis.window;
    delete globalThis.isTauri;
    delete globalThis.localStorage;
  }
});
