// The customer app's runtime in the browser: it holds the model, calls the API through the
// generated client, re-renders the screens (views.ts) and listens for payouts as they are
// decided. The token comes from the page's fragment (`#token=…`, never sent to a server) when
// the partner's app or the insurer's sign-in opens the page, or, in demo mode, from the demo
// sessions.
import { ApiError, type Schemas, createClient, money, t } from "../../api-client/src/index.ts";
import { type Model, type Tab, render, tabs } from "./views.ts";

const root = document.getElementById("app") as HTMLElement;
const KEY = "microinsurance.token";
// Demo tokens are issued afresh on each load: demo mode remembers who, not the token.
const DEMO_KEY = "microinsurance.demo";

function stored(key = KEY): string | null {
  try {
    return sessionStorage.getItem(key);
  } catch {
    return null;
  }
}

function store(value: string, key = KEY): void {
  try {
    sessionStorage.setItem(key, value);
  } catch {
    // Private mode: the token lives for this page only.
  }
}

let token: string | null = null;
const api = createClient({ baseUrl: "", token: () => token });

let model: Model;
let streaming: AbortController | null = null;

function show(): void {
  root.innerHTML = render(model);
}

function fail(e: unknown): void {
  if (e instanceof ApiError && e.status === 401) {
    model.flash = { kind: "error", text: t(model.copy, "errors.signed_out") };
  } else if (e instanceof ApiError && e.body) {
    model.flash = {
      kind: "error",
      text: t(model.copy, "errors.refused", { detail: e.body.detail }),
    };
  } else {
    model.flash = { kind: "error", text: t(model.copy, "errors.generic") };
  }
  show();
}

async function orNull<T>(call: Promise<T>): Promise<T | null> {
  try {
    return await call;
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) {
      return null;
    }
    throw e;
  }
}

async function loadCopy(language: string): Promise<void> {
  const pack = await api.get("/copy/{language}", { path: { language } });
  model.copy = pack.text;
  model.language = pack.language;
  document.documentElement.lang = pack.language;
}

/** Everything the customer's screens show, read again after each change. */
async function refresh(): Promise<void> {
  const me = await api.get("/me");
  model.me = me;
  const language = me.profile.language ?? model.registry.default_language;
  if (language !== model.language) {
    await loadCopy(language);
  }
  const [policy, tasks, score, rewards, payouts, claims, decisions, answers, plans] =
    await Promise.all([
      orNull(api.get("/me/policy")),
      api.get("/me/prevention-tasks"),
      api.get("/me/score"),
      orNull(api.get("/me/rewards")),
      api.get("/me/payouts"),
      api.get("/me/claims"),
      api.get("/me/decisions"),
      api.get("/me/rating-answers"),
      api.get("/plans", me.partner_id ? { query: { channel: me.partner_id } } : {}),
    ]);
  Object.assign(model, { policy, tasks, score, rewards, payouts, claims, decisions, plans });
  model.ratingAnswers = answers.answers;
  if (!tabs(model).includes(model.tab)) {
    model.tab = model.policy ? "schedule" : "plans";
  }
}

function listen(): void {
  streaming?.abort();
  const mine = new AbortController();
  streaming = mine;
  model.live = true;
  api
    .stream(
      "/me/payouts/stream",
      (event, data) => {
        if (event !== "payout") {
          return;
        }
        const notice = data as Schemas["PayoutNotice"];
        model.payouts = [
          notice,
          ...model.payouts.filter(
            (p) => p.decision.idempotency_key !== notice.decision.idempotency_key,
          ),
        ];
        model.flash = {
          kind: "ok",
          text: t(model.copy, "payouts.new", {
            amount: money(notice.decision.amount, model.registry.currency),
          }),
        };
        show();
      },
      mine.signal,
    )
    .catch(() => {
      // A stream this page replaced ended on purpose; only the current one going down matters.
      if (streaming === mine) {
        model.live = false;
        show();
      }
    });
}

async function signIn(next: string): Promise<void> {
  token = next;
  store(next);
  model.quote = undefined;
  model.renewal = undefined;
  model.flash = undefined;
  if (model.demo) {
    model.demo.current = next;
    const label = model.demo.sessions.find((s) => s.token === next)?.label;
    if (label) {
      store(label, DEMO_KEY);
    }
  }
  await refresh();
  listen();
}

// ---- reading forms

function form(target: HTMLFormElement): FormData {
  return new FormData(target);
}

function minor(value: FormDataEntryValue | null): number | null {
  const n = Number(value);
  return value === null || value === "" || Number.isNaN(n) ? null : Math.round(n * 100);
}

function minutes(hhmm: string): number {
  const [h, m] = hhmm.split(":").map(Number);
  return (h ?? 0) * 60 + (m ?? 0);
}

async function fileEvidence(data: FormData): Promise<Schemas["EvidenceFile"][]> {
  const sample = String(data.get("sample") ?? "");
  if (sample) {
    const res = await fetch(`/demo/evidence/${encodeURIComponent(sample)}`);
    return [(await res.json()) as Schemas["EvidenceFile"]];
  }
  const file = data.get("file");
  if (!(file instanceof File) || file.size === 0) {
    return [];
  }
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  for (const b of bytes) {
    binary += String.fromCharCode(b);
  }
  return [{ media_type: file.type || "image/jpeg", data_base64: btoa(binary) }];
}

