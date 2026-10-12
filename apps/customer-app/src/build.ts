// `pnpm --filter @microinsurance/customer-app build`: after tsc compiles src/ into dist/js, copies
// the page and its stylesheet into dist/, which the API serves at /app.
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const app = join(dirname(fileURLToPath(import.meta.url)), "..");
mkdirSync(join(app, "dist"), { recursive: true });
for (const file of ["index.html", "style.css"]) {
  copyFileSync(join(app, "static", file), join(app, "dist", file));
}
console.log(`wrote ${join(app, "dist")}`);
