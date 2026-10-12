// The partner console as pure functions of what the API returns. Results are aggregates only:
// a withheld figure is shown as withheld, never as a number (R21). Offers are the partner's own
// spare capacity, published as partner rewards (R16, R56).
import { type Schemas, escapeHtml, money, percent } from "../../api-client/src/index.ts";
import { FIELD, METRIC, VIEW } from "./copy.ts";

export interface Model {
  partnerId: string;
  currency: string;
  results?: Schemas["PartnerResults"] | undefined;
  /** None when the partner is not a reward supplier. */
  offers?: Schemas["RewardOffers"] | null | undefined;
  flash?: string | undefined;
  demo?: { sessions: { label: string; token: string }[]; current: string } | undefined;
}

/** A figure as a partner may read it. */
export function figure(f: Schemas["Figure"], min: number, currency: string): string {
  if (f.value === null || f.value === undefined) {
    return VIEW.withheld(min);
  }
  switch (f.unit) {
    case "bps":
      return percent(f.value);
    case "minor":
      return money(f.value, currency);
    default:
      return f.value.toLocaleString("en");
  }
}

function results(m: Model): string {
  const r = m.results;
  if (!r) {
    return "";
  }
  const rows = r.figures
    .map((f) => {
      const shown = f.value !== null && f.value !== undefined;
      return `<tr${shown ? "" : ' class="withheld"'}><td>${escapeHtml(METRIC[f.metric] ?? f.metric)}</td><td>${escapeHtml(figure(f, r.min_cohort, m.currency))}</td><td class="muted">${shown ? escapeHtml(VIEW.over(f.cohort_size)) : ""}</td></tr>`;
    })
    .join("");
  return `<section><h2>${escapeHtml(VIEW.results)}</h2><p class="muted">${escapeHtml(VIEW.resultsNote(r.min_cohort))}</p><table><tbody>${rows}</tbody></table></section>`;
}

function offers(m: Model): string {
  if (m.offers === null) {
    return `<section><h2>${escapeHtml(VIEW.offers)}</h2><p class="muted">${escapeHtml(VIEW.notSupplier)}</p></section>`;
  }
  const rows = (m.offers?.offers ?? [])
    .map(
      (o, i) => `<fieldset data-index="${i}">
<legend>${escapeHtml(o.offer_id)}</legend>
<label>${escapeHtml(FIELD.description)} <input name="description" value="${escapeHtml(o.description)}"></label>
<label>${escapeHtml(FIELD.value)} <input type="number" min="0" step="0.01" name="perceived_value" value="${o.perceived_value / 100}"></label>
<label>${escapeHtml(FIELD.price)} <input type="number" min="0.01" step="0.01" name="price" value="${o.price / 100}"></label>
<label>${escapeHtml(FIELD.stock)} <input type="number" min="0" step="1" name="inventory" value="${o.inventory ?? ""}"></label>
</fieldset>`,
    )
    .join("");
  return `<section><h2>${escapeHtml(VIEW.offers)}</h2><p class="muted">${escapeHtml(VIEW.offersNote)}</p>
<form data-form="offers">${rows}<button type="submit">${escapeHtml(VIEW.publish)}</button></form></section>`;
}

function embed(m: Model): string {
  const snippet = [
    `<script type="module" src="https://<insurer api>/embed/js/embed-sdk/src/index.js"></script>`,
    `<microinsurance-embed api="https://<insurer api>" partner="${m.partnerId}" token="<customer token>"></microinsurance-embed>`,
  ].join("\n");
  const exchange = [
    "POST /partners/me/customer-tokens",
    "Authorization: Bearer <partner token>",
    `{ "customer_ref": "<your reference>" }`,
  ].join("\n");
  return `<section><h2>${escapeHtml(VIEW.embed)}</h2><p class="muted">${escapeHtml(VIEW.embedNote)}</p><pre>${escapeHtml(exchange)}</pre><pre>${escapeHtml(snippet)}</pre></section>`;
}

export function render(m: Model): string {
  const demo = m.demo
    ? `<label class="demo">${escapeHtml(VIEW.demoAs)} <select data-change="session">${m.demo.sessions
        .map(
          (s) =>
            `<option value="${escapeHtml(s.token)}"${s.token === m.demo?.current ? " selected" : ""}>${escapeHtml(s.label)}</option>`,
        )
        .join("")}</select></label>`
    : "";
  const flash = m.flash ? `<p class="flash" role="status">${escapeHtml(m.flash)}</p>` : "";
  return `<header><h1>${escapeHtml(VIEW.title)}: ${escapeHtml(m.partnerId)}</h1>${demo}</header>${flash}${results(m)}${offers(m)}${embed(m)}`;
}
