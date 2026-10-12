import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { type Copy, type Schemas, placeholders } from "../../api-client/src/index.ts";
import { type Model, type Tab, plansView, render, rewardsView, tabs } from "./views.ts";

const repo = join(dirname(fileURLToPath(import.meta.url)), "../../..");

/** The copy files' two-level YAML as dotted keys (the API checks the same files at start-up). */
function copyPack(language: string): Copy {
  const out: Copy = {};
  let section = "";
  for (const raw of readFileSync(join(repo, "config/copy", `${language}.yaml`), "utf8").split(
    "\n",
  )) {
    const line = raw.replace(/\s+$/, "");
    if (!line || line.trimStart().startsWith("#")) {
      continue;
    }
    const top = /^([a-z_]+):$/.exec(line);
    if (top?.[1]) {
      section = top[1];
      continue;
    }
    const entry = /^ {2}([a-z_0-9]+): (.*)$/.exec(line);
    if (entry?.[1] && entry[2] !== undefined) {
      const value = entry[2].startsWith('"') ? (JSON.parse(entry[2]) as string) : entry[2];
      out[`${section}.${entry[1]}`] = value;
    }
  }
  return out;
}

const en = copyPack("en");
const zh = copyPack("zh-Hant");

const registry: Schemas["Registry"] = {
  currency: "HKD",
  utc_offset_minutes: 480,
  default_language: "en",
  languages: ["en", "zh-Hant"],
  data_types: ["photos", "trip_data"],
  evidence_kinds: ["photo"],
  upcoming_event_kinds: ["event.concert", "trip.air"],
  declared_item_kinds: ["device.phone"],
  routes: ["mtr.tsuen_wan"],
  areas: [],
  allocation_step: 500,
  urgent_circumstances: ["injury"],
};

const plans: Schemas["PlanPrice"][] = ["starter", "essential", "plus", "max"].map((id, i) => ({
  plan_id: id,
  list_price: [4_000, 8_000, 11_500, 15_000][i] ?? 0,
  discount: 0,
  payable: [4_000, 8_000, 11_500, 15_000][i] ?? 0,
  promotion_id: null,
  funded_by: null,
}));

const cash: Schemas["RewardOffer"] = {
  offer_id: "cash",
  family: "cash",
  partner_id: null,
  description: "",
  perceived_value: 1_250,
  price: 1_250,
  funded_by: "insurer",
  inventory: null,
  is_cash: true,
};

function model(over: Partial<Model> = {}): Model {
  return {
    copy: en,
    registry,
    language: "en",
    tab: "plans",
    covers: [],
    plans,
    ratingQuestions: [],
    ratingAnswers: {},
    tasks: [],
    payouts: [],
    claims: [],
    decisions: [],
    samples: [],
    live: false,
    me: {
      profile: {
        customer_id: "cust-1",
        occupation: null,
        work_windows: [],
        declared_items: [],
        commute_routes: [],
        work_areas: [],
        upcoming_events: [],
        typical_daily_earnings: null,
        consents: [],
        language: null,
      },
      partner_id: null,
      created_at: "2026-11-18T07:00:00Z",
      cancelled: null,
    },
    policy: null,
    ...over,
  };
}

test("every offered language has the same copy keys and placeholders", () => {
  assert.deepEqual(Object.keys(zh).sort(), Object.keys(en).sort());
  for (const [key, text] of Object.entries(en)) {
    assert.deepEqual(placeholders(zh[key] ?? ""), placeholders(text), key);
  }
});

test("r45 every plan is listed in the API's order, none marked as recommended", () => {
  const html = plansView(model());
  const order = [...html.matchAll(/data-action="quote" data-arg="([a-z]+)"/g)].map((m) => m[1]);
  assert.deepEqual(order, ["starter", "essential", "plus", "max"]);
  assert.doesNotMatch(html, /recommend|popular|best/i);
  const cards = [...html.matchAll(/<li class="([^"]+)">/g)].map((m) => m[1]);
  assert.ok(
    cards.every((c) => c === "plan"),
    "no plan is styled apart",
  );
});

test("r16 cash for the whole balance comes first, as the API sends it", () => {
  const lounge: Schemas["RewardOffer"] = {
    ...cash,
    offer_id: "lounge_day_pass",
    family: "partner",
    partner_id: "airport_lounge_network",
    description: "One airport lounge visit",
    price: 200,
    is_cash: false,
  };
  const html = rewardsView(
    model({ rewards: { balance: 1_250, menu: [cash, lounge], ranked_by: "model", grants: [] } }),
  );
  const offers = [...html.matchAll(/data-action="redeem" data-arg="([a-z_]+)"/g)].map((m) => m[1]);
  assert.deepEqual(offers, ["cash", "lounge_day_pass"]);
  assert.match(html, /HK\$12\.50/);
});

test("every screen renders from the copy pack, in both languages", () => {
  const sections = new Set(Object.keys(en).map((k) => k.split(".")[0]));
  const missing = new RegExp(`\\b(${[...sections].join("|")})\\.[a-z_]+\\b`);
  for (const copy of [en, zh]) {
    for (const tab of [
      "plans",
      "about",
      "tasks",
      "rewards",
      "payouts",
      "claims",
      "privacy",
    ] as Tab[]) {
      const html = render(model({ copy, tab }));
      assert.doesNotMatch(html, missing, `${tab}: a copy key with no text`);
    }
  }
});

test("values from the API are escaped", () => {
  const hostile = model({ tab: "about" });
  if (hostile.me) {
    hostile.me.profile.occupation = `"><script>alert(1)</script>`;
  }
  assert.ok(!render(hostile).includes("<script>alert"));
});

test("a customer with a policy sees their cover, not the plan picker", () => {
  const withPolicy = model({ policy: {} as Schemas["PolicyView"] });
  assert.ok(!tabs(withPolicy).includes("plans"));
  assert.ok(tabs(model()).includes("plans"));
});

test("a market's own declarations and money come from the registry, never from the app", () => {
  // A market whose income cover pays where the customer works: areas to declare, no rail lines,
  // its own currency and allocation step (ADR-012).
  const taiwan = model({
    tab: "about",
    registry: {
      ...registry,
      currency: "TWD",
      routes: [],
      areas: ["tw.new_taipei_city", "tw.taipei_city"],
      allocation_step: 1_000,
    },
  });
  if (taiwan.me) {
    taiwan.me.profile.work_areas = ["tw.taipei_city"];
  }
  const html = render(taiwan);
  assert.match(html, /name="area" value="tw.taipei_city" checked/);
  assert.match(html, /name="area" value="tw.new_taipei_city">/);
  assert.doesNotMatch(html, /name="route"/, "no routes to declare in this market");
  assert.doesNotMatch(render(model({ tab: "about" })), /name="area"/);
});
