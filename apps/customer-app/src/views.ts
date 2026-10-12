// The customer app's screens as pure functions of the model: HTML strings, every value from the
// API escaped, every word from the customer's copy pack (config/copy). Buttons name an action
// (`data-action`) the runtime in main.ts performs through the API; the screens add no rule of
// their own: the plans are listed as the API sends them and none is marked as recommended
// (R45); cash comes first on the rewards menu because the API puts it there (R16); a payout's
// alternatives are the ones the engine offered (R28).
import {
  type Copy,
  type Schemas,
  day,
  escapeHtml,
  money,
  percent,
  t,
} from "../../api-client/src/index.ts";

export type Tab =
  | "plans"
  | "schedule"
  | "about"
  | "tasks"
  | "rewards"
  | "payouts"
  | "claims"
  | "privacy";

export interface Session {
  label: string;
  token: string;
}

export interface Model {
  copy: Copy;
  registry: Schemas["Registry"];
  language: string;
  tab: Tab;
  covers: Schemas["CoverInfo"][];
  plans: Schemas["PlanPrice"][];
  me?: Schemas["CustomerRecord"] | undefined;
  policy?: Schemas["PolicyView"] | null;
  quote?: Schemas["Quote"] | undefined;
  renewal?: Schemas["RenewalView"] | undefined;
  ratingQuestions: Schemas["RatingQuestion"][];
  ratingAnswers: Record<string, string>;
  tasks: Schemas["PreventionTask"][];
  score?: Schemas["ScoreView"] | undefined;
  rewards?: Schemas["RewardsView"] | null;
  payouts: Schemas["PayoutNotice"][];
  claims: Schemas["ClaimRecord"][];
  decisions: Schemas["DecisionRecord"][];
  /** The evidence images demo mode offers. */
  samples: string[];
  /** HTML, made by `t` from the copy pack, which escapes every value it fills in. */
  flash?: { kind: "ok" | "error"; text: string } | undefined;
  live: boolean;
  demo?: { sessions: Session[]; current: string } | undefined;
}

const TABS: Tab[] = [
  "plans",
  "schedule",
  "about",
  "tasks",
  "rewards",
  "payouts",
  "claims",
  "privacy",
];

function m(model: Model, minor: number): string {
  return escapeHtml(money(minor, model.registry.currency, model.language));
}

function d(model: Model, iso: string): string {
  return escapeHtml(day(iso, model.language));
}

function coverName(model: Model, id: string): string {
  const key = `covers.${id}`;
  return model.copy[key] ? t(model.copy, key) : escapeHtml(id);
}

function button(action: string, label: string, arg = "", kind = ""): string {
  return `<button type="button" class="${kind}" data-action="${escapeHtml(action)}" data-arg="${escapeHtml(arg)}">${label}</button>`;
}

// ---- header and navigation

function header(model: Model): string {
  const c = model.copy;
  const languages = model.registry.languages
    .map(
      (l) =>
        `<option value="${escapeHtml(l)}"${l === model.language ? " selected" : ""}>${escapeHtml(l)}</option>`,
    )
    .join("");
  const demo = model.demo
    ? `<label class="demo">${t(c, "app.demo_as")} <select data-change="session">${model.demo.sessions
        .map(
          (s) =>
            `<option value="${escapeHtml(s.token)}"${s.token === model.demo?.current ? " selected" : ""}>${escapeHtml(s.label)}</option>`,
        )
        .join("")}</select></label><p class="note">${t(c, "app.demo_note")}</p>`
    : "";
  return `<header>
<div><h1>${t(c, "app.title")}</h1><p class="muted">${t(c, "app.tagline")}</p></div>
<label>${t(c, "app.language")} <select data-change="language">${languages}</select></label>
${demo}
</header>`;
}

export function tabs(model: Model): Tab[] {
  return TABS.filter((tab) =>
    tab === "plans" ? !model.policy : tab !== "schedule" || !!model.policy,
  );
}

function nav(model: Model): string {
  return `<nav>${tabs(model)
    .map(
      (tab) =>
        `<button type="button" data-action="tab" data-arg="${tab}"${tab === model.tab ? ' aria-current="page"' : ""}>${t(model.copy, `nav.${tab}`)}</button>`,
    )
    .join("")}</nav>`;
}

