// The live console: the work queue from the API (`GET /ops/queue`) and each button posted as the
// signed-in person's decision (`POST /ops/decisions`). The person is the token's, never a field
// the page sends (R15); the API refuses an action the item does not offer (R13). The token comes
// from the page's fragment (`#token=…`) or, in demo mode, the demo sessions.
import { ApiError, type Schemas, createClient } from "../../api-client/src/index.ts";
import { ACTION, LIVE } from "./copy.ts";
import { adoption, body, escapeHtml } from "./render.ts";

const root = document.getElementById("app") as HTMLElement;
let token: string | null = null;
const api = createClient({ baseUrl: "", token: () => token });
let snapshot: Schemas["OpsSnapshot"] | null = null;
let jobs: Schemas["JobsView"] | null = null;
let pilot: Schemas["PilotReport"] | null = null;
// The market's currency, from /registry.
let currency = "";
let flash = "";
let sessions: { label: string; token: string }[] = [];

function show(): void {
  const picker = sessions.length
    ? `<label>${escapeHtml(LIVE.as)} <select data-change="session">${sessions
        .map(
          (s) =>
            `<option value="${escapeHtml(s.token)}"${s.token === token ? " selected" : ""}>${escapeHtml(s.label)}</option>`,
        )
        .join("")}</select></label>`
    : "";
  const note = flash ? `<p class="flash" role="status">${escapeHtml(flash)}</p>` : "";
  root.innerHTML = `<header class="live">${picker}</header>${note}${snapshot ? body(snapshot, currency) : ""}${adoption(jobs, pilot)}`;
}

async function load(): Promise<void> {
  currency = (await api.get("/registry")).currency;
  snapshot = await api.get("/ops/queue");
  // A market without a pilot answers 404; the console shows the rest.
  jobs = await api.get("/ops/jobs").catch(() => null);
  pilot = await api.get("/ops/pilot").catch(() => null);
}

async function run(step: () => Promise<void>): Promise<void> {
  try {
    await step();
  } catch (e) {
    flash = e instanceof ApiError && e.body ? LIVE.refused(e.body.detail) : LIVE.signedOut;
  }
  show();
}

function field(item: string, name: string): string {
  const scope = root.querySelector<HTMLElement>(`.fields[data-item="${CSS.escape(item)}"]`);
  return (
    scope
      ?.querySelector<HTMLInputElement | HTMLSelectElement>(`[data-field="${name}"]`)
      ?.value.trim() ?? ""
  );
}

root.addEventListener("click", (event) => {
  const el = (event.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]");
  const item = el?.dataset.item;
  const action = el?.dataset.action as Schemas["PersonAction"] | undefined;
  if (!item || !action) {
    return;
  }
  void run(async () => {
    const amount = Number(field(item, "amount"));
    const kinds = field(item, "evidence_kinds")
      .split(",")
      .map((k) => k.trim())
      .filter(Boolean);
    const decision: Schemas["PersonDecision"] = {
      item_id: item,
      action,
      note: field(item, "note"),
      decline_reason:
        action === "decline" ? (field(item, "decline_reason") as Schemas["DeclineReason"]) : null,
      amount: action === "approve" && amount > 0 ? Math.round(amount * 100) : null,
      evidence_kinds: action === "request_evidence" ? kinds : null,
    };
    await api.post("/ops/decisions", { body: decision });
    flash = LIVE.done(ACTION[action], item);
    await load();
  });
});

root.addEventListener("change", (event) => {
  const el = event.target as HTMLSelectElement;
  if (el.dataset.change === "session") {
    token = el.value;
    flash = "";
    void run(load);
  }
});

async function start(): Promise<void> {
  token = new URLSearchParams(location.hash.slice(1)).get("token");
  const demo = await fetch("/demo/sessions").catch(() => null);
  if (demo?.ok) {
    sessions = (
      (await demo.json()) as { label: string; token: string; principal: { kind: string } }[]
    )
      .filter((s) => s.principal.kind === "staff")
      .map(({ label, token }) => ({ label, token }));
    token = token ?? sessions[0]?.token ?? null;
  }
  if (!token) {
    flash = LIVE.signedOut;
    show();
    return;
  }
  await run(load);
}

void start();
