import assert from "node:assert/strict";
import { test } from "node:test";
import type { Schemas } from "../../api-client/src/index.ts";
import { type EmbedModel, view } from "./views.ts";

const copy = {
  "embed.title": "Protection for your work",
  "embed.intro": "Pick a monthly price.",
  "embed.refer": "Continue on the insurer's own site.",
  "embed.refer_link": "Continue to the insurer",
  "embed.accept": "Get covered",
  "embed.powered": "Underwritten by a licensed insurer.",
  "plans.per_month": "{price} a month",
  "app.cancel": "Cancel",
};

const plans: Schemas["PlanPrice"][] = ["starter", "essential"].map((id) => ({
  plan_id: id,
  list_price: 4_000,
  discount: 0,
  payable: 4_000,
  promotion_id: null,
  funded_by: null,
}));

function model(step: EmbedModel["step"]): EmbedModel {
  return { copy, language: "en", currency: "HKD", plans, covers: [], step };
}

test("r52 a referral-only partner shows the way to the insurer, never a way to bind", () => {
  const html = view(model({ kind: "refer", url: "https://insurer.example/journey" }));
  assert.match(html, /href="https:\/\/insurer\.example\/journey"/);
  assert.doesNotMatch(html, /data-accept|data-plan/);
});

test("r45 the plans are listed as sent, none recommended", () => {
  const html = view(model({ kind: "plans" }));
  const order = [...html.matchAll(/data-plan="([a-z]+)"/g)].map((m) => m[1]);
  assert.deepEqual(order, ["starter", "essential"]);
  assert.doesNotMatch(html, /recommend|popular|best/i);
});

test("an error from the API is escaped", () => {
  const html = view(model({ kind: "error", text: "<img src=x onerror=alert(1)>" }));
  assert.ok(!html.includes("<img"));
});