// ---- actions

type Action = (arg: string) => Promise<void>;

const ACTIONS: Record<string, Action> = {
  tab: async (arg) => {
    model.tab = arg as Tab;
    model.flash = undefined;
  },
  quote: async (plan) => {
    model.quote = await api.post("/me/quotes", { body: { plan_id: plan } });
  },
  accept: async (quoteId) => {
    const done = await api.post("/me/quotes/{quote_id}/accept", { path: { quote_id: quoteId } });
    model.quote = undefined;
    model.flash = {
      kind: "ok",
      text: done.policy
        ? t(model.copy, "quote.accepted_new", { date: done.policy.period.starts })
        : t(model.copy, "quote.accepted_change"),
    };
    await refresh();
    model.tab = "schedule";
  },
  renewal: async () => {
    model.renewal = await api.get("/me/renewal");
  },
  "remove-event": async (i) => {
    const events = [...(model.me?.profile.upcoming_events ?? [])];
    events.splice(Number(i), 1);
    model.me = await api.patch("/me/profile", { body: { upcoming_events: events } });
  },
  "remove-item": async (i) => {
    const items = [...(model.me?.profile.declared_items ?? [])];
    items.splice(Number(i), 1);
    model.me = await api.patch("/me/profile", { body: { declared_items: items } });
  },
  "tasks-refresh": async () => {
    await api.post("/me/prevention-tasks/refresh");
    await refresh();
  },
  "task-accept": async (id) => {
    await api.post("/me/prevention-tasks/{task_id}/accept", { path: { task_id: id } });
    await refresh();
  },
  "task-review": async (id) => {
    await api.post("/me/prevention-tasks/{task_id}/review-request", { path: { task_id: id } });
    await refresh();
  },
  redeem: async (offerId) => {
    await api.post("/me/rewards/redeem", { body: { offer_id: offerId } });
    model.flash = { kind: "ok", text: t(model.copy, "rewards.taken") };
    await refresh();
  },
  "payout-choice": async (arg) => {
    const { key, settlement } = JSON.parse(arg) as {
      key: string;
      settlement: Schemas["Settlement"];
    };
    await api.post("/me/payouts/{payout_key}/choice", {
      path: { payout_key: key },
      body: { settlement },
    });
    await refresh();
  },
  "claim-choice": async (arg) => {
    const { claim, settlement } = JSON.parse(arg) as {
      claim: string;
      settlement: Schemas["Settlement"];
    };
    await api.post("/me/claims/{claim_id}/choice", {
      path: { claim_id: claim },
      body: { settlement },
    });
    await refresh();
  },
  review: async (decisionId) => {
    const reason = window.prompt(t(model.copy, "decisions.reason")) ?? "";
    if (!reason.trim()) {
      return;
    }
    await api.post("/me/review-requests", { body: { decision_id: decisionId, reason } });
    model.flash = { kind: "ok", text: t(model.copy, "decisions.sent") };
    await refresh();
  },
  identity: async () => {
    const check = await api.post("/me/identity-checks");
    model.flash = {
      kind: "ok",
      text: t(model.copy, "privacy.identity_status", { status: check.status }),
    };
  },
  cancel: async () => {
    if (!window.confirm(t(model.copy, "privacy.cancel_confirm"))) {
      return;
    }
    await api.post("/me/cancel");
    await refresh();
  },
};

type FormAction = (target: HTMLFormElement, arg: string) => Promise<void>;

