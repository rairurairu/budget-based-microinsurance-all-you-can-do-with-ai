// The work queue as rows. The engine decides the order and the actions each item allows
// (`guardrails::queue`); the console shows them and adds nothing: a decline button exists only
// where the snapshot offers one, which is only on a claim referral (R13).
import { type Schemas, money as format } from "../../api-client/src/index.ts";
import { AUDIT_REASON, CLAIM_REFERRAL, KIND, VIEW } from "./copy.ts";

export type Snapshot = Schemas["OpsSnapshot"];
export type WorkItem = Schemas["WorkItem"];

export interface Row {
  id: string;
  kind: string;
  customer: string;
  subject: string;
  why: string;
  due: string;
  overdue: boolean;
  urgent: boolean;
  actions: Schemas["PersonAction"][];
}

const HOUR_MS = 3_600_000;

/**
 * Money in minor units, as the engine sends it, in the market's currency (its ISO code from
 * `/registry`); without one (a snapshot read offline), the amount alone.
 */
export function money(minor: number, currency = ""): string {
  if (currency) {
    return format(minor, currency);
  }
  return (minor / 100).toLocaleString("en", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

/** Basis points as a percentage with one decimal. */
export function percent(bps: number): string {
  return `${(bps / 100).toFixed(1)}%`;
}

function referralText(r: Schemas["ClaimReferral"]): string {
  const base = CLAIM_REFERRAL[r.referral];
  if (r.referral === "audit") {
    return `${base}: ${AUDIT_REASON[r.reason]}`;
  }
  if (r.referral === "appeal") {
    return `${base} by ${r.declined_by}`;
  }
  return base;
}

function taskReferralText(r: Schemas["Referral"]): string {
  if (typeof r === "string") {
    return r.replaceAll("_", " ");
  }
  return `audit: ${AUDIT_REASON[r.audit]}`;
}

function describe(work: Schemas["WorkKind"], currency: string): { subject: string; why: string } {
  switch (work.kind) {
    case "claim_referral":
      return {
        subject: `${work.claim_id} · ${work.cover_id} · ${money(work.amount_claimed, currency)}`,
        why: referralText(work.referral),
      };
    case "task_referral":
      return { subject: work.task_id, why: taskReferralText(work.referral) };
    case "audit":
      return {
        subject: `${work.decision_kind} ${work.decision_id}`,
        why: AUDIT_REASON[work.reason],
      };
    case "appeal":
      return { subject: `${work.subject_type} ${work.subject_id}`, why: KIND.appeal };
  }
}

/** Hours left until the item is due, rounded down; negative once overdue. */
export function hoursLeft(item: WorkItem, asOf: string): number {
  return Math.floor((Date.parse(item.due_at) - Date.parse(asOf)) / HOUR_MS);
}

/** One row per item, in the snapshot's order; amounts in `currency` when known. */
export function rows(snapshot: Snapshot, currency = ""): Row[] {
  return snapshot.items.map((item) => {
    const left = hoursLeft(item, snapshot.as_of);
    const overdue = Date.parse(item.due_at) < Date.parse(snapshot.as_of);
    return {
      id: item.item_id,
      kind: KIND[item.work.kind],
      customer: item.customer_id,
      ...describe(item.work, currency),
      due: overdue ? VIEW.overdue : VIEW.dueIn(left),
      overdue,
      urgent: item.urgent,
      actions: [...item.actions],
    };
  });
}