function flash(model: Model): string {
  if (!model.flash) {
    return "";
  }
  return `<p class="flash ${model.flash.kind}" role="status">${model.flash.text}</p>`;
}

// ---- plans and quotes

/** Every plan, in the API's order, the same for everyone; none is marked as recommended (R45). */
export function plansView(model: Model): string {
  const c = model.copy;
  const cards = model.plans
    .map((p) => {
      const promo =
        p.promotion_id && p.discount > 0
          ? `<p class="promo">${t(c, "plans.promotion", { discount: money(p.discount, model.registry.currency, model.language), promotion: p.promotion_id })}</p>`
          : "";
      return `<li class="plan">
<h3>${escapeHtml(p.plan_id)}</h3>
<p class="price">${t(c, "plans.per_month", { price: money(p.payable, model.registry.currency, model.language) })}</p>
${promo}
${button("quote", t(c, "plans.choose"), p.plan_id)}
</li>`;
    })
    .join("");
  return `<section><h2>${t(c, "plans.title")}</h2><p class="muted">${t(c, "plans.intro")}</p>
<ul class="plans">${cards}</ul>
${model.quote ? quoteView(model, model.quote) : ""}</section>`;
}

function benefitLine(model: Model, coverId: string, sumInsured: number): string {
  const cover = model.covers.find((c) => c.cover_id === coverId);
  const amount = money(sumInsured, model.registry.currency, model.language);
  if (cover?.cover_type === "parametric") {
    return t(
      model.copy,
      cover.daily_benefit ? "quote.benefit_per_day" : "quote.benefit_per_event",
      { amount },
    );
  }
  return t(model.copy, "quote.sum_insured", { amount });
}

function scheduleLines(
  model: Model,
  schedule: Schemas["Schedule"],
  explanations: Schemas["CoverExplanation"][],
): string {
  return schedule.allocations
    .map((a) => {
      const sentence = explanations.find((e) => e.cover_id === a.cover_id)?.sentence ?? "";
      return `<li class="cover">
<div><strong>${coverName(model, a.cover_id)}</strong> <span class="muted">${t(model.copy, "quote.premium", { amount: money(a.premium, model.registry.currency, model.language) })}</span></div>
<div class="si">${benefitLine(model, a.cover_id, a.sum_insured)}</div>
<p>${escapeHtml(sentence)}</p>
</li>`;
    })
    .join("");
}

/** The factors behind the suggestion, as shown and kept (R49). */
function why(model: Model, quote: Schemas["Quote"]): string {
  const basis = quote.suggestion?.basis ?? [];
  if (basis.length === 0) {
    return "";
  }
  const lines = basis
    .map((b) => {
      const need =
        b.need > 0
          ? t(model.copy, "quote.need", {
              amount: money(b.need, model.registry.currency, model.language),
            })
          : t(model.copy, "quote.no_need");
      return `<li>${coverName(model, b.cover_id)}: ${need}</li>`;
    })
    .join("");
  return `<details><summary>${t(model.copy, "quote.why")}</summary><ul>${lines}</ul></details>`;
}

/** A quote with its split editable: the customer may move premium between covers (R34). */
export function quoteView(model: Model, quote: Schemas["Quote"]): string {
  const c = model.copy;
  const change = quote.offer.change;
  const plan = change.kind === "plan_change" ? change.plan : quote.schedule.plan_id;
  const outcome = quote.suggestion?.outcome;
  const asked =
    outcome && (outcome.placed_beyond_need > 0 || outcome.unallocated > 0)
      ? `<p class="note">${t(c, "quote.update_asked")}</p>`
      : "";
  const split = quote.schedule.allocations
    .map(
      (a) =>
        `<label>${coverName(model, a.cover_id)} <input type="number" min="0" step="${model.registry.allocation_step / 100}" name="${escapeHtml(a.cover_id)}" value="${a.premium / 100}"></label>`,
    )
    .join("");
  const accept = change.kind === "new_policy" ? t(c, "quote.accept") : t(c, "quote.accept_change");
  return `<section class="quote">
<h2>${t(c, "quote.title", { plan })}</h2>
<p class="muted">${t(c, "quote.starts", { date: day(quote.period_starts, model.language) })}</p>
${change.kind === "plan_change" ? `<p>${t(c, "quote.plan_change")}</p>` : ""}
<ul class="covers">${scheduleLines(model, quote.schedule, quote.explanations)}</ul>
${asked}
${why(model, quote)}
<details><summary>${t(c, "quote.own_split")}</summary>
<form data-form="split" data-arg="${escapeHtml(quote.schedule.plan_id)}">${split}<button type="submit">${t(c, "quote.price_split")}</button></form>
</details>
${button("accept", accept, quote.offer.offer_id, "primary")}
</section>`;
}

