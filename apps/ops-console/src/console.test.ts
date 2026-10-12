import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { type Snapshot, money, percent, rows } from "./queue.ts";
import { adoption, escapeHtml, render } from "./render.ts";

const app = join(dirname(fileURLToPath(import.meta.url)), "..");
const demo = JSON.parse(readFileSync(join(app, "demo", "snapshot.json"), "utf8")) as Snapshot;

test("r13 only a claim referral shows a decline", () => {
  for (const r of rows(demo)) {
    assert.equal(r.actions.includes("decline"), r.kind === "Claim", r.id);
  }
  const html = render(demo);
  const declines = html.match(/data-action="decline"/g) ?? [];
  const claims = demo.items.filter((i) => i.work.kind === "claim_referral").length;
  assert.equal(declines.length, claims);
});

test("r46 the queue keeps the engine's order, urgent first", () => {
  const ids = rows(demo).map((r) => r.id);
  assert.deepEqual(
    ids,
    demo.items.map((i) => i.item_id),
  );
  assert.equal(rows(demo)[0]?.urgent, true);
});

test("an item past its due time is overdue", () => {
  const late: Snapshot = { ...demo, as_of: "2026-12-31T00:00:00Z" };
  assert.ok(rows(late).every((r) => r.overdue && r.due === "overdue"));
  const first = rows(demo)[0];
  assert.equal(first?.overdue, false);
  assert.match(first?.due ?? "", /^due in \d+ h$/);
});

test("snapshot text is escaped", () => {
  assert.equal(escapeHtml(`<img src=x onerror="a">`), "&lt;img src=x onerror=&quot;a&quot;&gt;");
  const hostile: Snapshot = {
    ...demo,
    items: demo.items.map((i) => ({ ...i, customer_id: "<script>alert(1)</script>" })),
  };
  assert.ok(!render(hostile).includes("<script>alert"));
});

test("money and ratios as people read them", () => {
  assert.equal(money(380_000, "HKD"), "HK$3,800.00");
  assert.equal(money(5, "HKD"), "HK$0.05");
  assert.equal(money(120_000, "TWD"), "NT$1,200.00");
  assert.equal(money(380_000), "3,800.00", "offline, without the market's currency");
  assert.equal(percent(6_667), "66.7%");
});

test("the job queue and the pilot report show what is measured, and what is not yet", () => {
  const html = adoption(
    {
      counts: { done: 12, failed: 1 },
      failed: [
        {
          id: 7,
          kind: "open_policy_period",
          key: "open_policy_period/pol-1/4",
          status: "failed",
          attempts: 5,
          run_at: "2026-11-02T00:00:00Z",
          last_error: "period 3 opens first",
          finished_at: "2026-11-02T05:00:00Z",
        },
      ],
    },
    {
      pilot_id: "hk_2027_typhoon_season",
      starts: "2027-05-01",
      ends: "2027-10-31",
      gates_fixed_on: "2027-04-15",
      from: "2027-05-01",
      to: "2027-10-31",
      as_of: "2027-06-01",
      metrics: [
        { metric: "loss_ratio", unit: "bps", value: 5_500, sample: 120 },
        { metric: "claims_avoided_vs_control", unit: "bps", value: null, sample: 0 },
      ],
      gates: [{ metric: "loss_ratio", kind: "stop", at_least: null, at_most: 0.75 }],
    },
  );
  assert.match(html, /open_policy_period\/pol-1\/4/);
  assert.match(html, /55\.0%/);
  assert.match(html, /at most 0\.75 · stops/);
  assert.match(html, /not measured yet/);
});
