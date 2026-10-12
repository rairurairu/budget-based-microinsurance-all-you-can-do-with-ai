// `pnpm --filter @microinsurance/ops-console build [snapshot.json]`: after tsc compiles src/ into
// dist/js, writes dist/index.html, the live console the API serves at /ops (it reads the queue
// from the API), and dist/snapshot.html, the page rendered from an ops snapshot that opens
// offline, by default the demo one (`DEMO_MODE=1 OPS_SNAPSHOT=apps/ops-console/demo/snapshot.json
// cargo run -p microinsurance-workers`).
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { VIEW } from "./copy.ts";
import type { Snapshot } from "./queue.ts";
import { STYLE, escapeHtml, render } from "./render.ts";

const app = join(dirname(fileURLToPath(import.meta.url)), "..");
const source = process.argv[2] ?? join(app, "demo", "snapshot.json");
const snapshot = JSON.parse(readFileSync(source, "utf8")) as Snapshot;
mkdirSync(join(app, "dist"), { recursive: true });
writeFileSync(join(app, "dist", "snapshot.html"), render(snapshot));
const live = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(VIEW.title)}</title>
<style>${STYLE}</style>
<script type="module" src="./js/ops-console/src/main.js"></script>
</head>
<body><main id="app">…</main></body>
</html>
`;
writeFileSync(join(app, "dist", "index.html"), live);
console.log(`wrote ${join(app, "dist")} (snapshot: ${snapshot.items.length} items)`);