// ---- the policy

function premiumLine(model: Model, premium: Schemas["PremiumStatus"]): string {
  switch (premium.status) {
    case "paid":
      return t(model.copy, "schedule.paid");
    case "in_grace":
      return t(model.copy, "schedule.in_grace", { date: day(premium.last_day, model.language) });
    case "lapsed":
      return t(model.copy, "schedule.lapsed", { date: day(premium.reinstate_by, model.language) });
  }
}

function renewalView(model: Model, r: Schemas["RenewalView"]): string {
  const c = model.copy;
  const lines = r.notice.changes
    .map((ch) => {
      const sentence = r.sentences.find((s) => s.cover_id === ch.cover_id)?.sentence ?? "";
      const below = ch.below_declared_value
        ? `<p class="note">${t(c, "schedule.below_value")}</p>`
        : "";
      return `<li class="cover"><strong>${coverName(model, ch.cover_id)}</strong>
<div class="si">${m(model, ch.before)} → ${m(model, ch.after)}</div><p>${escapeHtml(sentence)}</p>${below}</li>`;
    })
    .join("");
  const update = r.notice.update_request
    ? `<p class="note">${t(c, "schedule.update_by", { date: day(r.notice.update_request.respond_by, model.language) })}</p>`
    : "";
  return `<section><h3>${t(c, "schedule.renewal")}: ${d(model, r.notice.period_starts)}</h3><ul class="covers">${lines}</ul>${update}</section>`;
}

export function scheduleView(model: Model): string {
  const c = model.copy;
  const view = model.policy;
  if (!view) {
    return `<section><p>${t(c, "schedule.none")}</p></section>`;
  }
  const p = view.policy;
  const next = view.endorsements.length
    ? `<p class="note">${t(c, "schedule.next")}: ${view.endorsements
        .map((e) => escapeHtml(e.change.kind.replaceAll("_", " ")))
        .join(", ")}</p>`
    : "";
  const plan = model.plans
    .map((pl) =>
      button(
        "quote",
        escapeHtml(pl.plan_id),
        pl.plan_id,
        pl.plan_id === p.plan.id ? "current" : "",
      ),
    )
    .join("");
  return `<section>
<h2>${t(c, "schedule.title")}: ${escapeHtml(p.plan.id)}</h2>
<p class="muted">${t(c, "schedule.period", { from: day(p.period.starts, model.language), to: day(p.period.ends, model.language) })} · ${premiumLine(model, view.premium)}</p>
<ul class="covers">${scheduleLines(model, p.schedule, view.explanations)}</ul>
${next}
<div class="row">${button("renewal", t(c, "schedule.renewal_check"))}</div>
${model.renewal ? renewalView(model, model.renewal) : ""}
<h3>${t(c, "nav.plans")}</h3><div class="row">${plan}</div>
${model.quote ? quoteView(model, model.quote) : ""}
</section>`;
}

// ---- about you

function kindOptions(kinds: string[], selected = ""): string {
  return kinds
    .map(
      (k) =>
        `<option value="${escapeHtml(k)}"${k === selected ? " selected" : ""}>${escapeHtml(k)}</option>`,
    )
    .join("");
}

