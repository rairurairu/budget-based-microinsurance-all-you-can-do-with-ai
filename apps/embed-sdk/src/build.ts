// `pnpm --filter @microinsurance/embed-sdk build`: after tsc compiles src/ into dist/js, copies the
// demo partner page into dist/, which the API serves at /embed. A partner loads
// `/embed/js/embed-sdk/src/index.js` as a module.
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const app = join(dirname(fileURLToPath(import.meta.url)), "..");
mkdirSync(join(app, "dist"), { recursive: true });
copyFileSync(join(app, "static", "index.html"), join(app, "dist", "index.html"));
console.log(`wrote ${join(app, "dist")}`);
