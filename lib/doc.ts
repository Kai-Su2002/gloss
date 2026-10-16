import { blockText, modelText, numberBlocks, wordCount, type Article } from "./article";
import type { DocPayload } from "./prompts";

/** Everything the reader derives once from an article. */
export type DocIndex = {
  article: Article;
  texts: Map<string, string>;
  order: Map<string, number>;
  numbers: Map<string, number>;
  byNumber: Map<number, string>;
  words: number;
};

export function indexArticle(article: Article): DocIndex {
  const texts = new Map<string, string>();
  const order = new Map<string, number>();
  article.blocks.forEach((b, i) => {
    texts.set(b.id, blockText(b));
    order.set(b.id, i);
  });
  const numbers = numberBlocks(article.blocks);
  const byNumber = new Map<number, string>();
  numbers.forEach((n, id) => byNumber.set(n, id));
  return { article, texts, order, numbers, byNumber, words: wordCount(article) };
}

/** Roughly 100k tokens. Past that, send a window around where the reader is. */
const BUDGET = 400_000;

export type ContextScope = "nearby" | "document";

export function docPayload(ix: DocIndex, focusBlock?: string, scope?: ContextScope, endBlock?: string): DocPayload {
  const paras = ix.article.blocks
    .filter((b) => ix.numbers.has(b.id))
    .map((b) => ({ n: ix.numbers.get(b.id)!, text: b.type === "table" ? modelText(b) : ix.texts.get(b.id) ?? "" }));
  const base = { title: ix.article.title, author: ix.article.author, published: ix.article.published };
  const total = paras.reduce((a, p) => a + p.text.length, 0);
  const budget = scope ? 32_000 : BUDGET;
  if (scope !== "nearby" && total <= budget) return { ...base, paras };

  // Grow a window outward from the focus paragraph until the budget is spent.
  const center = Math.max(0, paras.findIndex((p) => p.n === (focusBlock ? ix.numbers.get(focusBlock) : 1)));
  const end = endBlock ? Math.max(center, paras.findIndex((p) => p.n === ix.numbers.get(endBlock))) : center;
  let lo = center;
  let hi = scope ? end : center;
  let used = paras.slice(lo, hi + 1).reduce((sum, p) => sum + p.text.length, 0);
  if (scope && used > budget) throw new Error("This selection exceeds the 32,000-character context limit. Select a shorter passage.");
  while (lo > 0 || hi < paras.length - 1) {
    const grew = used;
    if (hi < paras.length - 1 && (scope !== "nearby" || hi < end + 2) && used + paras[hi + 1].text.length <= budget) used += paras[++hi].text.length;
    if (lo > 0 && (scope !== "nearby" || lo > center - 2) && used + paras[lo - 1].text.length <= budget) used += paras[--lo].text.length;
    if (used === grew) break;
  }
  return { ...base, paras: paras.slice(lo, hi + 1), partial: scope ? lo > 0 || hi < paras.length - 1 : true };
}

/** A saved citation must still identify the exact source paragraphs sent. */
export function citationMatches(ix: DocIndex, context: { doc: DocPayload; blocks: Record<number, string> }, from: number, to = from): boolean {
  if (!Number.isInteger(from) || !Number.isInteger(to) || to < from || to - from >= context.doc.paras.length) return false;
  for (let n = from; n <= to; n++) {
    const saved = context.doc.paras.find((p) => p.n === n);
    const id = context.blocks[n];
    const block = ix.article.blocks.find((b) => b.id === id);
    if (!saved || !block || ix.byNumber.get(n) !== id || saved.text !== modelText(block)) return false;
  }
  return true;
}
