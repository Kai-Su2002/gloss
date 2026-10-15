import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const root = fileURLToPath(new URL("../", import.meta.url));

export default defineConfig({
  root: `${root}desktop`,
  publicDir: `${root}public`,
  plugins: [react(), {
    name: "font-licenses",
    generateBundle() {
      const fonts = ["newsreader", "schibsted-grotesk", "literata", "atkinson-hyperlegible-next"];
      const source = fonts.map((font) => `${font}\n\n${readFileSync(`${root}node_modules/@fontsource-variable/${font}/LICENSE`, "utf8")}`).join("\n\n---\n\n");
      this.emitFile({ type: "asset", fileName: "font-licenses.txt", source });
    },
  }],
  resolve: {
    alias: [
      // The shared UI's only Next-specific browser API is navigation.
      { find: /^next\/(link|navigation)$/, replacement: `${root}desktop/navigation.tsx` },
      // Desktop has no cloud account. Keep Clerk and server-only code out of this bundle.
      { find: "@clerk/nextjs", replacement: `${root}desktop/account.tsx` },
      { find: "@", replacement: root },
    ],
  },
  define: { "process.env.NEXT_PUBLIC_DESKTOP": JSON.stringify("1") },
  server: { host: "0.0.0.0", port: 1420, strictPort: true },
  preview: { host: "0.0.0.0", port: 1420, strictPort: true },
  build: { outDir: `${root}dist-desktop`, emptyOutDir: true, target: "es2022" },
});
