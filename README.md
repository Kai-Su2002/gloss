<p align="center">
  <a href="https://usegloss.io"><img alt="Gloss" src="public/og.png"></a>
</p>

<h1 align="center">Gloss</h1>

<p align="center">A reader for articles, chapters and PDFs: select any line, start typing, and an open model writes the answer in the margin beside it, citing paragraphs you can check: <a href="https://usegloss.io">usegloss.io</a></p>

## Tech stack

- Next.js 16 (App Router, Turbopack), React 19, Tailwind v4, Zustand. Local-first: texts live in IndexedDB, and glosses, highlights and settings in localStorage, all in the browser and all working signed out.
- Optional accounts with Clerk. Signed in, your shelf, texts, highlights, notes, glosses and reading progress sync to Postgres on Neon (`lib/sync.ts`, `app/api/sync`); a text is fetched onto another device when it is opened there.
- Your own notes on any highlight, in the margin beside the model's (`Lens.tsx`, `Margin.tsx`). A daily review on the home page brings back five of your highlights, spaced like flashcards (`lib/review.ts`); the shelf searches every highlight and note, and exports them all as Markdown files ready for Obsidian (`lib/export.ts`, `lib/zip.ts`).
- The library (`/library`, `components/library/`): everything on your shelf as a bookcase, arranged by topic, recency, colour or length, with a topic view of painted plates and a 3D book that comes off the shelf. Any text can be renamed (click its title, there or in the shelf sheet), and the three samples can be taken off the shelf, remembered per device (`hiddenSamples` in `lib/library-prefs.ts`). Every new text is sorted onto one of 24 fixed topics, with a few tags, by one DeepSeek V4.1 Flash call (`lib/shelve.ts`, `app/api/shelve`, about $0.0001 a text); texts from before are sorted the first time the library opens (`lib/shelving.ts`). Covers show the page's own picture printed in the topic's ink; painted covers (FLUX.2 [dev], `lib/paint.ts`) are opt-in under Aa, kept on the device and, signed in, with the account (`covers`). Topic plates and icons are generated art in `public/library/`.
- Agents can save to the library through an MCP server at `/mcp` (`lib/mcp.ts`): Streamable HTTP, stateless, four tools (`save_articles`, `save_text`, `search_library`, `list_topics`). Each reader makes a token in the library under Connect an agent (`app/api/agent-tokens`, only its hash is stored in `agent_tokens`), sent as `Authorization: Bearer gls_...` or as the last part of the URL. Saved links go through the same importer as the site (`lib/fetch-article.ts`); `save_text` takes Markdown the agent read itself, for what Gloss can't fetch (X Articles and long posts, video transcripts, pages behind a login). Both are sorted and written to the reader's synced shelf, so they appear on the next sync. Up to 25 links a call, one text a call, and 1,000 saves a day.
- Read aloud, off by default (Aa, then Listen): Cartesia Sonic 3 on Together, a passage at a time, fetched just ahead of the voice (`app/api/speak`, `components/reader/Listen.tsx`). It always uses the reader's own key.
- Free use of the shared key is a dollar budget (`lib/quota.ts`): what answers and briefs actually cost, up to 10¢ a day signed out (by hashed IP) or 20¢ signed in, and $1,000 a day across everyone, kept in Postgres (`free_usage`).
- Newsreader and Schibsted Grotesk, plus Literata and Atkinson Hyperlegible as optional reading faces; six themes, four pens and four highlighters from CSS variables, applied before first paint.
- Together AI over raw SSE fetch: DeepSeek V4.1 Flash (Quick, about 0.5s to first token) and Kimi K3 (Deep, at low reasoning effort), allow-listed in `lib/models.ts`.
- Web search with Exa (`lib/search.ts`, instant mode, about 0.5s): the model gets a `web_search` tool and decides when to look something up (a named person, organisation, work, or anything newer than the text); Fact-check always searches. On by default, the Web toggle turns it off. Results are cited as [1], [2] beside the text's own [¶n].
- The whole text goes in the system prompt with `[¶n]` numbers, so follow-ups hit Together's prefix cache and answers cite paragraphs.
- Anchors are block id plus character offsets, rendered as React marks; margin layout and leader lines in `components/reader/Margin.tsx`.
- Import: links through `@mozilla/readability` on `linkedom` (with a private-address guard), keeping figures (by their original URL, loaded without a referrer) and data tables (which the model reads too); plus pasted text and `.txt`/`.md`. Posts and videos (X, Threads, Bluesky, YouTube and the like) are turned away with a pointer to Paste text (`notAnArticle` in `lib/fetch-article.ts`). Two bookmarklets in the library's Add panel open any page at `/?url=` (read now) or `/?save=` (shelve for later); the same panel takes a Pocket, Instapaper or Raindrop export and saves its links 100 at a time.
- PDFs (`lib/pdf.ts`): LiteParse, Rust compiled to WebAssembly (`public/liteparse-2.15.1.wasm`, copied from `@llamaindex/liteparse-wasm`), reads the text layer in the browser, columns, headings, lists and tables, in milliseconds. Pages with no usable text (scans, garbled glyphs) are rendered and read by Qwen3.8 Flash on Together (`app/api/pdf-page`). Over 20 pages needs the reader's own key, since every question sends the whole text. The original PDF is kept in IndexedDB on that device. Pages whose layout looks hard (columns, tables, maths) are read again in the background by Qwen3.5-9B (`lib/upgrade.ts`, mode "layout" in `app/api/pdf-page`) and swapped in one at a time, while nothing cites their paragraphs yet. When columns, tables or maths still come out jumbled, "Read it more carefully" (`lib/careful.ts`, `app/api/pdf-careful`) reads the PDF again with LlamaParse's agentic tier, for readers with their own Together key, up to 200 pages a day each.