export function aboutView(model: Model): string {
  const c = model.copy;
  const p = model.me?.profile;
  if (!p) {
    return "";
  }
  const days = new Set(p.work_windows.map((w) => w.weekday));
  const first = p.work_windows[0];
  const hhmm = (minute: number) =>
    `${String(Math.floor(minute / 60)).padStart(2, "0")}:${String(minute % 60).padStart(2, "0")}`;
  const weekdays = [1, 2, 3, 4, 5, 6, 7]
    .map(
      (w) =>
        `<label class="day"><input type="checkbox" name="weekday" value="${w}"${days.has(w) ? " checked" : ""}>${t(c, `about.weekday_${w}`)}</label>`,
    )
    .join("");
  const events = p.upcoming_events
    .map(
      (e, i) =>
        `<li>${escapeHtml(e.kind)} · ${d(model, e.starts)}${e.reference ? ` · ${escapeHtml(e.reference)}` : ""}${e.value ? ` · ${m(model, e.value)}` : ""} ${button("remove-event", t(c, "about.remove"), String(i), "link")}</li>`,
    )
    .join("");
  const items = p.declared_items
    .map(
      (it, i) =>
        `<li>${escapeHtml(it.kind)} · ${escapeHtml(it.description)}${it.declared_value ? ` · ${m(model, it.declared_value)}` : ""} ${button("remove-item", t(c, "about.remove"), String(i), "link")}</li>`,
    )
    .join("");
  // Routes and work areas are whatever the market's data sources report (ADR-012); a market
  // without either shows no field for it.
  const choices = (name: string, legend: string, ids: string[], held: string[]) =>
    ids.length === 0
      ? ""
      : `<fieldset><legend>${t(c, legend)}</legend>${ids
          .map(
            (id) =>
              `<label class="day"><input type="checkbox" name="${name}" value="${escapeHtml(id)}"${held.includes(id) ? " checked" : ""}>${escapeHtml(id)}</label>`,
          )
          .join("")}</fieldset>`;
  const routes = choices("route", "about.routes", model.registry.routes, p.commute_routes);
  const areas = choices("area", "about.areas", model.registry.areas, p.work_areas ?? []);
  const questions = model.ratingQuestions
    .map((q) => {
      const options = [`<option value="">${t(c, "about.no_answer")}</option>`]
        .concat(
          q.answers.map(
            (a) =>
              `<option value="${escapeHtml(a)}"${model.ratingAnswers[q.factor_id] === a ? " selected" : ""}>${escapeHtml(a)}</option>`,
          ),
        )
        .join("");
      return `<label>${escapeHtml(q.factor_id.replaceAll("_", " "))} <select name="${escapeHtml(q.factor_id)}">${options}</select></label>`;
    })
    .join("");
  return `<section>
<h2>${t(c, "about.title")}</h2><p class="muted">${t(c, "about.intro")}</p>
<form data-form="profile">
<label>${t(c, "about.occupation")} <input name="occupation" value="${escapeHtml(p.occupation ?? "")}"></label>
<label>${t(c, "about.earnings")} <input type="number" min="0" name="earnings" value="${p.typical_daily_earnings ? p.typical_daily_earnings / 100 : ""}"></label>
<fieldset><legend>${t(c, "about.work")}</legend>${weekdays}
<label>${t(c, "about.from")} <input type="time" name="start" value="${first ? hhmm(first.start_minute) : "09:00"}"></label>
<label>${t(c, "about.to")} <input type="time" name="end" value="${first ? hhmm(first.end_minute) : "18:00"}"></label></fieldset>
${routes}${areas}
<button type="submit">${t(c, "app.save")}</button>
</form>
<h3>${t(c, "about.events")}</h3><ul>${events}</ul>
<form data-form="event" class="inline">
<label>${t(c, "about.event_kind")} <select name="kind">${kindOptions(model.registry.upcoming_event_kinds)}</select></label>
<label>${t(c, "about.event_date")} <input type="date" name="starts" required></label>
<label>${t(c, "about.event_reference")} <input name="reference"></label>
<label>${t(c, "about.event_value")} <input type="number" min="0" name="value"></label>
<button type="submit">${t(c, "about.add_event")}</button>
</form>
<h3>${t(c, "about.items")}</h3><ul>${items}</ul>
<form data-form="item" class="inline">
<label>${t(c, "about.item_kind")} <select name="kind">${kindOptions(model.registry.declared_item_kinds)}</select></label>
<label>${t(c, "about.item_description")} <input name="description" required></label>
<label>${t(c, "about.item_value")} <input type="number" min="0" name="value"></label>
<button type="submit">${t(c, "about.add_item")}</button>
</form>
${
  questions
    ? `<h3>${t(c, "about.rating")}</h3><p class="muted">${t(c, "about.rating_note")}</p><form data-form="rating">${questions}<button type="submit">${t(c, "app.save")}</button></form>`
    : ""
}
</section>`;
}

