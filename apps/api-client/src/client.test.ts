import assert from "node:assert/strict";
import { test } from "node:test";
import {
  ApiError,
  createClient,
  escapeHtml,
  fill,
  money,
  parseEvent,
  placeholders,
  t,
} from "./index.ts";

function fakeFetch(calls: { url: string; init: RequestInit }[], status = 200, body: unknown = {}) {
  return async (url: string | URL | Request, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    return new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  };
}

test("path parameters are percent-encoded, so ids with slashes reach their route", () => {
  assert.equal(
    fill("/me/prevention-tasks/{task_id}/accept", { task_id: "task/cust-1/screen/CX500" }),
    "/me/prevention-tasks/task%2Fcust-1%2Fscreen%2FCX500/accept",
  );
  assert.equal(
    fill("/plans", undefined, { channel: "delivery_app" }),
    "/plans?channel=delivery_app",
  );
  assert.equal(fill("/plans", undefined, { channel: undefined }), "/plans");
  assert.throws(() => fill("/me/claims/{claim_id}", {}));
});

test("the token goes in the Authorization header, never the URL", async () => {
  const calls: { url: string; init: RequestInit }[] = [];
  const api = createClient({
    baseUrl: "https://api.example",
    token: () => "tok",
    fetch: fakeFetch(calls),
  });
  await api.post("/me/quotes", { body: { plan_id: "essential" } });
  const call = calls[0];
  assert.ok(call);
  assert.equal(call.url, "https://api.example/me/quotes");
  assert.equal((call.init.headers as Record<string, string>).Authorization, "Bearer tok");
  assert.equal(call.init.body, JSON.stringify({ plan_id: "essential" }));
  assert.ok(!call.url.includes("tok"));
});

test("a refusal names its rule", async () => {
  const body = { code: "rule", rule: "R52", detail: "refer to the insurer" };
  const api = createClient({ baseUrl: "", fetch: fakeFetch([], 422, body) });
  await assert.rejects(
    api.post("/me/quotes/{quote_id}/accept", { path: { quote_id: "q-1" } }),
    (e) => {
      assert.ok(e instanceof ApiError);
      assert.equal(e.status, 422);
      assert.equal(e.rule, "R52");
      return true;
    },
  );
});

test("server-sent events are parsed", () => {
  assert.deepEqual(parseEvent('event: payout\ndata: {"a":1}'), { event: "payout", data: { a: 1 } });
  assert.equal(parseEvent(": keep-alive"), null);
});

test("copy templates are filled and escaped", () => {
  const copy = { "plans.per_month": "{price} a month", "x.y": "<b>{name}</b>" };
  assert.equal(t(copy, "plans.per_month", { price: "HK$80.00" }), "HK$80.00 a month");
  assert.equal(t(copy, "x.y", { name: "<script>" }), "&lt;b&gt;&lt;script&gt;&lt;/b&gt;");
  assert.equal(t(copy, "missing.key"), "missing.key");
  assert.deepEqual(placeholders("{b} and {a}"), ["a", "b"]);
  assert.equal(escapeHtml(`"'&`), "&quot;&#39;&amp;");
});

test("money as the engine sends it, in minor units", () => {
  assert.equal(money(8_000, "HKD"), "HK$80.00");
  assert.equal(money(123_456_7, "HKD"), "HK$12,345.67");
  assert.equal(money(-150, "HKD"), "-HK$1.50");
  // Any market's currency, from its ISO code: nothing about Hong Kong is written in the app.
  assert.equal(money(120_000, "TWD"), "NT$1,200.00");
  assert.match(money(120_000, "TWD", "zh-Hant"), /1,200.00/);
  assert.equal(money(500, "XYZ1"), "XYZ1 5.00");
});