## Cloning & running

1. Install: `npm install`
2. Add keys to `.env.local` (see `.env.example`): `TOGETHER_API_KEY` (without one, answers are labelled demo output and readers can paste their own key in the app), `DATABASE_URL` from Neon, and `NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY` and `CLERK_SECRET_KEY` from Clerk
3. Create the tables: `npm run db:migrate` (safe to run again)
4. Run: `npm run dev`, open http://localhost:3100, and paste a link (or press ⌘V anywhere), or open the library and shelve a few classics
5. Read: select text and type to ask, `/` asks about the whole text, `j` and `k` walk paragraphs, `B` glosses the whole text, `F` is focus mode, `?` lists every key
6. Check: `npm run lint && npm run build`, then deploy with `vercel --prod` (set the same variables, plus `NEXT_PUBLIC_SITE_URL=https://usegloss.io`, in the project first)

## Desktop literature library (first version)

The desktop app uses **Tauri 2 + Rust + SQLite**, reusing Gloss's bookcase,
reader, themes, highlights and margin notes. `desktop/` is a separate Vite entry
point; the Next.js website and its server routes remain available separately.
It does not wrap the hosted website or require a Next.js server at runtime.

- Import local text PDFs (up to 400 pages), Markdown and plain text; keep original
  PDFs, extracted documents, shelf entries, notes and reading progress locally.
- The native app stores these in `gloss.sqlite3` in Tauri's application data
  directory (`~/Library/Application Support/io.gloss.desktop/` on macOS).
  SQLite is authoritative; browser storage is only a synchronous UI cache.
  Back up the **whole directory with the app closed**, not just a live SQLite file.
- Local reading and notes need no account, API key or network. Fonts and PDF parsing
  are bundled. Optional ChatGPT Q&A uses the connected account's plan in OpenAI's
  cloud. OCR, cloud sync, URL import and automatic classification remain unavailable.
  It is not yet a full citation manager:
  DOI metadata retrieval, BibTeX/RIS import and citation generation are future work.
- PDFs are read as extracted text, not a full-fidelity page viewer. Do not remove
  your source files until you have an independent backup. Desktop and web libraries
  are separate; there is no automatic migration between them.

### Develop and build