// ---- prevention and rewards

function taskState(model: Model, task: Schemas["PreventionTask"]): string {
  return t(model.copy, `tasks.state_${task.state.state}`);
}

export function tasksView(model: Model): string {
  const c = model.copy;
  const samples = model.samples
    .map((s) => `<option value="${escapeHtml(s)}">${escapeHtml(s)}</option>`)
    .join("");
  const list = model.tasks
    .map((task) => {
      const s = task.state.state;
      const actions: string[] = [];
      if (s === "offered") {
        actions.push(button("task-accept", t(c, "tasks.accept"), task.task_id, "primary"));
      }
      if (s === "accepted" || s === "not_verified") {
        actions.push(`<form data-form="task-evidence" data-arg="${escapeHtml(task.task_id)}" class="inline">
<input type="file" name="file" accept="image/*">
${samples ? `<select name="sample"><option value="">${t(c, "tasks.sample")}</option>${samples}</select>` : ""}
<button type="submit">${t(c, "tasks.evidence")}</button></form>`);
      }
      if (s === "not_verified") {
        actions.push(button("task-review", t(c, "tasks.review"), task.task_id));
      }
      const reason =
        task.state.state === "not_verified"
          ? `<p class="note">${escapeHtml(task.state.reason)}</p>`
          : "";
      return `<li class="task">
<div><strong>${escapeHtml(task.draft.action)}</strong></div>
<div class="muted">${t(c, "tasks.reward", { amount: money(task.draft.reward_cost, model.registry.currency, model.language) })} · ${t(c, "tasks.due", { date: day(task.due, model.language) })} · ${taskState(model, task)}</div>
${reason}${actions.join("")}
</li>`;
    })
    .join("");
  const pools = (model.score?.pools ?? [])
    .map(
      (p) =>
        `<li><strong>${escapeHtml(p.pool)}</strong>: ${t(c, "score.points", { points: p.points })} · ${t(c, "score.effect", { effect: percent(p.effect) })}</li>`,
    )
    .join("");
  return `<section>
<h2>${t(c, "tasks.title")}</h2><p class="muted">${t(c, "tasks.intro")}</p>
${button("tasks-refresh", t(c, "tasks.refresh"))}
${list ? `<ul class="tasks">${list}</ul>` : `<p>${t(c, "tasks.none")}</p>`}
<h3>${t(c, "score.title")}</h3>${pools ? `<ul>${pools}</ul>` : `<p class="muted">${t(c, "score.none")}</p>`}
</section>`;
}

/** The reward balance and its menu in the API's order: cash first, always (R16). */
export function rewardsView(model: Model): string {
  const c = model.copy;
  const r = model.rewards;
  if (!r || r.balance <= 0) {
    return `<section><h2>${t(c, "rewards.title")}</h2><p>${t(c, "rewards.empty")}</p></section>`;
  }
  const menu = r.menu
    .map((o) => {
      const label = o.is_cash
        ? t(c, "rewards.cash", { amount: money(o.price, model.registry.currency, model.language) })
        : `${escapeHtml(o.description)} <span class="muted">${m(model, o.price)}</span>`;
      return `<li class="offer${o.is_cash ? " cash" : ""}">${label} ${button("redeem", t(c, "rewards.take"), o.offer_id)}</li>`;
    })
    .join("");
  return `<section><h2>${t(c, "rewards.title")}</h2><p class="balance">${t(c, "rewards.balance", { amount: money(r.balance, model.registry.currency, model.language) })}</p><ul class="menu">${menu}</ul></section>`;
}

// ---- recovery

function option(model: Model, s: Schemas["Settlement"]): string {
  return s.method === "cash"
    ? t(model.copy, "payouts.cash")
    : t(model.copy, "payouts.in_kind", { option: s.option.replaceAll("_", " ") });
}

