// The console page from a snapshot: as one document for the offline snapshot, or as the page
// body the live console (main.ts) renders from the API's queue. Every value is escaped. Buttons
// carry the item id and the action; live, each item also carries the fields its actions need: a
// decline reason from the closed list (only where a decline is offered, R13), the loss assessed
// for an approval (R48), the evidence asked for, and a note.
import type { Schemas } from "../../api-client/src/index.ts";
import { ACTION, ADOPTION, DECLINE_REASON, LIVE, VIEW } from "./copy.ts";
import { type Row, type Snapshot, percent, rows } from "./queue.ts";

const ENTITIES: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ENTITIES[c] ?? c);
}

function fields(r: Row, currency: string): string {
  const parts: string[] = [];
  if (r.actions.includes("decline")) {
    const reasons = Object.entries(DECLINE_REASON)
      .map(([id, label]) => `<option value="${id}">${escapeHtml(label)}</option>`)
      .join("");
    parts.push(
      `<label>${escapeHtml(LIVE.reason)} <select data-field="decline_reason">${reasons}</select></label>`,
    );
  }
  if (r.actions.includes("approve")) {
    parts.push(
      `<label>${escapeHtml(LIVE.amount(currency))} <input type="number" min="0.01" step="0.01" data-field="amount"></label>`,
    );
  }
  if (r.actions.includes("request_evidence")) {
    parts.push(`<label>${escapeHtml(LIVE.kinds)} <input data-field="evidence_kinds"></label>`);
  }
  parts.push(`<label>${escapeHtml(LIVE.note)} <input data-field="note"></label>`);
  return `<div class="fields" data-item="${escapeHtml(r.id)}">${parts.join("")}</div>`;
}

function row(r: Row, live: string | null): string {
  const buttons = r.actions
    .map(
      (a) =>
        `<button type="button" data-item="${escapeHtml(r.id)}" data-action="${a}">${escapeHtml(ACTION[a])}</button>`,
    )
    .join("");
  const flags = [
    r.urgent ? `<span class="tag urgent">${escapeHtml(VIEW.urgent)}</span>` : "",
    r.overdue ? `<span class="tag overdue">${escapeHtml(VIEW.overdue)}</span>` : "",
  ].join("");
  return `<tr${r.urgent ? ' class="urgent"' : ""}>
<td>${flags}${escapeHtml(r.kind)}</td>
<td>${escapeHtml(r.subject)}<div class="muted">${escapeHtml(r.customer)}</div></td>
<td>${escapeHtml(r.why)}</td>
<td>${escapeHtml(r.due)}</td>
<td class="actions">${buttons}${live === null ? "" : fields(r, live)}</td>
</tr>`;
}

function fairness(report: Schemas["ParityReport"]): string {
  const groups = Object.entries(report.groups)
    .map(([group, g]) => {
      const flagged = report.flagged.includes(group);
      const note = g.credible ? (flagged ? VIEW.flagged : "") : VIEW.notCredible;
      return `<tr${flagged ? ' class="flagged"' : ""}><td>${escapeHtml(group)}</td><td>${g.approved} / ${g.decisions}</td><td>${percent(g.rate)}</td><td>${escapeHtml(note)}</td></tr>`;
    })
    .join("");
  const ratio = report.min_ratio == null ? "–" : percent(report.min_ratio);
  return `<section class="report">
<h3>${escapeHtml(report.kind)} · ${escapeHtml(report.attribute)} <span class="muted">min ratio ${ratio}</span></h3>
<table><tbody>${groups}</tbody></table>
</section>`;
}

function audit(stats: Schemas["AuditStats"][]): string {
  const lines = stats
    .map(
      (s) =>
        `<tr><td>${escapeHtml(s.kind)}</td><td>${s.overturned} / ${s.sampled}</td><td>${percent(s.overturn_rate)}</td></tr>`,
    )
    .join("");
  return `<table><tbody>${lines}</tbody></table>`;
}