const FORMS: Record<string, FormAction> = {
  split: async (target, plan) => {
    const premiums: Record<string, number> = {};
    for (const [cover, value] of form(target).entries()) {
      premiums[cover] = minor(value) ?? 0;
    }
    model.quote = await api.post("/me/quotes", { body: { plan_id: plan, premiums } });
  },
  profile: async (target) => {
    const data = form(target);
    const start = minutes(String(data.get("start") ?? "09:00"));
    const end = minutes(String(data.get("end") ?? "18:00"));
    const work_windows = data.getAll("weekday").map((w) => ({
      weekday: Number(w),
      start_minute: start,
      end_minute: end,
    }));
    const earnings = minor(data.get("earnings"));
    model.me = await api.patch("/me/profile", {
      body: {
        occupation: String(data.get("occupation") ?? ""),
        work_windows,
        commute_routes: data.getAll("route").map(String),
        work_areas: data.getAll("area").map(String),
        ...(earnings === null ? {} : { typical_daily_earnings: earnings }),
      },
    });
    model.flash = { kind: "ok", text: t(model.copy, "app.saved") };
  },
  event: async (target) => {
    const data = form(target);
    const event: Schemas["UpcomingEvent"] = {
      kind: String(data.get("kind")),
      starts: String(data.get("starts")),
      ends: null,
      reference: String(data.get("reference") ?? "") || null,
      value: minor(data.get("value")),
    };
    const events = [...(model.me?.profile.upcoming_events ?? []), event];
    model.me = await api.patch("/me/profile", { body: { upcoming_events: events } });
  },
  item: async (target) => {
    const data = form(target);
    const item: Schemas["DeclaredItem"] = {
      kind: String(data.get("kind")),
      description: String(data.get("description")),
      declared_value: minor(data.get("value")),
    };
    const items = [...(model.me?.profile.declared_items ?? []), item];
    model.me = await api.patch("/me/profile", { body: { declared_items: items } });
  },
  rating: async (target) => {
    const answers: Record<string, string> = {};
    for (const [factor, value] of form(target).entries()) {
      if (value) {
        answers[factor] = String(value);
      }
    }
    const saved = await api.put("/me/rating-answers", { body: { answers } });
    model.ratingAnswers = saved.answers;
    model.flash = { kind: "ok", text: t(model.copy, "app.saved") };
  },
  "task-evidence": async (target, taskId) => {
    const files = await fileEvidence(form(target));
    const out = await api.post("/me/prevention-tasks/{task_id}/evidence", {
      path: { task_id: taskId },
      body: { files },
    });
    if (out.granted) {
      model.flash = {
        kind: "ok",
        text: t(model.copy, "tasks.credited", {
          amount: money(out.granted.granted, model.registry.currency),
        }),
      };
    }
    await refresh();
  },
  claim: async (target) => {
    const data = form(target);
    const guarantee = minor(data.get("guarantee"));
    await api.post("/me/claims", {
      body: {
        cover_id: String(data.get("cover_id")),
        loss_date: String(data.get("loss_date")),
        amount_claimed: minor(data.get("amount")) ?? 0,
        description: String(data.get("description") ?? ""),
        circumstance: String(data.get("circumstance") ?? "") || null,
        asked_for_person: data.get("person") === "on",
        guarantee_requested: guarantee,
      },
    });
    await refresh();
  },
  "claim-evidence": async (target, claimId) => {
    const data = form(target);
    const files = await fileEvidence(data);
    await api.post("/me/claims/{claim_id}/evidence", {
      path: { claim_id: claimId },
      body: { kinds: data.getAll("kind").map(String), files },
    });
    await refresh();
  },
  consents: async (target) => {
    model.me = await api.put("/me/consents", {
      body: { consents: form(target).getAll("consent").map(String) },
    });
    model.flash = { kind: "ok", text: t(model.copy, "app.saved") };
  },
};

async function run(step: () => Promise<void>): Promise<void> {
  try {
    await step();
    show();
  } catch (e) {
    fail(e);
  }
}

root.addEventListener("click", (event) => {
  const el = (event.target as HTMLElement).closest<HTMLElement>("[data-action]");
  const action = el?.dataset.action ? ACTIONS[el.dataset.action] : undefined;
  if (el && action) {
    void run(() => action(el.dataset.arg ?? ""));
  }
});

root.addEventListener("submit", (event) => {
  const target = event.target as HTMLFormElement;
  const action = target.dataset.form ? FORMS[target.dataset.form] : undefined;
  if (action) {
    event.preventDefault();
    void run(() => action(target, target.dataset.arg ?? ""));
  }
});

root.addEventListener("change", (event) => {
  const el = event.target as HTMLSelectElement;
  if (el.dataset.change === "session") {
    void run(() => signIn(el.value));
  } else if (el.dataset.change === "language") {
    void run(async () => {
      await loadCopy(el.value);
      if (model.me) {
        model.me = await api.patch("/me/profile", { body: { language: el.value } });
      }
    });
  }
});

async function start(): Promise<void> {
  const fragment = new URLSearchParams(location.hash.slice(1)).get("token");
  if (fragment) {
    store(fragment);
    history.replaceState(null, "", location.pathname + location.search);
  }
  const registry = await api.get("/registry");
  const pack = await api.get("/copy/{language}", { path: { language: registry.default_language } });
  model = {
    copy: pack.text,
    registry,
    language: pack.language,
    tab: "plans",
    covers: await api.get("/covers"),
    plans: [],
    ratingQuestions: await api.get("/rating-questions"),
    ratingAnswers: {},
    tasks: [],
    payouts: [],
    claims: [],
    decisions: [],
    samples: [],
    live: false,
  };
  const demo = await fetch("/demo/sessions").catch(() => null);
  if (demo?.ok) {
    const sessions = (
      (await demo.json()) as { label: string; token: string; principal: { kind: string } }[]
    )
      .filter((s) => s.principal.kind === "customer")
      .map(({ label, token }) => ({ label, token }));
    model.demo = { sessions, current: "" };
    model.samples = (await (await fetch("/demo/evidence")).json()) as string[];
  }
  const remembered = model.demo?.sessions.find((s) => s.label === stored(DEMO_KEY))?.token;
  const first = fragment ?? remembered ?? model.demo?.sessions[0]?.token ?? stored() ?? null;
  if (!first) {
    model.flash = { kind: "error", text: t(model.copy, "errors.signed_out") };
    show();
    return;
  }
  await signIn(first);
  show();
}

void start().catch(fail);
