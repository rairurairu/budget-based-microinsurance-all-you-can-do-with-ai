// The embed's screens as pure functions: a partner's channel either binds cover (a licensed
// partner) or refers the customer to the insurer's own journey (R52). Plans are shown as the API
// lists them, none recommended (R45); a quote shows each cover with its sentence (R23); one tap
// accepts (R03). Every value is escaped; every word comes from the copy pack.
import { type Copy, type Schemas, day, escapeHtml, money, t } from "../../api-client/src/index.ts";

export type Step =
  | { kind: "loading" }
  | { kind: "refer"; url: string }
  | { kind: "plans" }
  | { kind: "quote"; quote: Schemas["Quote"] }
  | { kind: "covered"; starts: string; policy: Schemas["Policy"] }
  | { kind: "error"; text: string };

export interface EmbedModel {
  copy: Copy;
  language: string;
  currency: string;
  plans: Schemas["PlanPrice"][];
  covers: Schemas["CoverInfo"][];
  step: Step;
}

function coverName(m: EmbedModel, id: string): string {
  return m.copy[`covers.${id}`] ? t(m.copy, `covers.${id}`) : escapeHtml(id);
}

function covers(
  m: EmbedModel,
  schedule: Schemas["Schedule"],
  explanations: Schemas["CoverExplanation"][],
): string {
  return schedule.allocations
    .map((a) => {
      const info = m.covers.find((c) => c.cover_id === a.cover_id);
      const amount = money(a.sum_insured, m.currency);
      const line =
        info?.cover_type === "parametric"
          ? t(m.copy, info.daily_benefit ? "quote.benefit_per_day" : "quote.benefit_per_event", {
              amount,
            })
          : t(m.copy, "quote.sum_insured", { amount });
      const sentence = explanations.find((e) => e.cover_id === a.cover_id)?.sentence ?? "";
      return `<li><strong>${coverName(m, a.cover_id)}</strong><div>${line}</div><p>${escapeHtml(sentence)}</p></li>`;
    })
    .join("");
}

export function view(m: EmbedModel): string {
  const c = m.copy;
  const head = `<h2>${t(c, "embed.title")}</h2>`;
  const foot = `<p class="powered">${t(c, "embed.powered")}</p>`;
  switch (m.step.kind) {
    case "loading":
      return `<p>${t(c, "app.loading")}</p>`;
    case "error":
      return `${head}<p class="error" role="alert">${escapeHtml(m.step.text)}</p>`;
    case "refer":
      return `${head}<p>${t(c, "embed.refer")}</p><a class="primary" href="${escapeHtml(m.step.url)}" target="_blank" rel="noopener">${t(c, "embed.refer_link")}</a>${foot}`;
    case "plans": {
      const plans = m.plans
        .map(
          (p) =>
            `<li><button type="button" data-plan="${escapeHtml(p.plan_id)}"><span>${escapeHtml(p.plan_id)}</span> <strong>${t(c, "plans.per_month", { price: money(p.payable, m.currency) })}</strong></button></li>`,
        )
        .join("");
      return `${head}<p>${t(c, "embed.intro")}</p><ul class="plans">${plans}</ul>${foot}`;
    }
    case "quote": {
      const q = m.step.quote;
      const outcome = q.suggestion?.outcome;
      // R42: part of the price buys cover the customer cannot use yet; they are told.
      const asked =
        outcome && (outcome.placed_beyond_need > 0 || outcome.unallocated > 0)
          ? `<p class="note">${t(c, "quote.update_asked")}</p>`
          : "";
      return `${head}<p>${t(c, "quote.starts", { date: day(q.period_starts, m.language) })}</p>
<ul class="covers">${covers(m, q.schedule, q.explanations)}</ul>${asked}
<div class="row"><button type="button" data-back="1">${t(c, "app.cancel")}</button>
<button type="button" class="primary" data-accept="${escapeHtml(q.offer.offer_id)}">${t(c, "embed.accept")}</button></div>${foot}`;
    }
    case "covered":
      return `${head}<p class="ok">${t(c, "embed.done", { date: day(m.step.starts, m.language) })}</p>
<ul class="covers">${covers(m, m.step.policy.schedule, [])}</ul>${foot}`;
  }
}

export const STYLE = `
:host { display: block; font: 15px/1.5 system-ui, -apple-system, "Segoe UI", "PingFang HK", sans-serif;
  --mi-bg: #ffffff; --mi-fg: #1c1917; --mi-muted: #6b6560; --mi-line: #e4e1dc; --mi-accent: #0f766e; --mi-accent-fg: #fff; }
@media (prefers-color-scheme: dark) { :host { --mi-bg: #211f1d; --mi-fg: #f5f4f2; --mi-muted: #a8a29e; --mi-line: #3a3532; --mi-accent: #2dd4bf; --mi-accent-fg: #0b1513; } }
.box { background: var(--mi-bg); color: var(--mi-fg); border: 1px solid var(--mi-line); border-radius: 14px; padding: 16px; }
h2 { font-size: 17px; margin: 0 0 6px; } p { margin: 6px 0; } ul { list-style: none; padding: 0; margin: 10px 0; }
li { border-top: 1px solid var(--mi-line); padding: 8px 0; } .plans li { border: 0; padding: 4px 0; }
button, a.primary { font: inherit; padding: 9px 14px; border-radius: 10px; border: 1px solid var(--mi-line); background: transparent; color: var(--mi-fg); cursor: pointer; text-decoration: none; display: inline-block; }
.plans button { width: 100%; display: flex; justify-content: space-between; text-transform: capitalize; }
.primary { background: var(--mi-accent) !important; color: var(--mi-accent-fg) !important; border-color: var(--mi-accent) !important; }
.row { display: flex; gap: 8px; justify-content: flex-end; flex-wrap: wrap; }
.powered { color: var(--mi-muted); font-size: 12px; } .error { color: #b91c1c; } .ok { font-weight: 600; } .note { color: #b45309; font-size: 14px; }
`;
