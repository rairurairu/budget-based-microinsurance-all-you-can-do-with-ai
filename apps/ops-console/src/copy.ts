// Words the console shows insurer staff, in one place so an adopter can replace them. Ids come
// from the generated contract types; customers never see this console.
import type { Schemas } from "../../api-client/src/index.ts";

export const ACTION: Record<Schemas["PersonAction"], string> = {
  approve: "Approve",
  decline: "Decline",
  request_evidence: "Ask for evidence",
  verify: "Verify",
  not_verify: "Not verified",
  agree: "Agree",
  overturn: "Overturn",
  uphold: "Uphold",
};

export const CLAIM_REFERRAL: Record<Schemas["ClaimReferral"]["referral"], string> = {
  not_in_force: "No cover in force for the loss",
  waiting_period: "Loss in a waiting period",
  no_assessment: "No assessment from the assessor",
  assessor_referred: "Assessor recommends a person",
  low_confidence: "Assessor not confident enough",
  above_estimate: "Claimed above the assessor's estimate",
  above_stp_limit: "Above the straight-through limit",
  audit: "Audit",
  human_care: "Customer needs a person",
  appeal: "Review of a decline",
};

export const AUDIT_REASON: Record<Schemas["AuditReason"], string> = {
  random_sample: "random sample",
  risk_review: "risk review",
};

export const KIND: Record<Schemas["WorkKind"]["kind"], string> = {
  claim_referral: "Claim",
  task_referral: "Prevention task",
  audit: "Audit",
  appeal: "Review request",
};

/** A decline names one of these, each tied to the wording (R13). */
export const DECLINE_REASON: Record<Schemas["DeclineReason"], string> = {
  not_in_force: "No cover in force",
  waiting_period: "Waiting period",
  not_an_insured_loss: "Not an insured loss",
  policy_exclusion: "Policy exclusion",
  evidence_not_provided: "Evidence not provided",
  fraud: "Fraud",
};

export const LIVE = {
  as: "Acting as",
  amount: (currency: string) => `Loss assessed (${currency})`,
  kinds: "Evidence kinds, comma separated",
  note: "Note",
  reason: "Reason",
  done: (action: string, item: string) => `${action}: ${item}`,
  signedOut: "Open the console with your staff token.",
  refused: (detail: string) => `Refused: ${detail}`,
};

export const ADOPTION = {
  jobs: "Batch jobs",
  jobsNote:
    "Renewals, period openings, erasures and experience studies run from the job queue; a failed job waits here for a person.",
  noFailed: "No failed jobs.",
  pilot: "Pilot",
  pilotNote: (from: string, to: string, fixed: string) =>
    `Customers bound from ${from} to ${to}; gates fixed on ${fixed}. Evaluated with microinsurance_economics.pilot_evaluation.`,
  notMeasured: "not measured yet",
  over: (n: number) => `over ${n}`,
  atLeast: (v: number) => `at least ${v}`,
  atMost: (v: number) => `at most ${v}`,
  gateKind: { scale: "to scale", stop: "stops" } as Record<string, string>,
};

export const VIEW = {
  title: "Operations",
  queue: "Work queue",
  fairness: "Fairness audit",
  audit: "Audit overturn rate (random sample)",
  overdue: "overdue",
  dueIn: (hours: number) => `due in ${hours} h`,
  urgent: "Urgent",
  notCredible: "too few decisions to compare",
  flagged: "flagged",
  noItems: "Nothing waiting for a person.",
  asOf: (iso: string) => `As of ${iso}`,
};