export const STYLE = `
:root { --bg: #fafaf9; --fg: #1c1917; --muted: #78716c; --line: #e7e5e4; --accent: #b91c1c; --warn: #b45309; }
@media (prefers-color-scheme: dark) { :root { --bg: #1c1917; --fg: #fafaf9; --muted: #a8a29e; --line: #44403c; --accent: #f87171; --warn: #fbbf24; } }
body { margin: 0; padding: 24px 16px; background: var(--bg); color: var(--fg); font: 14px/1.45 system-ui, sans-serif; }
main { max-width: 1100px; margin: 0 auto; }
h1 { font-size: 20px; margin: 0 0 4px; } h2 { font-size: 16px; margin: 28px 0 8px; } h3 { font-size: 14px; margin: 16px 0 6px; }
table { width: 100%; border-collapse: collapse; } td { padding: 8px 6px; border-top: 1px solid var(--line); vertical-align: top; }
.muted { color: var(--muted); font-size: 12px; } .tag { font-size: 11px; font-weight: 600; margin-right: 6px; text-transform: uppercase; }
.urgent .tag.urgent, .tag.urgent { color: var(--accent); } .tag.overdue { color: var(--warn); } tr.flagged td { color: var(--accent); }
.actions { white-space: nowrap; } button { margin: 0 4px 4px 0; padding: 4px 10px; border: 1px solid var(--line); border-radius: 6px; background: transparent; color: inherit; cursor: pointer; }
.queue td:nth-child(4) { white-space: nowrap; }
.fields { display: grid; gap: 4px; margin-top: 6px; } .fields label { display: grid; font-size: 12px; color: var(--muted); } .fields input, .fields select { font: inherit; padding: 4px 6px; border: 1px solid var(--line); border-radius: 6px; background: transparent; color: var(--fg); }
header.live { display: flex; justify-content: space-between; align-items: center; gap: 8px; flex-wrap: wrap; } .flash { border-left: 4px solid var(--warn); padding: 6px 10px; }
@media (max-width: 720px) { .queue, .queue tbody, .queue tr, .queue td { display: block; } .queue tr { border-top: 1px solid var(--line); padding: 8px 0; } .queue td { border: 0; padding: 2px 0; } .actions { white-space: normal; padding-top: 6px; } }
`;

/**
 * The page body: the queue, the fairness audit and the overturn rates. `live` is the market's
 * currency when people decide here (the live console), none for a snapshot read offline.
 */
export function body(snapshot: Snapshot, live: string | null = null): string {
  const queue = rows(snapshot, live ?? "");
  const table =
    queue.length === 0
      ? `<p class="muted">${escapeHtml(VIEW.noItems)}</p>`
      : `<table class="queue"><tbody>${queue.map((r) => row(r, live)).join("")}</tbody></table>`;
  return `<h1>${escapeHtml(VIEW.title)}</h1>
<div class="muted">${escapeHtml(VIEW.asOf(snapshot.as_of))}</div>
<h2>${escapeHtml(VIEW.queue)}</h2>
${table}
<h2>${escapeHtml(VIEW.fairness)}</h2>
${snapshot.fairness.map(fairness).join("")}
<h2>${escapeHtml(VIEW.audit)}</h2>
${audit(snapshot.audit)}`;
}

/** The whole page as one document, for a snapshot read offline. */
export function render(snapshot: Snapshot): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(VIEW.title)}</title>
<style>${STYLE}</style>
</head>
<body>
<main>
${body(snapshot)}
</main>
</body>
</html>
`;
}

/** A pilot metric as people read it: a rate as a percentage, hours as hours. */
function metricValue(m: Schemas["PilotMetric"]): string {
  if (m.value === null || m.value === undefined) {
    return ADOPTION.notMeasured;
  }
  switch (m.unit) {
    case "bps":
      return percent(m.value);
    case "hours":
      return `${m.value} h`;
    default:
      return m.value.toLocaleString("en");
  }
}

function gate(g: Schemas["PilotGate"] | undefined): string {
  if (!g) {
    return "";
  }
  const bound =
    g.at_least != null
      ? ADOPTION.atLeast(g.at_least)
      : g.at_most != null
        ? ADOPTION.atMost(g.at_most)
        : "";
  return `${bound} · ${ADOPTION.gateKind[g.kind] ?? g.kind}`;
}

/** The job queue and the pilot's report (ADR-012), for people at the insurer. */
export function adoption(
  jobs: Schemas["JobsView"] | null,
  pilot: Schemas["PilotReport"] | null,
): string {
  const parts: string[] = [];
  if (jobs) {
    const counts = Object.entries(jobs.counts)
      .map(([status, n]) => `${escapeHtml(status)} ${n}`)
      .join(" · ");
    const failed =
      jobs.failed.length === 0
        ? `<p class="muted">${escapeHtml(ADOPTION.noFailed)}</p>`
        : `<table><tbody>${jobs.failed
            .map(
              (j) =>
                `<tr><td>${escapeHtml(j.key)}</td><td>${j.attempts}</td><td>${escapeHtml(j.last_error ?? "")}</td></tr>`,
            )
            .join("")}</tbody></table>`;
    parts.push(
      `<h2>${escapeHtml(ADOPTION.jobs)}</h2><p class="muted">${escapeHtml(ADOPTION.jobsNote)}</p><p>${counts}</p>${failed}`,
    );
  }
  if (pilot) {
    const lines = pilot.metrics
      .map(
        (m) =>
          `<tr><td>${escapeHtml(m.metric)}</td><td>${escapeHtml(metricValue(m))}</td><td class="muted">${escapeHtml(ADOPTION.over(m.sample))}</td><td>${escapeHtml(gate(pilot.gates.find((g) => g.metric === m.metric)))}</td></tr>`,
      )
      .join("");
    parts.push(
      `<h2>${escapeHtml(ADOPTION.pilot)} · ${escapeHtml(pilot.pilot_id)}</h2><p class="muted">${escapeHtml(ADOPTION.pilotNote(pilot.from, pilot.to, pilot.gates_fixed_on))}</p><table><tbody>${lines}</tbody></table>`,
    );
  }
  return parts.join("");
}
