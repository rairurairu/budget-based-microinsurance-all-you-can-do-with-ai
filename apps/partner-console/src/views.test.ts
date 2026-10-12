import assert from "node:assert/strict";
import { test } from "node:test";
import { figure, render } from "./views.ts";

test("r21 a withheld figure is shown as withheld, never as a number", () => {
  const withheld = { metric: "payouts", unit: "count" as const, value: null, cohort_size: 12 };
  assert.equal(figure(withheld, 50, "HKD"), "withheld: fewer than 50 customers");
  const html = render({
    partnerId: "delivery_app",
    currency: "HKD",
    results: { partner_id: "delivery_app", min_cohort: 50, figures: [withheld] },
  });
  assert.ok(!html.includes("12 customers"), "nor the size of the cohort behind it");
});

test("figures as a partner reads them", () => {
  assert.equal(
    figure({ metric: "x", unit: "bps", value: 7_500, cohort_size: 60 }, 50, "HKD"),
    "75%",
  );
  assert.equal(
    figure({ metric: "x", unit: "minor", value: 4_800_000, cohort_size: 60 }, 50, "HKD"),
    "HK$48,000.00",
  );
});

test("offers are escaped", () => {
  const html = render({
    partnerId: "p",
    currency: "HKD",
    offers: {
      offers: [
        {
          offer_id: "x",
          family: "partner",
          partner_id: "p",
          description: '"><script>alert(1)</script>',
          perceived_value: 100,
          price: 100,
          funded_by: "partner",
          inventory: 1,
          is_cash: false,
        },
      ],
    },
  });
  assert.ok(!html.includes("<script>alert"));
});