export function payoutsView(model: Model): string {
  const c = model.copy;
  const list = model.payouts
    .map((p) => {
      const amount = money(p.decision.amount, model.registry.currency, model.language);
      const covers = p.decision.lines.map((l) => coverName(model, l.cover_id)).join(", ");
      const stage = t(c, `payouts.stage_${p.decision.stage}`);
      let body: string;
      if (p.settled) {
        const setOff =
          p.settled.premium_set_off > 0
            ? `<p class="note">${t(c, "payouts.set_off", { amount: money(p.settled.premium_set_off, model.registry.currency, model.language) })}</p>`
            : "";
        body = `<p>${t(c, "payouts.paid", { amount })} · ${option(model, p.settled.method)}</p>${setOff}`;
      } else {
        const choices = p.decision.options
          .map((o) =>
            button(
              "payout-choice",
              option(model, o),
              JSON.stringify({ key: p.decision.idempotency_key, settlement: o }),
              o.method === "cash" ? "primary" : "",
            ),
          )
          .join("");
        body = `<p>${t(c, "payouts.choose", { amount })}</p><div class="row">${choices}</div>`;
      }
      return `<li class="payout"><strong>${covers}</strong> <span class="muted">${stage} · ${d(model, p.decided_at)}</span>${body}</li>`;
    })
    .join("");
  return `<section><h2>${t(c, "payouts.title")} ${model.live ? `<span class="live">${t(c, "payouts.live")}</span>` : ""}</h2>
<p class="muted">${t(c, "payouts.intro")}</p>
${list ? `<ul class="payouts">${list}</ul>` : `<p>${t(c, "payouts.none")}</p>`}</section>`;
}

function claimState(model: Model, r: Schemas["ClaimRecord"]): string {
  const s = r.claim.state;
  if (s.state === "approved") {
    return t(model.copy, "claims.state_approved", {
      amount: money(s.amount, model.registry.currency, model.language),
    });
  }
  return t(model.copy, `claims.state_${s.state}`);
}

function claimCard(model: Model, r: Schemas["ClaimRecord"]): string {
  const c = model.copy;
  const s = r.claim.state;
  const parts: string[] = [];
  const first = r.triage.first_response;
  if (first) {
    parts.push(`<p class="urgent">${t(c, "claims.urgent")}</p>`);
    if (first.guarantee) {
      parts.push(
        `<p>${t(c, "claims.guarantee_auto", { amount: money(first.guarantee.automatic, model.registry.currency, model.language) })}</p>`,
      );
      if (first.guarantee.to_confirm > 0) {
        parts.push(
          `<p>${t(c, "claims.guarantee_confirm", { amount: money(first.guarantee.to_confirm, model.registry.currency, model.language) })}</p>`,
        );
      }
    }
  }
  if (r.triage.person === "offered") {
    parts.push(`<p class="muted">${t(c, "claims.person_offered")}</p>`);
  }
  if (s.state === "awaiting_evidence") {
    const kinds = s.missing
      .map(
        (k) =>
          `<label class="day"><input type="checkbox" name="kind" value="${escapeHtml(k)}" checked>${escapeHtml(k)}</label>`,
      )
      .join("");
    const samples = model.samples
      .map((x) => `<option value="${escapeHtml(x)}">${escapeHtml(x)}</option>`)
      .join("");
    parts.push(`<form data-form="claim-evidence" data-arg="${escapeHtml(r.claim.claim_id)}">
<fieldset><legend>${t(c, "claims.evidence_kinds")}</legend>${kinds}</fieldset>
<input type="file" name="file" accept="image/*,text/plain">
${samples ? `<select name="sample"><option value="">${t(c, "tasks.sample")}</option>${samples}</select>` : ""}
<button type="submit">${t(c, "claims.evidence")}</button></form>`);
  }
  if (r.payment && !r.settled) {
    const amount = money(r.payment.amount, model.registry.currency, model.language);
    const choices = r.payment.options
      .map((o) =>
        button(
          "claim-choice",
          option(model, o),
          JSON.stringify({ claim: r.claim.claim_id, settlement: o }),
          o.method === "cash" ? "primary" : "",
        ),
      )
      .join("");
    parts.push(`<p>${t(c, "claims.choose", { amount })}</p><div class="row">${choices}</div>`);
  }
  if (s.state === "declined") {
    const decision = model.decisions.find(
      (x) => x.subject_id === r.claim.claim_id && x.decision.startsWith("declined"),
    );
    if (decision) {
      parts.push(button("review", t(c, "claims.review"), decision.decision_id));
    }
  }
  return `<li class="claim"><strong>${coverName(model, r.claim.cover_id)}</strong> · ${m(model, r.claim.amount_claimed)} · ${d(model, r.claim.loss_date)}
<div class="muted">${claimState(model, r)}</div>${parts.join("")}</li>`;
}

