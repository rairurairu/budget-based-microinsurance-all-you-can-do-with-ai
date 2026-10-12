// The partner console's runtime: a partner token from the page's fragment (`#token=…`) or, in
// demo mode, the demo sessions; then the partner's results and offers from the API.
import { ApiError, type Schemas, createClient } from "../../api-client/src/index.ts";
import { VIEW } from "./copy.ts";
import { type Model, render } from "./views.ts";

const root = document.getElementById("app") as HTMLElement;
let token: string | null = null;
const api = createClient({ baseUrl: "", token: () => token });
// The currency is the market's, from /registry.
const model: Model = { partnerId: "", currency: "" };

function show(): void {
  root.innerHTML = render(model);
}

async function load(): Promise<void> {
  const registry = await api.get("/registry");
  model.currency = registry.currency;
  model.results = await api.get("/partners/me/results");
  model.partnerId = model.results.partner_id;
  const published = await api.get("/partners/me/reward-offers");
  // Nothing published: publishing nothing says whether the partner is a reward supplier.
  model.offers =
    published.offers.length > 0
      ? published
      : await api.put("/partners/me/reward-offers", { body: { offers: [] } }).catch(() => null);
}

async function run(step: () => Promise<void>): Promise<void> {
  try {
    await step();
  } catch (e) {
    model.flash = e instanceof ApiError && e.body ? VIEW.error(e.body.detail) : VIEW.signedOut;
  }
  show();
}

function read(form: HTMLFormElement, index: number, name: string): string {
  return (
    form.querySelector<HTMLInputElement>(`fieldset[data-index="${index}"] [name="${name}"]`)
      ?.value ?? ""
  );
}

root.addEventListener("submit", (event) => {
  const form = event.target as HTMLFormElement;
  if (form.dataset.form !== "offers") {
    return;
  }
  event.preventDefault();
  void run(async () => {
    const offers: Schemas["RewardOffer"][] = (model.offers?.offers ?? []).map((o, i) => {
      const stock = read(form, i, "inventory");
      return {
        ...o,
        description: read(form, i, "description"),
        perceived_value: Math.round(Number(read(form, i, "perceived_value")) * 100),
        price: Math.round(Number(read(form, i, "price")) * 100),
        inventory: stock === "" ? null : Number(stock),
      };
    });
    model.offers = await api.put("/partners/me/reward-offers", { body: { offers } });
    model.flash = VIEW.published;
  });
});

root.addEventListener("change", (event) => {
  const el = event.target as HTMLSelectElement;
  if (el.dataset.change === "session") {
    token = el.value;
    if (model.demo) {
      model.demo.current = el.value;
    }
    model.flash = undefined;
    void run(load);
  }
});

async function start(): Promise<void> {
  token = new URLSearchParams(location.hash.slice(1)).get("token");
  const demo = await fetch("/demo/sessions").catch(() => null);
  if (demo?.ok) {
    const sessions = (
      (await demo.json()) as { label: string; token: string; principal: { kind: string } }[]
    )
      .filter((s) => s.principal.kind === "partner")
      .map(({ label, token }) => ({ label, token }));
    token = token ?? sessions[0]?.token ?? null;
    model.demo = { sessions, current: token ?? "" };
  }
  if (!token) {
    model.flash = VIEW.signedOut;
    show();
    return;
  }
  await run(load);
}

void start();
