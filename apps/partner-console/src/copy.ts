// Words the console shows a partner's staff, in one place so an adopter can replace them.
export const VIEW = {
  title: "Partner console",
  results: "Results",
  resultsNote: (min: number) =>
    `Aggregates over your customers only. A figure from fewer than ${min} customers is withheld, never estimated.`,
  withheld: (min: number) => `withheld: fewer than ${min} customers`,
  over: (n: number) => `over ${n} customers`,
  offers: "Spare capacity offered as rewards",
  offersNote:
    "Your offers appear on customers' reward menus after cash, at the price taken from their reward balance; you are paid that price for each delivery on your monthly statement.",
  notSupplier:
    "You are not registered as a reward supplier; ask the insurer to add you to the supplier panel.",
  publish: "Publish",
  published: "Published.",
  embed: "Embed in your app",
  embedNote:
    "Your server exchanges your own reference for a customer for a short-lived customer token, then your page shows the embed with it.",
  demoAs: "Demo: act as",
  signedOut: "Open this console with your partner token.",
  error: (detail: string) => `Refused: ${detail}`,
};

export const METRIC: Record<string, string> = {
  subscribers: "Customers who came through you",
  in_force: "With cover in force today",
  prevention_task_completion: "Prevention tasks completed",
  payouts: "Automatic payouts",
  payouts_amount: "Paid out automatically",
  redemptions: "Your rewards taken",
  quoted: "Shown a quote",
  conversion: "Bound from a quote",
  renewal: "Renewed after their first month",
  reward_redemption: "Rewards taken from the reward balance",
};

export const FIELD = {
  description: "What the customer gets",
  value: "Value to the customer",
  price: "Price from the reward balance",
  stock: "Stock",
};