Install Node.js 22.12+ and stable Rust, plus the [Tauri system
prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS. macOS needs
Xcode Command Line Tools. In an Amp orb, `.agents/setup` prepares the Linux toolchain.

```sh
npm install
npm run desktop dev           # native app + Vite hot reload
npm run desktop:build         # frontend and TypeScript check
npm run desktop:test          # native storage and navigation tests
node --test scripts/desktop-ai.test.mjs  # context budgets and citation validity
```

`npm run desktop:dev` alone is a **browser UI preview**: it uses browser storage,
not SQLite. A banner makes this distinction explicit. The installed app uses
native storage. To build an Apple Silicon DMG, run on macOS:

```sh
rustup target add aarch64-apple-darwin
npm run desktop build -- --target aarch64-apple-darwin --bundles dmg -- --locked
```

### ChatGPT Q&A (phase 1)

In the installed app, open **AI → Continue with ChatGPT**. Authorization opens
your system browser and returns through a temporary `127.0.0.1` callback. Choose
an authorized account and a model from that account's catalog. No ChatGPT password
or copied session cookie is accepted by Gloss. API Key and Local model are
future entries, not working providers in this version.

Select a passage and choose **Ask**, or press `/` to ask about the paper. Before
submitting, the reader shows the paragraph range sent to OpenAI. Nearby context
includes the selected paragraphs and up to two neighbors on either side. The
document option sends the whole paper when it fits, otherwise a disclosed window;
both have a 32,000-character source-text budget. Oversized selections are refused.
No document text is uploaded merely by connecting an account.

Answers stream into the margin and persist in local SQLite with their account/model
metadata and original context. Follow-ups reuse that saved context and up to 12
recent non-error conversation turns. Paragraph citations are navigable only when
they were included in the sent context and still match the current document.
This checks source identity, **not whether the model's claim is true**. Stopped,
interrupted and quota-failed answers keep partial text but remain marked incomplete.
Switching accounts/models, leaving a paper or closing the app cancels active requests.

OAuth tokens stay in the OS credential service (macOS Keychain; Linux Secret
Service), never SQLite or browser storage. A locked/unavailable credential service
must be unlocked; Gloss does not fall back to plaintext. Disconnect removes the
local session and attempts remote revocation; heed any warning and revoke access in
ChatGPT Settings if needed. Plan eligibility, available models and limits are set by
OpenAI. There is no automatic paid API fallback or web search in desktop Q&A.

This uses the official [local/open-source token-sharing flow](https://developers.openai.com/siwc/token-sharing-open-source/).
Do not assume it authorizes a commercial or hosted offering; check OpenAI's current
eligibility and the repository's licensing before distribution. Browser preview
cannot authorize accounts. Automated tests use controlled HTTP/vault fixtures;
real subscription login/inference and macOS Keychain/DMG validation require a real
account and a Mac and have not been exercised in the Linux orb.

### Releases and macOS security

`.github/workflows/desktop.yml` builds `Gloss_<version>_aarch64.dmg` on macOS.
A manual workflow run only uploads a build artifact. Pushing a `desktop-vX.Y.Z`
tag builds and publishes a GitHub Release; do this only when ready to publish.
The version is taken from the tag, or `src-tauri/tauri.conf.json` for manual builds.

The current configuration uses **ad-hoc signing**, not Apple Developer ID signing
or notarization. It does not require a paid Apple account, but downloaded builds
can trigger Gatekeeper. Only explicitly allow an app you trust in macOS Privacy
& Security; do not disable Gatekeeper globally. To distribute without that
warning, configure Developer ID signing and notarization using an Apple Developer
Program membership and protected CI secrets. That paid path is **not configured**.
Creating this workflow does not itself publish a release.

### Amp orbs

`.agents/setup` uses the orb's preinstalled Node.js/npm, installs dependencies from
`package-lock.json` (including development tools), installs missing Linux Tauri
system packages and stable Rust, fetches the locked Cargo dependencies, and copies
`.env.example` to `.env.local` only if it is missing. Amp snapshots these for
future orbs; warm setup runs reuse the installed packages instead of removing
them with `npm ci`. `.agents/resume` only restores a missing environment template
and never reinstalls dependencies.

Setup does not authenticate services, run database migrations, or start a server.
Configure the keys from `.env.example` through Amp secrets/environment settings or
the orb's `.env.local` when those features are needed. Run `npm run db:migrate`
only against an explicitly chosen development database. Start the dev server as a
supervised service with `amp orb service start gloss --command 'npm run dev' --port 3100 --portal`.
The setup hooks become active for future project orbs once committed to the
project's default branch.

## Roadmap

- [ ] ship it as a browser extension that opens the same margin on any page
- [ ] let a gloss compare two passages you pick at once
- [ ] move the shared-key rate limit from memory to a store like Upstash so it holds across instances
- Accounts are skipped on purpose for now: the reader should work the second you paste a link.

## Security

`TOGETHER_API_KEY` stays server side. A key a reader pastes lives in their browser's localStorage and is sent only with their own requests. Free use of the shared key is capped per reader per day in dollars, and in total, as above; reading aloud ($65 per million characters on Sonic 3) never uses it; link import refuses private, loopback and link-local addresses on every redirect hop.

Samples, all public domain via Project Gutenberg: Huxley, "On a Piece of Chalk" (1868); Schopenhauer, "On Thinking for Oneself" (trans. T. Bailey Saunders); Thoreau, "Reading" from Walden (1854). Rebuild them with `scripts/build-samples.mjs`; dashes are normalized to plain punctuation.