export function claimsView(model: Model): string {
  const c = model.copy;
  const indemnity = model.policy
    ? model.policy.policy.schedule.allocations
        .map((a) => a.cover_id)
        .filter((id) => model.covers.find((cv) => cv.cover_id === id)?.cover_type !== "parametric")
    : [];
  const covers = indemnity
    .map((id) => `<option value="${escapeHtml(id)}">${coverName(model, id)}</option>`)
    .join("");
  const circumstances = [`<option value="">${t(c, "claims.circumstance_none")}</option>`]
    .concat(
      model.registry.urgent_circumstances.map(
        (x) => `<option value="${escapeHtml(x)}">${escapeHtml(x.replaceAll("_", " "))}</option>`,
      ),
    )
    .join("");
  const form = covers
    ? `<details><summary>${t(c, "claims.new")}</summary><form data-form="claim">
<label>${t(c, "claims.cover")} <select name="cover_id">${covers}</select></label>
<label>${t(c, "claims.date")} <input type="date" name="loss_date" required></label>
<label>${t(c, "claims.amount")} <input type="number" min="1" step="0.01" name="amount" required></label>
<label>${t(c, "claims.description")} <textarea name="description" required></textarea></label>
<label>${t(c, "claims.circumstance")} <select name="circumstance">${circumstances}</select></label>
<label>${t(c, "claims.guarantee")} <input type="number" min="0" name="guarantee"></label>
<label class="day"><input type="checkbox" name="person"> ${t(c, "claims.person")}</label>
<button type="submit">${t(c, "claims.submit")}</button></form></details>`
    : "";
  const list = model.claims.map((r) => claimCard(model, r)).join("");
  return `<section><h2>${t(c, "claims.title")}</h2><p class="muted">${t(c, "claims.intro")}</p>${form}
${list ? `<ul class="claims">${list}</ul>` : `<p>${t(c, "claims.none")}</p>`}
${decisionsView(model)}</section>`;
}

/** Decisions about the customer, each of which they may put to a person (R15). */
export function decisionsView(model: Model): string {
  const c = model.copy;
  const reviewable = model.decisions.filter(
    (x) => x.actor !== "customer" && x.subject_type !== "claim",
  );
  if (reviewable.length === 0) {
    return "";
  }
  const list = reviewable
    .map(
      (x) =>
        `<li><span>${escapeHtml(x.subject_type)} · ${escapeHtml(x.decision)} · ${d(model, x.at)}</span> ${button("review", t(c, "decisions.review"), x.decision_id, "link")}</li>`,
    )
    .join("");
  return `<h3>${t(c, "decisions.title")}</h3><ul class="decisions">${list}</ul>`;
}

// ---- privacy

export function privacyView(model: Model): string {
  const c = model.copy;
  const me = model.me;
  if (!me) {
    return "";
  }
  if (me.cancelled) {
    return `<section><h2>${t(c, "privacy.title")}</h2><p>${t(c, "privacy.cancelled", { date: day(me.cancelled.retain_records_until, model.language) })}</p></section>`;
  }
  const consents = model.registry.data_types
    .map(
      (d) =>
        `<label class="day"><input type="checkbox" name="consent" value="${escapeHtml(d)}"${me.profile.consents.includes(d) ? " checked" : ""}>${t(c, `privacy.data_${d}`)}</label>`,
    )
    .join("");
  return `<section><h2>${t(c, "privacy.title")}</h2><p class="muted">${t(c, "privacy.intro")}</p>
<form data-form="consents"><fieldset>${consents}</fieldset><button type="submit">${t(c, "app.save")}</button></form>
<div class="row">${button("identity", t(c, "privacy.identity"))} ${button("cancel", t(c, "privacy.cancel"), "", "danger")}</div>
</section>`;
}

const SCREENS: Record<Tab, (model: Model) => string> = {
  plans: plansView,
  schedule: scheduleView,
  about: aboutView,
  tasks: tasksView,
  rewards: rewardsView,
  payouts: payoutsView,
  claims: claimsView,
  privacy: privacyView,
};

export function render(model: Model): string {
  return `${header(model)}${nav(model)}${flash(model)}<main>${SCREENS[model.tab](model)}</main>`;
}
